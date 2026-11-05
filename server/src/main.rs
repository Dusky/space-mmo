use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use server::net::{self, ServeOptions};
use server::sim::World;
use server::{config, dev_tls};
use tokio::net::TcpListener;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

const USAGE: &str = "\
Usage: server [options]
  --bind ADDR       Address to listen on (default 127.0.0.1:7777)
  --config DIR      Config directory (default config)
  --tick-ms N       Override tick length, for sped-up local testing
  --cert FILE       TLS certificate chain, PEM
  --key FILE        TLS private key, PEM
  --dev             Local development: accept dev logins without proof of
                    identity, and use a throwaway CA written to dev-certs/ca.pem";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut bind = "127.0.0.1:7777".to_string();
    let mut config_dir = PathBuf::from("config");
    let mut tick_ms = None;
    let mut cert = None;
    let mut key = None;
    let mut dev = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--bind" => bind = value()?,
            "--config" => config_dir = value()?.into(),
            "--tick-ms" => tick_ms = Some(value()?.parse::<u32>().context("--tick-ms")?),
            "--cert" => cert = Some(PathBuf::from(value()?)),
            "--key" => key = Some(PathBuf::from(value()?)),
            "--dev" => dev = true,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown option {other}\n{USAGE}"),
        }
    }

    let (mut game, content) = config::load(&config_dir)?;
    if let Some(tick_ms) = tick_ms {
        game.tick_ms = tick_ms;
    }

    let tls = match (cert, key, dev) {
        (Some(cert), Some(key), _) => {
            let certs = CertificateDer::pem_file_iter(&cert)
                .with_context(|| format!("reading {}", cert.display()))?
                .collect::<Result<Vec<_>, _>>()?;
            let key = PrivateKeyDer::from_pem_file(&key)
                .with_context(|| format!("reading {}", key.display()))?;
            net::tls_acceptor(certs, key)?
        }
        (None, None, true) => {
            let dev_tls = dev_tls::generate()?;
            std::fs::create_dir_all("dev-certs")?;
            std::fs::write("dev-certs/ca.pem", &dev_tls.ca_pem)?;
            net::tls_acceptor(dev_tls.certs, dev_tls.key)?
        }
        _ => bail!("pass both --cert and --key, or --dev for local development\n{USAGE}"),
    };

    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, tick_ms = game.tick_ms, dev, "server listening");
    if dev {
        tracing::warn!("development login is on: anyone who can connect can be any account");
    }

    let world = World::new(game, content);
    tokio::select! {
        result = net::serve(listener, tls, world, ServeOptions { dev_login: dev }) => result,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutting down");
            Ok(())
        }
    }
}
