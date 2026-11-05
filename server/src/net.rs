//! TLS sessions and the tick loop. One task owns the `World`; sessions only pass
//! it messages, so game state never needs a lock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use protocol::{
    client_message, login, server_message, ClientMessage, CommandResult, LoginResult, ServerMessage,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, timeout, MissedTickBehavior};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

use crate::sim::World;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
/// A client only sends short command lines, so its frames are small.
const MAX_CLIENT_FRAME_BYTES: usize = 4 * 1024;
const ACTOR_QUEUE: usize = 4096;
/// A client this many messages behind is disconnected, not waited for.
const SESSION_QUEUE: usize = 256;
const MAX_ACCOUNT_CHARS: usize = 32;

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    /// Accept `dev_account` logins with no proof of identity. Local use only.
    pub dev_login: bool,
}

enum ToWorld {
    Connect {
        account: String,
        session: u64,
        out: mpsc::Sender<ServerMessage>,
        kick: oneshot::Sender<()>,
    },
    Command {
        session: u64,
        id: u64,
        line: String,
    },
    Disconnect {
        session: u64,
    },
}

struct Session {
    account: String,
    out: mpsc::Sender<ServerMessage>,
    /// Dropping this ends the session's reader.
    _kick: oneshot::Sender<()>,
}

pub fn tls_acceptor(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<TlsAcceptor> {
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("building the TLS config")?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn serve(
    listener: TcpListener,
    tls: TlsAcceptor,
    world: World,
    options: ServeOptions,
) -> Result<()> {
    let (to_world, from_sessions) = mpsc::channel(ACTOR_QUEUE);
    tokio::spawn(run_world(world, from_sessions));
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(err) => {
                tracing::warn!(%err, "accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let tls = tls.clone();
        let to_world = to_world.clone();
        let options = options.clone();
        tokio::spawn(async move {
            if let Err(err) = run_session(tcp, tls, to_world, options).await {
                tracing::debug!(%peer, %err, "session ended");
            }
        });
    }
}

async fn run_world(mut world: World, mut inbox: mpsc::Receiver<ToWorld>) {
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    let mut ticker = interval(Duration::from_millis(u64::from(world.tick_ms())));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                world.tick();
                let ids: Vec<u64> = sessions.keys().copied().collect();
                for id in ids {
                    send_snapshot(&mut world, &mut sessions, id);
                }
            }
            message = inbox.recv() => {
                let Some(message) = message else { return };
                handle(&mut world, &mut sessions, message);
            }
        }
    }
}

fn handle(world: &mut World, sessions: &mut HashMap<u64, Session>, message: ToWorld) {
    match message {
        ToWorld::Connect {
            account,
            session,
            out,
            kick,
        } => {
            // A new login replaces any older session on the same account.
            let stale: Vec<u64> = sessions
                .iter()
                .filter(|(_, s)| s.account == account)
                .map(|(id, _)| *id)
                .collect();
            for id in stale {
                sessions.remove(&id);
            }
            world.login(&account);
            let reply = login_reply(true, "Logged in.");
            sessions.insert(
                session,
                Session {
                    account,
                    out,
                    _kick: kick,
                },
            );
            deliver(world, sessions, session, reply);
            send_snapshot(world, sessions, session);
        }
        ToWorld::Command { session, id, line } => {
            let Some(account) = sessions.get(&session).map(|s| s.account.clone()) else {
                return;
            };
            let outcome = world.execute(&account, &line);
            let reply = ServerMessage {
                body: Some(server_message::Body::CommandResult(CommandResult {
                    id,
                    ok: outcome.is_ok(),
                    refusal: outcome.refusal as i32,
                    message: outcome.message,
                })),
            };
            deliver(world, sessions, session, reply);
            send_snapshot(world, sessions, session);
        }
        ToWorld::Disconnect { session } => {
            if let Some(ended) = sessions.remove(&session) {
                world.logout(&ended.account);
            }
        }
    }
}

fn send_snapshot(world: &mut World, sessions: &mut HashMap<u64, Session>, id: u64) {
    let Some(account) = sessions.get(&id).map(|s| s.account.clone()) else {
        return;
    };
    if let Some(snapshot) = world.snapshot(&account) {
        let message = ServerMessage {
            body: Some(server_message::Body::Snapshot(snapshot)),
        };
        deliver(world, sessions, id, message);
    }
}

/// Never waits: the tick loop must not stall on one slow client.
fn deliver(
    world: &mut World,
    sessions: &mut HashMap<u64, Session>,
    id: u64,
    message: ServerMessage,
) {
    let Some(session) = sessions.get(&id) else {
        return;
    };
    if session.out.try_send(message).is_err() {
        let dropped = sessions.remove(&id).expect("looked up above");
        tracing::info!(account = %dropped.account, "disconnecting a client that fell behind");
        world.logout(&dropped.account);
    }
}

fn login_reply(ok: bool, message: &str) -> ServerMessage {
    ServerMessage {
        body: Some(server_message::Body::LoginResult(LoginResult {
            ok,
            message: message.to_string(),
        })),
    }
}

fn authenticate(frame: &[u8], options: &ServeOptions) -> Result<String, String> {
    let message = ClientMessage::decode(frame).map_err(|_| "Malformed login.".to_string())?;
    let Some(client_message::Body::Login(login)) = message.body else {
        return Err("The first message must be a login.".to_string());
    };
    match login.credential {
        Some(login::Credential::DevAccount(account)) => {
            if !options.dev_login {
                return Err("Development login is disabled on this server.".to_string());
            }
            let valid = !account.is_empty()
                && account.len() <= MAX_ACCOUNT_CHARS
                && account
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if valid {
                Ok(account)
            } else {
                Err("Invalid account name.".to_string())
            }
        }
        Some(login::Credential::SteamTicket(_)) => {
            Err("Steam login is not available yet.".to_string())
        }
        None => Err("No login credential supplied.".to_string()),
    }
}

async fn run_session(
    tcp: TcpStream,
    tls: TlsAcceptor,
    to_world: mpsc::Sender<ToWorld>,
    options: ServeOptions,
) -> Result<()> {
    tcp.set_nodelay(true)?;
    let stream = timeout(HANDSHAKE_TIMEOUT, tls.accept(tcp))
        .await
        .context("TLS timeout")??;
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = FramedRead::new(
        read_half,
        LengthDelimitedCodec::builder()
            .max_frame_length(MAX_CLIENT_FRAME_BYTES)
            .new_codec(),
    );
    let mut writer = FramedWrite::new(
        write_half,
        LengthDelimitedCodec::builder()
            .max_frame_length(protocol::MAX_FRAME_BYTES)
            .new_codec(),
    );

    let frame = timeout(LOGIN_TIMEOUT, reader.next())
        .await
        .context("login timeout")?;
    let Some(frame) = frame else {
        bail!("closed before login")
    };
    let account = match authenticate(&frame?, &options) {
        Ok(account) => account,
        Err(reason) => {
            let reply = login_reply(false, &reason).encode_to_vec();
            writer.send(Bytes::from(reply)).await?;
            bail!("login refused: {reason}");
        }
    };

    let session = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
    let (out, mut outbox) = mpsc::channel::<ServerMessage>(SESSION_QUEUE);
    let (kick, mut kicked) = oneshot::channel();
    to_world
        .send(ToWorld::Connect {
            account,
            session,
            out,
            kick,
        })
        .await?;

    let writer_task = tokio::spawn(async move {
        while let Some(message) = outbox.recv().await {
            if writer
                .send(Bytes::from(message.encode_to_vec()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            _ = &mut kicked => break,
            frame = reader.next() => {
                let Some(Ok(frame)) = frame else { break };
                let Ok(message) = ClientMessage::decode(&frame[..]) else { break };
                match message.body {
                    Some(client_message::Body::Command(command)) => {
                        let message = ToWorld::Command { session, id: command.id, line: command.line };
                        if to_world.send(message).await.is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
    }
    let _ = to_world.send(ToWorld::Disconnect { session }).await;
    writer_task.abort();
    Ok(())
}
