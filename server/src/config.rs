//! Tuning numbers and world content, loaded from `config/`. Nothing here is code.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

pub type Pos = [f64; 2];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameConfig {
    /// Real milliseconds per tick. One tick is always one second of game time,
    /// so tests can run the real numbers faster by lowering only this.
    pub tick_ms: u32,
    pub starting_credits: u64,
    pub logout_grace_s: u32,
    /// Seconds of game time between saves to Postgres.
    pub save_every_s: u32,
    pub timers: Timers,
    pub ranges: Ranges,
    pub ships: BTreeMap<String, ShipConfig>,
    pub lasers: BTreeMap<String, LaserConfig>,
    pub training: Training,
    pub skills: BTreeMap<String, SkillConfig>,
    pub rich_vein: RichVein,
    pub asteroid: Asteroid,
    pub market: Market,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Timers {
    pub undock_s: u32,
    pub align_s: u32,
    pub dock_s: u32,
    pub warp_min_s: u32,
    pub warp_max_s: u32,
    pub warp_max_distance: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ranges {
    pub docking_km: f64,
    pub gate_km: f64,
    pub approach_stop_km: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipConfig {
    pub name: String,
    pub price: u64,
    pub hold_m3: u32,
    pub speed_km_s: f64,
    pub laser: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaserConfig {
    pub name: String,
    pub price: u64,
    pub yield_m3: u32,
    pub cycle_s: u32,
    pub range_km: f64,
    /// Skill levels needed to buy and fit it, by skill id.
    #[serde(default)]
    pub requires: BTreeMap<String, u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Training {
    pub points_per_hour: f64,
    /// Points for level I of a rank 1 skill.
    pub base_points: f64,
    /// How much more each level costs than the one before.
    pub level_multiplier: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillConfig {
    pub name: String,
    /// Multiplies the points for every level. Harder skills have higher ranks.
    pub rank: u32,
    /// 0 for basic skills, which every character can train. Advanced skills
    /// need their book bought first.
    pub book_price: u64,
    #[serde(default)]
    pub requires: BTreeMap<String, u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RichVein {
    pub yield_multiplier: u32,
    pub lifetime_s: u32,
    pub trigger_hold_m3: u32,
    pub spawn_distance_km: f64,
    pub volume_m3: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asteroid {
    pub regrowth_m3_per_hour: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Market {
    pub ferrite_price: u64,
    pub order_volume_m3: u32,
    pub order_refill_m3_per_hour: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldConfig {
    pub home_system: String,
    pub tutorial_system: String,
    pub graduation_arrival: Pos,
    pub systems: Vec<SystemConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemConfig {
    pub id: String,
    pub name: String,
    pub security: f64,
    pub station: StationConfig,
    pub grids: Vec<GridConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StationConfig {
    pub id: String,
    pub name: String,
    pub services: Vec<String>,
    pub undock_pos: Pos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GridKind {
    Station,
    Belt,
    Gate,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridConfig {
    pub id: String,
    pub name: String,
    pub kind: GridKind,
    pub system_pos: Pos,
    pub arrival: Pos,
    #[serde(default)]
    pub rocks: Vec<RockConfig>,
    #[serde(default)]
    pub npc_miners: Vec<NpcConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RockConfig {
    pub ore: String,
    pub volume_m3: u32,
    pub pos: Pos,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpcConfig {
    pub name: String,
    pub pos: Pos,
}

/// The name of the grid holding a station. Every system must have one.
pub const STATION_GRID: &str = "station";

impl WorldConfig {
    pub fn system(&self, id: &str) -> Option<&SystemConfig> {
        self.systems.iter().find(|s| s.id == id)
    }
}

impl SystemConfig {
    pub fn grid(&self, id: &str) -> Option<&GridConfig> {
        self.grids.iter().find(|g| g.id == id)
    }
}

pub fn parse(game: &str, world: &str) -> Result<(GameConfig, WorldConfig)> {
    let game: GameConfig = toml::from_str(game).context("parsing game.toml")?;
    let world: WorldConfig = toml::from_str(world).context("parsing world.toml")?;
    validate(&game, &world)?;
    Ok((game, world))
}

pub fn load(dir: &Path) -> Result<(GameConfig, WorldConfig)> {
    let read = |name: &str| {
        let path = dir.join(name);
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    };
    parse(&read("game.toml")?, &read("world.toml")?)
}

fn validate(game: &GameConfig, world: &WorldConfig) -> Result<()> {
    ensure!(game.tick_ms > 0, "tick_ms must be above zero");
    ensure!(game.save_every_s > 0, "save_every_s must be above zero");
    for (id, ship) in &game.ships {
        ensure!(
            game.lasers.contains_key(&ship.laser),
            "ship {id} fits unknown laser {}",
            ship.laser
        );
    }
    let skill_lists = game
        .lasers
        .iter()
        .map(|(id, laser)| (id, &laser.requires))
        .chain(game.skills.iter().map(|(id, skill)| (id, &skill.requires)));
    for (owner, requires) in skill_lists {
        for (skill, level) in requires {
            ensure!(
                game.skills.contains_key(skill),
                "{owner} requires unknown skill {skill}"
            );
            ensure!(
                (1..=crate::skills::MAX_LEVEL).contains(level),
                "{owner} requires {skill} at level {level}, outside 1 to 5"
            );
        }
    }
    for (id, skill) in &game.skills {
        ensure!(skill.rank > 0, "skill {id} needs a rank above zero");
        ensure!(
            !skill.requires.contains_key(id),
            "skill {id} requires itself"
        );
    }
    for id in [&world.home_system, &world.tutorial_system] {
        ensure!(world.system(id).is_some(), "unknown system {id}");
    }
    for system in &world.systems {
        match system.grid(STATION_GRID) {
            Some(grid) if grid.kind == GridKind::Station => {}
            _ => bail!("system {} needs a station grid", system.id),
        }
    }
    Ok(())
}
