use anyhow::{bail, Context, Result};
use bot::Client;

const USAGE: &str = "\
Usage: bot --account NAME [options]
  --addr ADDR    Server address (default 127.0.0.1:7777)
  --ca FILE      CA certificate to trust (default dev-certs/ca.pem)
  --peer NAME    After the slice, wait until this player is on radar
  --quiet        Print only the final report";

/// Ticks to wait for a peer. Two bots started together finish close to each other.
const PEER_WAIT_TICKS: u64 = 600;

#[tokio::main]
async fn main() -> Result<()> {
    let mut addr = "127.0.0.1:7777".to_string();
    let mut ca = "dev-certs/ca.pem".to_string();
    let mut account = None;
    let mut peer = None;
    let mut verbose = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--addr" => addr = value()?,
            "--ca" => ca = value()?,
            "--account" => account = Some(value()?),
            "--peer" => peer = Some(value()?),
            "--quiet" => verbose = false,
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown option {other}\n{USAGE}"),
        }
    }
    let account = account.with_context(|| format!("--account is required\n{USAGE}"))?;
    let ca_pem = std::fs::read(&ca).with_context(|| format!("reading {ca}"))?;

    let mut client = Client::connect(&addr, &ca_pem, &account, verbose).await?;
    let report = client.play_slice().await?;
    if let Some(leak) = &client.pocket_leak {
        bail!("{leak} appeared on radar inside the private pocket");
    }
    println!("{account}: slice complete: {report:?}");
    if let Some(peer) = peer {
        client.wait_for_peer(&peer, PEER_WAIT_TICKS).await?;
        println!("{account}: {peer} is on radar in {}", client.snap.system);
    }
    Ok(())
}
