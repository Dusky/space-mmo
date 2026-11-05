//! Postgres storage. The world lives in memory; this only saves and loads it.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::sim::Saved;

pub struct Store {
    pool: PgPool,
}

impl Store {
    /// Connects and brings the schema up to date.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(10))
            .connect(url)
            .await
            .context("connecting to Postgres")?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .context("running migrations")?;
        Ok(Self { pool })
    }

    /// The last save, or `None` for a brand new database.
    pub async fn load(&self) -> Result<Option<Saved>> {
        let meta: Option<(i64, i64)> =
            sqlx::query_as("select tick, next_id from world_meta where id = 1")
                .fetch_optional(&self.pool)
                .await?;
        let Some((tick, next_id)) = meta else {
            return Ok(None);
        };
        let systems: Vec<(String, Value)> = sqlx::query_as("select key, state from systems")
            .fetch_all(&self.pool)
            .await?;
        let characters: Vec<(String, Value)> =
            sqlx::query_as("select account, state from characters")
                .fetch_all(&self.pool)
                .await?;
        Ok(Some(Saved {
            tick: u64::try_from(tick).context("negative tick")?,
            next_id: u64::try_from(next_id).context("negative id")?,
            systems: systems.into_iter().collect(),
            characters: characters.into_iter().collect(),
        }))
    }

    /// Writes a whole save in one transaction, so a crash never leaves half of one.
    pub async fn save(&self, saved: &Saved) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "insert into world_meta (id, tick, next_id, saved_at) values (1, $1, $2, now())
             on conflict (id) do update
             set tick = excluded.tick, next_id = excluded.next_id, saved_at = excluded.saved_at",
        )
        .bind(i64::try_from(saved.tick)?)
        .bind(i64::try_from(saved.next_id)?)
        .execute(&mut *tx)
        .await?;

        let (keys, states): (Vec<String>, Vec<Value>) = saved
            .systems
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .unzip();
        sqlx::query(
            "insert into systems (key, state) select * from unnest($1::text[], $2::jsonb[])
             on conflict (key) do update set state = excluded.state",
        )
        .bind(&keys)
        .bind(&states)
        .execute(&mut *tx)
        .await?;
        // Tutorial pockets are deleted at graduation, so drop their rows too.
        sqlx::query("delete from systems where key <> all($1)")
            .bind(&keys)
            .execute(&mut *tx)
            .await?;

        let (accounts, states): (Vec<String>, Vec<Value>) = saved
            .characters
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .unzip();
        sqlx::query(
            "insert into characters (account, state, updated_at)
             select a, s, now() from unnest($1::text[], $2::jsonb[]) as t(a, s)
             on conflict (account) do update
             set state = excluded.state, updated_at = excluded.updated_at",
        )
        .bind(&accounts)
        .bind(&states)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }
}
