//! A headless client that speaks the real protocol: used for slice tests, load
//! tests later, and as the reference for the automation API.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use prost::Message;
use protocol::{
    client_message, login, server_message, ClientMessage, Command, CommandResult, Contact,
    ContactKind, Login, Refusal, ServerMessage, ShipState, Snapshot, TutorialStep,
};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

/// No single server message should take this long, whatever the tick rate.
const FRAME_TIMEOUT: Duration = Duration::from_secs(30);
const POCKET_NAME: &str = "Training Pocket";

type Stream = TlsStream<TcpStream>;

pub struct Client {
    reader: FramedRead<ReadHalf<Stream>, LengthDelimitedCodec>,
    writer: FramedWrite<WriteHalf<Stream>, LengthDelimitedCodec>,
    next_id: u64,
    verbose: bool,
    pub snap: Snapshot,
    /// Every event seen so far.
    pub events: Vec<String>,
    /// Set if another player ever appears on radar inside the private pocket.
    pub pocket_leak: Option<String>,
}

impl Client {
    pub async fn connect(addr: &str, ca_pem: &[u8], account: &str, verbose: bool) -> Result<Self> {
        let mut roots = RootCertStore::empty();
        for cert in CertificateDer::pem_slice_iter(ca_pem) {
            roots.add(cert?)?;
        }
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(addr)
            .await
            .with_context(|| format!("connecting to {addr}"))?;
        tcp.set_nodelay(true)?;
        let name = ServerName::try_from("localhost")?;
        let stream = TlsConnector::from(Arc::new(config))
            .connect(name, tcp)
            .await?;
        let (read_half, write_half) = tokio::io::split(stream);
        let codec = || {
            LengthDelimitedCodec::builder()
                .max_frame_length(protocol::MAX_FRAME_BYTES)
                .new_codec()
        };
        let mut client = Self {
            reader: FramedRead::new(read_half, codec()),
            writer: FramedWrite::new(write_half, codec()),
            next_id: 1,
            verbose,
            snap: Snapshot::default(),
            events: Vec::new(),
            pocket_leak: None,
        };
        client.login(account).await?;
        Ok(client)
    }

    async fn login(&mut self, account: &str) -> Result<()> {
        self.send(client_message::Body::Login(Login {
            credential: Some(login::Credential::DevAccount(account.to_string())),
        }))
        .await?;
        match self.read().await?.body {
            Some(server_message::Body::LoginResult(result)) if result.ok => {}
            Some(server_message::Body::LoginResult(result)) => {
                bail!("login refused: {}", result.message)
            }
            other => bail!("expected a login result, got {other:?}"),
        }
        self.next_snapshot().await
    }

    async fn send(&mut self, body: client_message::Body) -> Result<()> {
        let message = ClientMessage { body: Some(body) };
        self.writer
            .send(Bytes::from(message.encode_to_vec()))
            .await?;
        Ok(())
    }

    async fn read(&mut self) -> Result<ServerMessage> {
        let frame = timeout(FRAME_TIMEOUT, self.reader.next())
            .await
            .context("the server went silent")?
            .context("the server closed the connection")??;
        let message = ServerMessage::decode(&frame[..])?;
        if let Some(server_message::Body::Snapshot(snapshot)) = &message.body {
            self.absorb(snapshot.clone());
        }
        Ok(message)
    }

    fn absorb(&mut self, snapshot: Snapshot) {
        if snapshot.system == POCKET_NAME {
            if let Some(other) = snapshot
                .contacts
                .iter()
                .find(|c| c.kind() == ContactKind::PlayerShip)
            {
                self.pocket_leak = Some(other.name.clone());
            }
        }
        for event in &snapshot.events {
            if self.verbose {
                println!("[{} t{}] {event}", snapshot.character, snapshot.tick);
            }
            self.events.push(event.clone());
        }
        self.snap = snapshot;
    }

    async fn next_snapshot(&mut self) -> Result<()> {
        loop {
            if let Some(server_message::Body::Snapshot(_)) = self.read().await?.body {
                return Ok(());
            }
        }
    }

    /// Sends one verb line and returns the server's answer.
    pub async fn command(&mut self, line: &str) -> Result<CommandResult> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(client_message::Body::Command(Command {
            id,
            line: line.to_string(),
        }))
        .await?;
        loop {
            if let Some(server_message::Body::CommandResult(result)) = self.read().await?.body {
                if result.id == id {
                    if self.verbose {
                        println!(
                            "[{} t{}] > {line}: {}",
                            self.snap.character, self.snap.tick, result.message
                        );
                    }
                    // The commander's fresh snapshot directly follows its result.
                    self.next_snapshot().await?;
                    return Ok(result);
                }
            }
        }
    }

    /// Sends a command that must succeed.
    pub async fn must(&mut self, line: &str) -> Result<()> {
        let result = self.command(line).await?;
        ensure!(result.ok, "`{line}` was refused: {}", result.message);
        Ok(())
    }

    /// Sends a command that must be refused for this reason.
    pub async fn must_refuse(&mut self, line: &str, want: Refusal) -> Result<()> {
        let result = self.command(line).await?;
        ensure!(
            result.refusal() == want,
            "`{line}` should be {want:?}, got {:?}: {}",
            result.refusal(),
            result.message
        );
        Ok(())
    }

    pub fn state(&self) -> ShipState {
        self.snap
            .ship
            .as_ref()
            .map_or(ShipState::None, |s| s.state())
    }

    pub fn hold(&self) -> (u32, u32) {
        self.snap
            .ship
            .as_ref()
            .map_or((0, 0), |s| (s.hold_used_m3, s.hold_capacity_m3))
    }

    /// Reads until `done` holds, failing after `limit_ticks` of game time.
    pub async fn wait(
        &mut self,
        what: &str,
        limit_ticks: u64,
        done: impl Fn(&Self) -> bool,
    ) -> Result<()> {
        let start = self.snap.tick;
        while !done(self) {
            ensure!(
                self.snap.tick - start <= limit_ticks,
                "timed out waiting for {what} after {limit_ticks} ticks"
            );
            self.next_snapshot().await?;
        }
        Ok(())
    }

    async fn wait_state(&mut self, want: ShipState, limit_ticks: u64) -> Result<()> {
        self.wait(&format!("{want:?}"), limit_ticks, |c| c.state() == want)
            .await
    }

    fn contact_of(&self, kind: ContactKind) -> Option<Contact> {
        self.snap
            .contacts
            .iter()
            .filter(|c| c.kind() == kind && (kind != ContactKind::Asteroid || c.volume_left_m3 > 0))
            .min_by(|a, b| a.distance_km.total_cmp(&b.distance_km))
            .cloned()
    }

    async fn warp(&mut self, site: &str) -> Result<()> {
        self.must(&format!("warp {site}")).await?;
        self.wait_state(ShipState::Warping, 30).await?;
        self.wait_state(ShipState::Idle, 120).await
    }

    async fn undock(&mut self) -> Result<()> {
        self.must("undock").await?;
        self.wait_state(ShipState::Idle, 60).await
    }

    async fn dock(&mut self) -> Result<()> {
        self.must("dock").await?;
        self.wait_state(ShipState::Docked, 60).await
    }

    /// From anywhere on a belt, mines until the hold is full. Switches to a rich
    /// vein as soon as one appears. Returns whether a vein cycle paid out.
    async fn mine_until_full(&mut self, target_hold: u32) -> Result<bool> {
        let range = self.snap.ship.as_ref().map_or(0.0, |s| s.laser_range_km);
        let mut used_vein = false;
        while self.hold().0 < target_hold {
            let vein = self.contact_of(ContactKind::RichVein);
            let rock = vein
                .clone()
                .or_else(|| self.contact_of(ContactKind::Asteroid))
                .context("no rock left on this grid")?;
            if rock.distance_km > range {
                self.must(&format!("approach {}", rock.id)).await?;
                self.wait_state(ShipState::Idle, 120).await?;
                continue;
            }
            let is_vein = vein.is_some();
            let target_id = rock.id.clone();
            self.must(&format!("mine {target_id}")).await?;
            let before = self.hold().0;
            self.wait("mining to stop or a vein to appear", 900, |c| {
                let on_vein = c.contact_of(ContactKind::RichVein).is_some() && !is_vein;
                c.state() != ShipState::Mining
                    || on_vein
                    || c.hold().0 >= target_hold
                    || (is_vein && c.hold().0 > before)
            })
            .await?;
            if is_vein && self.hold().0 > before {
                used_vein = true;
                // One cycle is all the vein's lifetime allows, so go back to rock.
                if self.state() == ShipState::Mining {
                    self.must("stop").await?;
                }
            }
            if self.hold().0 >= target_hold && self.state() == ShipState::Mining {
                self.must("stop").await?;
            }
        }
        Ok(used_vein)
    }

    /// One whole run on the current system: belt, full hold, station, sale.
    async fn ore_run(&mut self) -> Result<bool> {
        self.warp("belt-1").await?;
        let (_, capacity) = self.hold();
        let used_vein = self.mine_until_full(capacity).await?;
        self.warp("station").await?;
        self.dock().await?;
        self.must("sell ferrite all").await?;
        Ok(used_vein)
    }

    /// Plays the vertical slice: fresh login, tutorial, graduation, first sale in
    /// Kesh, the first upgrade, then back into space on the Kesh Station grid.
    pub async fn play_slice(&mut self) -> Result<SliceReport> {
        ensure!(
            self.snap.system == POCKET_NAME,
            "a fresh account should start in the pocket"
        );
        ensure!(
            self.snap.credits == 10_000,
            "unexpected starting grant: {}",
            self.snap.credits
        );
        ensure!(
            self.state() == ShipState::None,
            "a fresh account should own no ship"
        );
        ensure!(
            self.snap.tutorial_step() == TutorialStep::BuyShip,
            "tutorial should start at the shipyard"
        );

        self.must_refuse("undock", Refusal::NoShip).await?;
        self.must_refuse("jump exit", Refusal::NoShip).await?;
        self.must("buy ship sparrow").await?;
        ensure!(
            self.snap.credits == 4_000,
            "the Sparrow should cost 6,000 cr"
        );
        self.must_refuse("warp belt-1", Refusal::WrongState).await?;
        self.undock().await?;
        let first_loop_start = self.snap.tick;

        // Refusals along the real path, not only in unit tests.
        self.must("warp belt-1").await?;
        self.must_refuse("dock", Refusal::WrongState).await?;
        self.wait_state(ShipState::Warping, 30).await?;
        ensure!(self.snap.contacts.is_empty(), "radar must be empty in warp");
        self.wait_state(ShipState::Idle, 120).await?;
        let nearest = self
            .contact_of(ContactKind::Asteroid)
            .context("belt has no rocks")?;
        self.must_refuse(&format!("mine {}", nearest.id), Refusal::OutOfRange)
            .await?;
        self.must_refuse("dock", Refusal::OutOfRange).await?;

        let (_, capacity) = self.hold();
        let vein_paid = self.mine_until_full(capacity).await?;
        ensure!(
            vein_paid,
            "the rich vein should have paid out in the tutorial"
        );
        ensure!(
            self.events.iter().any(|e| e.contains("+75 m³")),
            "the vein cycle should yield three times the laser"
        );

        self.warp("station").await?;
        self.dock().await?;
        self.must("sell ferrite all").await?;
        let first_loop_ticks = self.snap.tick - first_loop_start;
        let after_tutorial_credits = self.snap.credits;
        ensure!(
            after_tutorial_credits == 8_500,
            "4,000 cr + 4,500 cr, got {after_tutorial_credits}"
        );
        ensure!(
            self.snap.tutorial_step() == TutorialStep::Exit,
            "the sale should open the exit gate"
        );

        self.undock().await?;
        self.warp("exit-gate").await?;
        self.must("jump exit").await?;
        ensure!(
            self.snap.system == "Kesh",
            "the jump should arrive in Kesh, got {}",
            self.snap.system
        );
        ensure!(
            self.state() == ShipState::Idle,
            "the jump should arrive in space"
        );
        ensure!(
            self.snap.tutorial_step() == TutorialStep::None,
            "graduation should end the tutorial"
        );
        self.must_refuse("dock", Refusal::OutOfRange).await?;
        self.must_refuse("jump exit", Refusal::NotFound).await?;

        self.ore_run().await?;
        let first_kesh_sale_credits = self.snap.credits;
        ensure!(
            first_kesh_sale_credits == 13_000,
            "8,500 cr + 4,500 cr, got {first_kesh_sale_credits}"
        );

        // Mining Laser II is a long-term goal, gated by skills nobody has yet.
        self.must_refuse("buy module mining-laser-2", Refusal::MissingSkill)
            .await?;
        self.undock().await?;

        Ok(SliceReport {
            first_loop_ticks,
            after_tutorial_credits,
            first_kesh_sale_credits,
            final_credits: self.snap.credits,
        })
    }

    /// Waits until another player shows on radar, with their ship on this grid.
    pub async fn wait_for_peer(&mut self, name: &str, limit_ticks: u64) -> Result<()> {
        self.wait(&format!("{name} on radar"), limit_ticks, |c| {
            c.snap
                .contacts
                .iter()
                .any(|x| x.kind() == ContactKind::PlayerShip && x.name == name)
        })
        .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceReport {
    pub first_loop_ticks: u64,
    pub after_tutorial_credits: u64,
    pub first_kesh_sale_credits: u64,
    pub final_credits: u64,
}
