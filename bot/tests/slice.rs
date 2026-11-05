//! The slice's "done when", over real TLS and the real protocol, with the real
//! numbers from `config/`. Only the tick length is shortened.

use std::time::Duration;

use bot::{Client, SliceReport};
use protocol::{ContactKind, Refusal, ShipState};
use server::net::{self, ServeOptions};
use server::sim::World;
use server::{config, dev_tls};
use tokio::net::TcpListener;
use tokio::time::timeout;

const TEST_TICK_MS: u32 = 5;

struct TestServer {
    addr: String,
    ca_pem: Vec<u8>,
}

async fn start(dev_login: bool) -> TestServer {
    let (mut game, content) = config::parse(
        include_str!("../../config/game.toml"),
        include_str!("../../config/world.toml"),
    )
    .unwrap();
    game.tick_ms = TEST_TICK_MS;
    let tls = dev_tls::generate().unwrap();
    let ca_pem = tls.ca_pem.clone().into_bytes();
    let acceptor = net::tls_acceptor(tls.certs, tls.key).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let world = World::new(game, content);
    tokio::spawn(net::serve(
        listener,
        acceptor,
        world,
        ServeOptions { dev_login },
    ));
    TestServer { addr, ca_pem }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_fresh_accounts_play_the_slice_and_see_each_other_in_kesh() {
    let server = start(true).await;
    let play = |name: &'static str, peer: &'static str| {
        let addr = server.addr.clone();
        let ca = server.ca_pem.clone();
        tokio::spawn(async move {
            let mut client = Client::connect(&addr, &ca, name, false).await?;
            let report = client.play_slice().await?;
            assert_eq!(client.pocket_leak, None, "the pocket must be private");
            client.wait_for_peer(peer, 20_000).await?;
            assert_eq!(client.snap.system, "Kesh");
            anyhow::Ok(report)
        })
    };
    let a = play("alice", "bob");
    let b = play("bob", "alice");
    let (a, b) = timeout(Duration::from_secs(120), async { (a.await, b.await) })
        .await
        .unwrap();
    for report in [a.unwrap().unwrap(), b.unwrap().unwrap()] {
        let SliceReport {
            first_loop_ticks,
            final_credits,
            ..
        } = report;
        assert_eq!(final_credits, 13_000);
        // The design's target is a 12 minute loop, undock to sale.
        assert!(
            (600..=840).contains(&first_loop_ticks),
            "loop took {first_loop_ticks} ticks"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn dev_login_is_refused_unless_enabled() {
    let server = start(false).await;
    let error = Client::connect(&server.addr, &server.ca_pem, "alice", false)
        .await
        .err()
        .expect("login should be refused");
    assert!(error.to_string().contains("disabled"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_account_names_are_refused() {
    let server = start(true).await;
    for name in ["", "has space", "../etc", &"x".repeat(33)] {
        assert!(
            Client::connect(&server.addr, &server.ca_pem, name, false)
                .await
                .is_err(),
            "{name:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_untrusted_certificate_is_rejected() {
    let server = start(true).await;
    let other_ca = dev_tls::generate().unwrap().ca_pem.into_bytes();
    assert!(Client::connect(&server.addr, &other_ca, "alice", false)
        .await
        .is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnecting_keeps_the_character_and_replaces_the_old_session() {
    let server = start(true).await;
    let mut first = Client::connect(&server.addr, &server.ca_pem, "carol", false)
        .await
        .unwrap();
    first.must("buy ship sparrow").await.unwrap();

    let mut second = Client::connect(&server.addr, &server.ca_pem, "carol", false)
        .await
        .unwrap();
    assert_eq!(second.state(), ShipState::Docked);
    assert_eq!(second.snap.credits, 4_000);
    second
        .must_refuse("buy ship sparrow", Refusal::AlreadyOwned)
        .await
        .unwrap();

    let ended = timeout(Duration::from_secs(10), async {
        loop {
            if first.command("stop").await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(ended.is_ok(), "the older session should be closed");
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_outside_the_players_grid_reaches_the_client() {
    let server = start(true).await;
    let mut client = Client::connect(&server.addr, &server.ca_pem, "dave", false)
        .await
        .unwrap();
    assert!(
        client.snap.contacts.is_empty(),
        "nothing is visible while docked"
    );
    client.must("buy ship sparrow").await.unwrap();
    client.must("undock").await.unwrap();
    client
        .wait("undock", 60, |c| c.state() == ShipState::Idle)
        .await
        .unwrap();
    let kinds: Vec<_> = client.snap.contacts.iter().map(|c| c.kind()).collect();
    assert_eq!(
        kinds,
        [ContactKind::Station],
        "the station grid shows only the station"
    );
}
