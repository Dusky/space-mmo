//! Needs Postgres. Each test makes its own throwaway database from
//! DATABASE_URL (or ../.env), so real data is never touched. Skipped when no
//! database is configured, unless REQUIRE_DATABASE_TESTS is set, as in CI.

use protocol::ShipState;
use server::config;
use server::sim::World;
use server::store::Store;
use sqlx::Connection;

struct TestDb {
    admin_url: String,
    name: String,
    url: String,
}

impl TestDb {
    async fn create() -> Option<Self> {
        let admin_url = std::env::var("DATABASE_URL").ok().or_else(|| {
            let env = std::fs::read_to_string("../.env").ok()?;
            env.lines()
                .find_map(|l| l.strip_prefix("DATABASE_URL="))
                .map(str::to_string)
        });
        let Some(admin_url) = admin_url else {
            assert!(
                std::env::var_os("REQUIRE_DATABASE_TESTS").is_none(),
                "REQUIRE_DATABASE_TESTS is set but DATABASE_URL is not"
            );
            eprintln!("skipping: no DATABASE_URL");
            return None;
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let name = format!("space_mmo_test_{}_{nanos}", std::process::id());
        let mut admin = sqlx::PgConnection::connect(&admin_url).await.unwrap();
        sqlx::query(&format!("create database {name}"))
            .execute(&mut admin)
            .await
            .unwrap();
        let (base, _) = admin_url.rsplit_once('/').unwrap();
        let url = format!("{base}/{name}");
        Some(Self {
            admin_url,
            name,
            url,
        })
    }

    async fn drop(self) {
        let mut admin = sqlx::PgConnection::connect(&self.admin_url).await.unwrap();
        sqlx::query(&format!("drop database {} with (force)", self.name))
            .execute(&mut admin)
            .await
            .unwrap();
    }
}

fn world() -> World {
    let (game, content) = config::parse(
        include_str!("../../config/game.toml"),
        include_str!("../../config/world.toml"),
    )
    .unwrap();
    World::new(game, content)
}

fn restore(saved: server::sim::Saved) -> World {
    let (game, content) = config::parse(
        include_str!("../../config/game.toml"),
        include_str!("../../config/world.toml"),
    )
    .unwrap();
    World::restore(game, content, saved).unwrap()
}

#[tokio::test]
async fn a_save_loads_back_exactly_and_deleted_pockets_stay_deleted() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let store = Store::connect(&db.url).await.unwrap();
    assert_eq!(store.load().await.unwrap(), None, "a new database is empty");

    let mut w = world();
    for name in ["alice", "bob"] {
        w.login(name);
        assert!(w.execute(name, "buy ship sparrow").is_ok());
        assert!(w.execute(name, "undock").is_ok());
    }
    for _ in 0..12 {
        w.tick();
    }
    for name in ["alice", "bob"] {
        w.logout(name);
    }
    let saved = w.export();
    store.save(&saved).await.unwrap();
    assert_eq!(store.load().await.unwrap().as_ref(), Some(&saved));

    // Saving again replaces rows, and systems missing from the save are removed,
    // as when bob graduates and his pocket is deleted.
    let mut fewer = saved.clone();
    fewer.systems.remove("tutorial:bob");
    fewer.characters.get_mut("bob").unwrap()["system"] = "kesh".into();
    fewer.tick += 5;
    store.save(&fewer).await.unwrap();
    let loaded = store.load().await.unwrap().unwrap();
    assert_eq!(loaded.tick, saved.tick + 5);
    assert!(!loaded.systems.contains_key("tutorial:bob"));
    assert!(loaded.systems.contains_key("tutorial:alice"));

    // Migrations run again on every start, and must be a no-op the second time.
    let store = Store::connect(&db.url).await.unwrap();
    let mut w = restore(store.load().await.unwrap().unwrap().clone());
    w.login("alice");
    let ship = w.snapshot("alice").unwrap().ship.unwrap();
    assert_eq!(ship.state(), ShipState::Idle);
    assert!(w.execute("alice", "buy ship sparrow").refusal != protocol::Refusal::None);

    db.drop().await;
}
