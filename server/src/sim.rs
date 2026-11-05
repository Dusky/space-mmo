//! The authoritative game state. No I/O: one `tick()` is one second of game
//! time, and `execute()` is the only way a player can change anything.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use anyhow::{ensure, Context};
use protocol as pb;
use protocol::{ContactKind, Refusal, ShipState, TutorialStep};
use serde::{Deserialize, Serialize};

use crate::config::{
    GameConfig, GridKind, LaserConfig, Pos, SystemConfig, WorldConfig, STATION_GRID,
};
use crate::skills;

const MAX_COMMAND_BYTES: usize = 256;
const SECONDS_PER_HOUR: f64 = 3600.0;
const FERRITE: &str = "ferrite";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Activity {
    Docked,
    Undocking {
        left: u32,
    },
    Idle,
    Aligning {
        left: u32,
        dest: String,
        warp_ticks: u32,
    },
    Warping {
        left: u32,
        dest: String,
    },
    Approaching {
        target: String,
    },
    Mining {
        rock: String,
        left: u32,
    },
    Docking {
        left: u32,
    },
    /// Warped to safety after logout. Off every grid until the next login.
    Stowed,
}

#[derive(Debug, Serialize, Deserialize)]
struct Ship {
    id: String,
    type_id: String,
    laser: String,
    activity: Activity,
    grid: Option<String>,
    pos: Pos,
    hold: BTreeMap<String, u32>,
}

impl Ship {
    fn hold_used(&self) -> u32 {
        self.hold.values().sum()
    }
}

/// Fields beyond these (faction and so on) arrive with the systems that use them.
#[derive(Debug, Serialize, Deserialize)]
struct Character {
    name: String,
    credits: u64,
    system: String,
    ship: Option<Ship>,
    #[serde(with = "step_as_number")]
    step: TutorialStep,
    /// Runtime only: everyone is offline after a restart.
    #[serde(skip)]
    online: bool,
    logged_out_at: u64,
    #[serde(skip)]
    events: Vec<String>,
    /// Skill levels by id. Nothing trains them yet.
    skills: skills::Levels,
}

impl Character {
    fn is_docked(&self) -> bool {
        self.ship
            .as_ref()
            .is_none_or(|s| s.activity == Activity::Docked)
    }

    /// Moves the tutorial forward, never back, and never restarts it after graduation.
    fn advance(&mut self, to: TutorialStep) {
        if self.step != TutorialStep::None && self.step < to {
            self.step = to;
        }
    }
}

/// Saves the tutorial step as its protocol number, which never changes meaning.
mod step_as_number {
    use protocol::TutorialStep;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(step: &TutorialStep, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_i32(*step as i32)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<TutorialStep, D::Error> {
        let number = i32::deserialize(input)?;
        TutorialStep::try_from(number)
            .map_err(|_| D::Error::custom(format!("unknown tutorial step {number}")))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Rock {
    id: String,
    ore: String,
    volume: f64,
    max_volume: f64,
    pos: Pos,
    /// The tutorial's scripted rocks never run dry.
    infinite: bool,
    /// Set on rich veins, the tick they fade.
    vein_expires: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct GridInstance {
    rocks: Vec<Rock>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SystemInstance {
    template: String,
    is_tutorial: bool,
    grids: HashMap<String, GridInstance>,
    orders: BTreeMap<String, f64>,
    vein_spawned: bool,
}

/// A copy of the world for saving. Taken between ticks, written elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub tick: u64,
    pub next_id: u64,
    pub systems: BTreeMap<String, serde_json::Value>,
    pub characters: BTreeMap<String, serde_json::Value>,
}

fn to_json<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("world state is plain data")
}

/// What a command returns to the client.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub refusal: Refusal,
    pub message: String,
}

impl Outcome {
    pub fn is_ok(&self) -> bool {
        self.refusal == Refusal::None
    }
}

struct Refused(Refusal, String);
type CommandResult = Result<String, Refused>;

fn refuse<T>(refusal: Refusal, message: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(refusal, message.into()))
}

fn distance(a: Pos, b: Pos) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn title_case(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub struct World {
    cfg: Arc<GameConfig>,
    content: Arc<WorldConfig>,
    tick: u64,
    next_id: u64,
    systems: HashMap<String, SystemInstance>,
    characters: HashMap<String, Character>,
}

impl World {
    pub fn new(cfg: GameConfig, content: WorldConfig) -> Self {
        let mut world = Self {
            cfg: Arc::new(cfg),
            content: Arc::new(content),
            tick: 0,
            next_id: 1,
            systems: HashMap::new(),
            characters: HashMap::new(),
        };
        let home = world.content.home_system.clone();
        world.create_system(&home, &home, false);
        world
    }

    pub fn tick_ms(&self) -> u32 {
        self.cfg.tick_ms
    }

    pub fn current_tick(&self) -> u64 {
        self.tick
    }

    pub fn save_every_ticks(&self) -> u64 {
        u64::from(self.cfg.save_every_s)
    }

    /// Everything a restart needs, one JSON document per system and character.
    pub fn export(&self) -> Saved {
        Saved {
            tick: self.tick,
            next_id: self.next_id,
            systems: self
                .systems
                .iter()
                .map(|(k, s)| (k.clone(), to_json(s)))
                .collect(),
            characters: self
                .characters
                .iter()
                .map(|(k, c)| (k.clone(), to_json(c)))
                .collect(),
        }
    }

    /// Rebuilds a saved world. Everyone starts offline, so ships left in space
    /// get the usual logout grace before warping to safety. Fails loudly if the
    /// save refers to content or config that no longer exists.
    pub fn restore(cfg: GameConfig, content: WorldConfig, saved: Saved) -> anyhow::Result<Self> {
        let mut world = Self {
            cfg: Arc::new(cfg),
            content: Arc::new(content),
            tick: saved.tick,
            next_id: saved.next_id,
            systems: HashMap::new(),
            characters: HashMap::new(),
        };
        for (key, value) in saved.systems {
            let system: SystemInstance =
                serde_json::from_value(value).with_context(|| format!("loading system {key}"))?;
            let source = world.content.system(&system.template).with_context(|| {
                format!("system {key} uses unknown template {}", system.template)
            })?;
            for grid in system.grids.keys() {
                ensure!(
                    source.grid(grid).is_some(),
                    "system {key} has unknown grid {grid}"
                );
            }
            world.systems.insert(key, system);
        }
        let home = world.content.home_system.clone();
        if !world.systems.contains_key(&home) {
            world.create_system(&home, &home, false);
        }
        for (account, value) in saved.characters {
            let mut ch: Character = serde_json::from_value(value)
                .with_context(|| format!("loading character {account}"))?;
            let system = world
                .systems
                .get(&ch.system)
                .with_context(|| format!("{account} is in missing system {}", ch.system))?;
            if let Some(ship) = &ch.ship {
                ensure!(
                    world.cfg.ships.contains_key(&ship.type_id),
                    "{account} flies unknown ship {}",
                    ship.type_id
                );
                ensure!(
                    world.cfg.lasers.contains_key(&ship.laser),
                    "{account} fits unknown laser {}",
                    ship.laser
                );
                if let Some(grid) = &ship.grid {
                    ensure!(
                        system.grids.contains_key(grid),
                        "{account} is on unknown grid {grid}"
                    );
                }
            }
            ch.online = false;
            ch.logged_out_at = world.tick;
            world.characters.insert(account, ch);
        }
        Ok(world)
    }

    fn create_system(&mut self, key: &str, template: &str, is_tutorial: bool) {
        let content = Arc::clone(&self.content);
        let source = content.system(template).expect("validated at load");
        let mut grids = HashMap::new();
        for grid in &source.grids {
            let mut rocks = Vec::new();
            for rock in &grid.rocks {
                rocks.push(Rock {
                    id: format!("rock-{}", self.allocate_id()),
                    ore: rock.ore.clone(),
                    volume: f64::from(rock.volume_m3),
                    max_volume: f64::from(rock.volume_m3),
                    pos: rock.pos,
                    infinite: is_tutorial,
                    vein_expires: None,
                });
            }
            grids.insert(grid.id.clone(), GridInstance { rocks });
        }
        let orders = BTreeMap::from([(
            FERRITE.to_string(),
            f64::from(self.cfg.market.order_volume_m3),
        )]);
        self.systems.insert(
            key.to_string(),
            SystemInstance {
                template: template.to_string(),
                is_tutorial,
                grids,
                orders,
                vein_spawned: false,
            },
        );
    }

    fn allocate_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Marks the account online, creating the character and its private pocket
    /// on first login. A ship stowed after logout returns near the station.
    pub fn login(&mut self, account: &str) {
        if !self.characters.contains_key(account) {
            let pocket = format!("tutorial:{account}");
            let template = self.content.tutorial_system.clone();
            self.create_system(&pocket, &template, true);
            self.characters.insert(
                account.to_string(),
                Character {
                    name: account.to_string(),
                    credits: self.cfg.starting_credits,
                    system: pocket,
                    ship: None,
                    step: TutorialStep::BuyShip,
                    online: true,
                    logged_out_at: 0,
                    events: Vec::new(),
                    skills: skills::Levels::new(),
                },
            );
            return;
        }
        let content = Arc::clone(&self.content);
        let systems = &self.systems;
        let ch = self.characters.get_mut(account).expect("checked above");
        ch.online = true;
        let template = &systems[&ch.system].template;
        if let Some(ship) = ch.ship.as_mut().filter(|s| s.activity == Activity::Stowed) {
            let station = content
                .system(template)
                .and_then(|s| s.grid(STATION_GRID))
                .expect("validated at load");
            ship.activity = Activity::Idle;
            ship.grid = Some(station.id.clone());
            ship.pos = station.arrival;
            ch.events
                .push("Your ship returned from safety.".to_string());
        }
    }

    pub fn logout(&mut self, account: &str) {
        if let Some(ch) = self.characters.get_mut(account) {
            ch.online = false;
            ch.logged_out_at = self.tick;
        }
    }

    pub fn tick(&mut self) {
        self.tick += 1;
        let cfg = Arc::clone(&self.cfg);
        for system in self.systems.values_mut() {
            let refill = cfg.market.order_refill_m3_per_hour / SECONDS_PER_HOUR;
            for left in system.orders.values_mut() {
                *left = (*left + refill).min(f64::from(cfg.market.order_volume_m3));
            }
            for grid in system.grids.values_mut() {
                for rock in grid.rocks.iter_mut().filter(|r| r.vein_expires.is_none()) {
                    let growth = cfg.asteroid.regrowth_m3_per_hour / SECONDS_PER_HOUR;
                    rock.volume = (rock.volume + growth).min(rock.max_volume);
                }
            }
        }

        let content = Arc::clone(&self.content);
        let now = self.tick;
        for ch in self.characters.values_mut() {
            step_character(
                ch,
                &mut self.systems,
                &cfg,
                &content,
                now,
                &mut self.next_id,
            );
        }

        // After the ships move, so a cycle ending on the final tick still pays.
        for system in self.systems.values_mut() {
            for grid in system.grids.values_mut() {
                grid.rocks
                    .retain(|r| r.vein_expires.is_none_or(|t| t > now));
            }
        }
    }

    /// Parses and runs one verb line. The only entry point for player actions.
    pub fn execute(&mut self, account: &str, line: &str) -> Outcome {
        match self.dispatch(account, line) {
            Ok(message) => Outcome {
                refusal: Refusal::None,
                message,
            },
            Err(Refused(refusal, message)) => Outcome { refusal, message },
        }
    }

    fn dispatch(&mut self, account: &str, line: &str) -> CommandResult {
        if line.len() > MAX_COMMAND_BYTES {
            return refuse(Refusal::BadArgument, "That command is too long.");
        }
        if !self.characters.contains_key(account) {
            return refuse(Refusal::NotFound, "No character for this account.");
        }
        let lowered = line.to_lowercase();
        let words: Vec<&str> = lowered.split_whitespace().collect();
        match words.as_slice() {
            ["undock"] => self.undock(account),
            ["warp", site] => self.warp(account, site),
            ["approach", target] => self.approach(account, target),
            ["mine", rock] => self.mine(account, rock),
            ["stop"] => self.stop(account),
            ["dock"] => self.dock(account),
            ["sell", ore, quantity] => self.sell(account, ore, quantity),
            ["buy", "ship", ship] => self.buy_ship(account, ship),
            ["buy", "module", module] => self.buy_module(account, module),
            ["jump", "exit"] => self.jump_exit(account),
            [] => refuse(Refusal::UnknownCommand, "Enter a command."),
            ["warp" | "approach" | "mine" | "sell" | "buy" | "jump", ..] => refuse(
                Refusal::BadArgument,
                format!("Wrong arguments for {}.", words[0]),
            ),
            [verb, ..] => refuse(Refusal::UnknownCommand, format!("Unknown command: {verb}")),
        }
    }

    fn undock(&mut self, account: &str) -> CommandResult {
        let undock_s = self.cfg.timers.undock_s;
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        let Some(ship) = ch.ship.as_mut() else {
            return refuse(
                Refusal::NoShip,
                "You need a ship to undock. Visit the Shipyard.",
            );
        };
        if ship.activity != Activity::Docked {
            return refuse(Refusal::WrongState, "You are not docked.");
        }
        ship.activity = Activity::Undocking { left: undock_s };
        Ok(format!("Undocking in {undock_s} s."))
    }

    fn warp(&mut self, account: &str, site: &str) -> CommandResult {
        let cfg = Arc::clone(&self.cfg);
        let content = Arc::clone(&self.content);
        let source = template_of(&self.systems, &content, &self.characters[account].system);
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        let ship = idle_ship(ch, "warp")?;
        let Some(dest) = source.grid(site) else {
            return refuse(
                Refusal::NotFound,
                format!("No site called {site} in this system."),
            );
        };
        let here = ship.grid.as_deref().expect("idle ships are on a grid");
        if here == dest.id {
            return refuse(
                Refusal::WrongState,
                format!("You are already at {}.", dest.name),
            );
        }
        let origin = source.grid(here).expect("ships stay on known grids");
        let timers = &cfg.timers;
        let fraction =
            (distance(origin.system_pos, dest.system_pos) / timers.warp_max_distance).min(1.0);
        let span = f64::from(timers.warp_max_s - timers.warp_min_s);
        let warp_ticks = timers.warp_min_s + (fraction * span).round() as u32;
        ship.activity = Activity::Aligning {
            left: timers.align_s,
            dest: dest.id.clone(),
            warp_ticks,
        };
        Ok(format!("Aligning to {}.", dest.name))
    }

    fn approach(&mut self, account: &str, target: &str) -> CommandResult {
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        let content = &self.content;
        let system = &self.systems[&ch.system];
        let source = content.system(&system.template).expect("validated at load");
        let Some(ship) = ch.ship.as_ref() else {
            return refuse(Refusal::NoShip, "You have no ship.");
        };
        if !matches!(ship.activity, Activity::Idle | Activity::Approaching { .. }) {
            return refuse(Refusal::WrongState, "Your ship can't approach right now.");
        }
        let grid = ship.grid.clone().expect("idle ships are on a grid");
        if fixed_position(system, source, &grid, target).is_none() {
            return refuse(
                Refusal::NotFound,
                format!("Nothing called {target} on this grid."),
            );
        }
        let ship = ch.ship.as_mut().expect("checked above");
        ship.activity = Activity::Approaching {
            target: target.to_string(),
        };
        ch.advance(TutorialStep::Mine);
        Ok(format!("Approaching {target}."))
    }

    fn mine(&mut self, account: &str, rock_id: &str) -> CommandResult {
        let cfg = Arc::clone(&self.cfg);
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        let system = &self.systems[&ch.system];
        let Some(ship) = ch.ship.as_ref() else {
            return refuse(Refusal::NoShip, "You have no ship.");
        };
        // Mining to Mining is allowed, so a player can switch to a rich vein.
        if !matches!(
            ship.activity,
            Activity::Idle | Activity::Approaching { .. } | Activity::Mining { .. }
        ) {
            return refuse(Refusal::WrongState, "Your ship can't mine right now.");
        }
        let grid = ship.grid.as_deref().expect("these states are on a grid");
        let rock = system
            .grids
            .get(grid)
            .and_then(|g| g.rocks.iter().find(|r| r.id == rock_id));
        let Some(rock) = rock else {
            return refuse(
                Refusal::NotFound,
                format!("No rock called {rock_id} on this grid."),
            );
        };
        let laser = &cfg.lasers[&ship.laser];
        let range = distance(ship.pos, rock.pos);
        if range > laser.range_km {
            return refuse(
                Refusal::OutOfRange,
                format!(
                    "{rock_id} is {range:.1} km away. Laser range is {} km.",
                    laser.range_km
                ),
            );
        }
        if rock.volume < 1.0 {
            return refuse(Refusal::NotFound, format!("{rock_id} is empty."));
        }
        if ship.hold_used() >= cfg.ships[&ship.type_id].hold_m3 {
            return refuse(Refusal::HoldFull, "Your ore hold is full.");
        }
        let ship = ch.ship.as_mut().expect("checked above");
        ship.activity = Activity::Mining {
            rock: rock_id.to_string(),
            left: laser.cycle_s,
        };
        ch.advance(TutorialStep::Mine);
        Ok(format!("Mining {rock_id}."))
    }

    fn stop(&mut self, account: &str) -> CommandResult {
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        let Some(ship) = ch.ship.as_mut() else {
            return refuse(Refusal::NoShip, "You have no ship.");
        };
        if !matches!(
            ship.activity,
            Activity::Approaching { .. } | Activity::Mining { .. }
        ) {
            return refuse(Refusal::WrongState, "Nothing to stop.");
        }
        ship.activity = Activity::Idle;
        Ok("Stopped.".to_string())
    }

    fn dock(&mut self, account: &str) -> CommandResult {
        let cfg = Arc::clone(&self.cfg);
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        let ship = idle_ship(ch, "dock")?;
        if ship.grid.as_deref() != Some(STATION_GRID) {
            return refuse(Refusal::OutOfRange, "There is no station on this grid.");
        }
        let range = distance(ship.pos, [0.0, 0.0]);
        if range > cfg.ranges.docking_km {
            return refuse(
                Refusal::OutOfRange,
                format!(
                    "Station is {range:.1} km away. Docking range is {} km.",
                    cfg.ranges.docking_km
                ),
            );
        }
        ship.activity = Activity::Docking {
            left: cfg.timers.dock_s,
        };
        Ok("Docking.".to_string())
    }

    fn sell(&mut self, account: &str, ore: &str, quantity: &str) -> CommandResult {
        let price = match ore {
            FERRITE => self.cfg.market.ferrite_price,
            _ => return refuse(Refusal::NoDemand, format!("Nobody here buys {ore}.")),
        };
        let content = Arc::clone(&self.content);
        let source = template_of(&self.systems, &content, &self.characters[account].system);
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        if !ch.is_docked() || !source.station.services.iter().any(|s| s == "market") {
            return refuse(
                Refusal::ServiceUnavailable,
                "You must be docked at a market to sell.",
            );
        }
        let Some(ship) = ch.ship.as_mut() else {
            return refuse(Refusal::NothingToSell, "You have nothing to sell.");
        };
        let held = ship.hold.get(ore).copied().unwrap_or(0);
        if held == 0 {
            return refuse(
                Refusal::NothingToSell,
                format!("You have no {ore} to sell."),
            );
        }
        let wanted = match quantity {
            "all" => held,
            text => match text.parse::<u32>() {
                Ok(n) if n > 0 => n.min(held),
                _ => return refuse(Refusal::BadArgument, "Sell a whole number of m³, or all."),
            },
        };
        let system = self
            .systems
            .get_mut(&ch.system)
            .expect("characters are in a system");
        let order = system
            .orders
            .get_mut(ore)
            .expect("priced ores have an order");
        let filled = wanted.min(order.floor() as u32);
        if filled == 0 {
            return refuse(
                Refusal::NoDemand,
                format!("The {ore} buy order here is empty for now."),
            );
        }
        *order -= f64::from(filled);
        let payout = u64::from(filled) * price;
        ch.credits += payout;
        let ship = ch.ship.as_mut().expect("checked above");
        if held == filled {
            ship.hold.remove(ore);
        } else {
            ship.hold.insert(ore.to_string(), held - filled);
        }
        ch.advance(TutorialStep::Exit);
        Ok(format!(
            "Sold {filled} m³ of {} for {payout} cr.",
            title_case(ore)
        ))
    }

    fn buy_ship(&mut self, account: &str, id: &str) -> CommandResult {
        let cfg = Arc::clone(&self.cfg);
        let Some(ship_cfg) = cfg.ships.get(id) else {
            return refuse(Refusal::NotFound, format!("No ship called {id} for sale."));
        };
        let content = Arc::clone(&self.content);
        let source = template_of(&self.systems, &content, &self.characters[account].system);
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        if !at_shipyard(ch, source) {
            return refuse(
                Refusal::ServiceUnavailable,
                "You must be docked at a Shipyard to buy a ship.",
            );
        }
        if ch.ship.is_some() {
            return refuse(Refusal::AlreadyOwned, "You already own a ship.");
        }
        if ch.credits < ship_cfg.price {
            return refuse(
                Refusal::InsufficientCredits,
                format!("The {} costs {} cr.", ship_cfg.name, ship_cfg.price),
            );
        }
        ch.credits -= ship_cfg.price;
        let serial = self.next_id;
        self.next_id += 1;
        ch.ship = Some(Ship {
            id: format!("ship-{serial}"),
            type_id: id.to_string(),
            laser: ship_cfg.laser.clone(),
            activity: Activity::Docked,
            grid: None,
            pos: [0.0, 0.0],
            hold: BTreeMap::new(),
        });
        ch.advance(TutorialStep::Undock);
        Ok(format!(
            "Bought a {}. It is now your active ship.",
            ship_cfg.name
        ))
    }

    fn buy_module(&mut self, account: &str, id: &str) -> CommandResult {
        let cfg = Arc::clone(&self.cfg);
        let module = cfg.lasers.get(id).filter(|l| l.price > 0);
        let Some(module) = module else {
            return refuse(
                Refusal::NotFound,
                format!("No module called {id} for sale."),
            );
        };
        let content = Arc::clone(&self.content);
        let source = template_of(&self.systems, &content, &self.characters[account].system);
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        if !at_shipyard(ch, source) {
            return refuse(
                Refusal::ServiceUnavailable,
                "You must be docked at a Shipyard to buy modules.",
            );
        }
        let credits = ch.credits;
        let unmet = skills::missing(&ch.skills, &module.requires);
        let Some(ship) = ch.ship.as_mut() else {
            return refuse(Refusal::NoShip, "You need a ship to fit that to.");
        };
        if ship.laser == id {
            return refuse(
                Refusal::AlreadyOwned,
                format!("{} is already fitted.", module.name),
            );
        }
        if !unmet.is_empty() {
            return refuse(
                Refusal::MissingSkill,
                format!(
                    "{} requires {}.",
                    module.name,
                    skills::describe(&cfg, &unmet)
                ),
            );
        }
        if credits < module.price {
            return refuse(
                Refusal::InsufficientCredits,
                format!("{} costs {} cr.", module.name, module.price),
            );
        }
        ship.laser = id.to_string();
        ch.credits -= module.price;
        Ok(format!("{} fitted.", module.name))
    }

    fn jump_exit(&mut self, account: &str) -> CommandResult {
        let cfg = Arc::clone(&self.cfg);
        let home = self.content.home_system.clone();
        let arrival = self.content.graduation_arrival;
        let content = Arc::clone(&self.content);
        let system_key = self.characters[account].system.clone();
        let is_tutorial = self.systems[&system_key].is_tutorial;
        let source = template_of(&self.systems, &content, &system_key);
        let ch = self
            .characters
            .get_mut(account)
            .expect("checked in dispatch");
        let step = ch.step;
        let ship = idle_ship(ch, "jump")?;
        if !is_tutorial {
            return refuse(Refusal::NotFound, "There is no exit gate in this system.");
        }
        let on_gate = ship
            .grid
            .as_deref()
            .and_then(|g| source.grid(g))
            .is_some_and(|g| g.kind == GridKind::Gate);
        if !on_gate {
            return refuse(Refusal::OutOfRange, "Warp to the exit gate first.");
        }
        let range = distance(ship.pos, [0.0, 0.0]);
        if range > cfg.ranges.gate_km {
            return refuse(
                Refusal::OutOfRange,
                format!(
                    "The gate is {range:.1} km away. Jump range is {} km.",
                    cfg.ranges.gate_km
                ),
            );
        }
        if step != TutorialStep::Exit {
            return refuse(
                Refusal::TutorialIncomplete,
                "Finish your training first: mine ore and sell it at the station.",
            );
        }
        ship.grid = Some(STATION_GRID.to_string());
        ship.pos = arrival;
        ch.system = home;
        ch.step = TutorialStep::None;
        self.systems.remove(&system_key);
        Ok("Jump complete. Welcome to your home system.".to_string())
    }

    /// Builds this player's view. Drains their pending events, so call once per send.
    pub fn snapshot(&mut self, account: &str) -> Option<pb::Snapshot> {
        let viewer_pos = self.characters.get(account)?.ship.as_ref().map(|s| s.pos);
        let contacts = self.contacts_for(account, viewer_pos);
        let cfg = Arc::clone(&self.cfg);
        let content = Arc::clone(&self.content);
        let ch = self.characters.get_mut(account)?;
        let system = self.systems.get(&ch.system)?;
        let source = content.system(&system.template)?;

        let station = ch
            .is_docked()
            .then(|| station_view(&cfg, source, system, ch));
        let ship = ch.ship.as_ref().map(|s| own_ship(&cfg, s));
        Some(pb::Snapshot {
            tick: self.tick,
            tick_ms: cfg.tick_ms,
            character: ch.name.clone(),
            credits: ch.credits,
            tutorial_step: ch.step as i32,
            system: source.name.clone(),
            security: source.security,
            sites: source
                .grids
                .iter()
                .map(|g| pb::Site {
                    id: g.id.clone(),
                    name: g.name.clone(),
                })
                .collect(),
            ship,
            station,
            contacts,
            events: std::mem::take(&mut ch.events),
        })
    }

    /// Radar: only what shares this player's grid, and nothing at all off-grid.
    fn contacts_for(&self, account: &str, viewer_pos: Option<Pos>) -> Vec<pb::Contact> {
        let ch = &self.characters[account];
        let (Some(viewer), Some(ship)) = (viewer_pos, ch.ship.as_ref()) else {
            return Vec::new();
        };
        let Some(grid_id) = ship.grid.as_deref() else {
            return Vec::new();
        };
        let system = &self.systems[&ch.system];
        let source = self
            .content
            .system(&system.template)
            .expect("validated at load");
        let grid_cfg = source.grid(grid_id).expect("ships stay on known grids");
        let mut contacts = Vec::new();
        let mut push = |id: String,
                        kind: ContactKind,
                        name: String,
                        pos: Pos,
                        activity: &str,
                        ore: String,
                        volume: u32| {
            contacts.push(pb::Contact {
                id,
                kind: kind as i32,
                name,
                x_km: pos[0],
                y_km: pos[1],
                distance_km: distance(viewer, pos),
                activity: activity.to_string(),
                ore,
                volume_left_m3: volume,
            });
        };
        match grid_cfg.kind {
            GridKind::Station => push(
                STATION_GRID.to_string(),
                ContactKind::Station,
                source.station.name.clone(),
                [0.0, 0.0],
                "",
                String::new(),
                0,
            ),
            GridKind::Gate => push(
                grid_cfg.id.clone(),
                ContactKind::Gate,
                grid_cfg.name.clone(),
                [0.0, 0.0],
                "",
                String::new(),
                0,
            ),
            GridKind::Belt => {}
        }
        for rock in &system.grids[grid_id].rocks {
            let (kind, name) = match rock.vein_expires {
                Some(_) => (
                    ContactKind::RichVein,
                    format!("Rich {} vein", title_case(&rock.ore)),
                ),
                None => (
                    ContactKind::Asteroid,
                    format!("{} asteroid", title_case(&rock.ore)),
                ),
            };
            push(
                rock.id.clone(),
                kind,
                name,
                rock.pos,
                "",
                rock.ore.clone(),
                rock.volume as u32,
            );
        }
        for (i, npc) in grid_cfg.npc_miners.iter().enumerate() {
            push(
                format!("npc-{grid_id}-{i}"),
                ContactKind::NpcShip,
                npc.name.clone(),
                npc.pos,
                "mining",
                String::new(),
                0,
            );
        }
        for other in self.characters.values() {
            if other.name == ch.name || other.system != ch.system {
                continue;
            }
            let Some(other_ship) = other.ship.as_ref() else {
                continue;
            };
            if other_ship.grid.as_deref() != Some(grid_id) {
                continue;
            }
            push(
                other_ship.id.clone(),
                ContactKind::PlayerShip,
                other.name.clone(),
                other_ship.pos,
                visible_activity(&other_ship.activity),
                String::new(),
                0,
            );
        }
        contacts
    }
}

fn template_of<'a>(
    systems: &HashMap<String, SystemInstance>,
    content: &'a WorldConfig,
    key: &str,
) -> &'a SystemConfig {
    content
        .system(&systems[key].template)
        .expect("validated at load")
}

fn requirement_summary(cfg: &GameConfig, laser: &LaserConfig) -> String {
    let stats = format!("{} m³ per {} s cycle", laser.yield_m3, laser.cycle_s);
    if laser.requires.is_empty() {
        return stats;
    }
    let requires: Vec<(String, u8)> = laser
        .requires
        .iter()
        .map(|(s, l)| (s.clone(), *l))
        .collect();
    format!("{stats}. Requires {}", skills::describe(cfg, &requires))
}

fn at_shipyard(ch: &Character, source: &SystemConfig) -> bool {
    ch.is_docked() && source.station.services.iter().any(|s| s == "shipyard")
}

fn idle_ship<'a>(ch: &'a mut Character, verb: &str) -> Result<&'a mut Ship, Refused> {
    let Some(ship) = ch.ship.as_mut() else {
        return refuse(Refusal::NoShip, "You have no ship.");
    };
    if ship.activity != Activity::Idle {
        return refuse(
            Refusal::WrongState,
            format!("Your ship can't {verb} right now."),
        );
    }
    Ok(ship)
}

/// Where a non-ship object sits on a grid: the station, a gate or a rock.
fn fixed_position(
    system: &SystemInstance,
    source: &SystemConfig,
    grid: &str,
    id: &str,
) -> Option<Pos> {
    let kind = source.grid(grid)?.kind;
    if (kind == GridKind::Station && id == STATION_GRID) || (kind == GridKind::Gate && id == grid) {
        return Some([0.0, 0.0]);
    }
    system
        .grids
        .get(grid)?
        .rocks
        .iter()
        .find(|r| r.id == id)
        .map(|r| r.pos)
}

/// What other players can see of a ship. Timers, targets and yield stay private.
fn visible_activity(activity: &Activity) -> &'static str {
    match activity {
        Activity::Mining { .. } => "mining",
        Activity::Aligning { .. } => "aligning",
        Activity::Approaching { .. } => "moving",
        Activity::Docking { .. } => "docking",
        _ => "idle",
    }
}

fn step_character(
    ch: &mut Character,
    systems: &mut HashMap<String, SystemInstance>,
    cfg: &GameConfig,
    content: &WorldConfig,
    now: u64,
    next_id: &mut u64,
) {
    let Some(ship) = ch.ship.as_mut() else { return };

    if !ch.online
        && now.saturating_sub(ch.logged_out_at) >= u64::from(cfg.logout_grace_s)
        && !matches!(ship.activity, Activity::Docked | Activity::Stowed)
    {
        ship.activity = Activity::Stowed;
        ship.grid = None;
        return;
    }

    let system = systems
        .get_mut(&ch.system)
        .expect("characters are in a system");
    let source = content.system(&system.template).expect("validated at load");
    let ship_cfg = &cfg.ships[&ship.type_id];

    match &mut ship.activity {
        Activity::Docked | Activity::Idle | Activity::Stowed => {}
        Activity::Undocking { left } => {
            *left -= 1;
            if *left == 0 {
                ship.activity = Activity::Idle;
                ship.grid = Some(STATION_GRID.to_string());
                ship.pos = source.station.undock_pos;
                ch.advance(TutorialStep::WarpToBelt);
            }
        }
        Activity::Aligning {
            left,
            dest,
            warp_ticks,
        } => {
            *left -= 1;
            if *left == 0 {
                ship.activity = Activity::Warping {
                    left: *warp_ticks,
                    dest: std::mem::take(dest),
                };
                ship.grid = None;
            }
        }
        Activity::Warping { left, dest } => {
            *left -= 1;
            if *left == 0 {
                let grid = source.grid(dest).expect("warp targets are known grids");
                ship.grid = Some(grid.id.clone());
                ship.pos = grid.arrival;
                ship.activity = Activity::Idle;
                ch.events.push(format!("Arrived at {}.", grid.name));
                match grid.kind {
                    GridKind::Belt => ch.advance(TutorialStep::Approach),
                    // Carrying ore home skips ahead, even from a partly filled hold.
                    GridKind::Station if ship.hold_used() > 0 => ch.advance(TutorialStep::Dock),
                    _ => {}
                }
            }
        }
        Activity::Approaching { target } => {
            let grid = ship
                .grid
                .as_deref()
                .expect("approaching ships are on a grid");
            let Some(pos) = fixed_position(system, source, grid, target) else {
                ch.events.push("Your approach target is gone.".to_string());
                ship.activity = Activity::Idle;
                return;
            };
            let gap = distance(ship.pos, pos);
            let travel = gap - cfg.ranges.approach_stop_km;
            if travel <= ship_cfg.speed_km_s {
                let share = (travel.max(0.0)) / gap.max(f64::EPSILON);
                ship.pos[0] += (pos[0] - ship.pos[0]) * share;
                ship.pos[1] += (pos[1] - ship.pos[1]) * share;
                ch.events.push(format!("Arrived at {target}."));
                ship.activity = Activity::Idle;
            } else {
                let share = ship_cfg.speed_km_s / gap;
                ship.pos[0] += (pos[0] - ship.pos[0]) * share;
                ship.pos[1] += (pos[1] - ship.pos[1]) * share;
            }
        }
        Activity::Mining {
            rock: rock_id,
            left,
        } => {
            *left -= 1;
            if *left > 0 {
                return;
            }
            let grid_id = ship.grid.clone().expect("mining ships are on a grid");
            let laser = &cfg.lasers[&ship.laser];
            let grid = system
                .grids
                .get_mut(&grid_id)
                .expect("ships stay on known grids");
            let Some(rock) = grid.rocks.iter_mut().find(|r| r.id == *rock_id) else {
                ch.events
                    .push("The vein faded before the cycle finished.".to_string());
                ship.activity = Activity::Idle;
                return;
            };
            let multiplier = if rock.vein_expires.is_some() {
                cfg.rich_vein.yield_multiplier
            } else {
                1
            };
            let free = ship_cfg.hold_m3 - ship.hold.values().sum::<u32>();
            let mut taken = (laser.yield_m3 * multiplier).min(free);
            if !rock.infinite {
                taken = taken.min(rock.volume as u32);
                rock.volume -= f64::from(taken);
            }
            let ore = rock.ore.clone();
            let is_empty = !rock.infinite && rock.volume < 1.0;
            *ship.hold.entry(ore.clone()).or_insert(0) += taken;
            ch.events.push(format!("+{taken} m³ {}", title_case(&ore)));
            *left = laser.cycle_s;

            let used = ship.hold.values().sum::<u32>();
            if system.is_tutorial && !system.vein_spawned && used >= cfg.rich_vein.trigger_hold_m3 {
                system.vein_spawned = true;
                let id = format!("rock-{next_id}");
                *next_id += 1;
                let lifetime = u64::from(cfg.rich_vein.lifetime_s);
                let pos = [ship.pos[0], ship.pos[1] + cfg.rich_vein.spawn_distance_km];
                grid.rocks.push(Rock {
                    id: id.clone(),
                    ore: ore.clone(),
                    volume: f64::from(cfg.rich_vein.volume_m3),
                    max_volume: f64::from(cfg.rich_vein.volume_m3),
                    pos,
                    infinite: false,
                    vein_expires: Some(now + lifetime),
                });
                ch.events.push(format!(
                    "A rich vein surfaced nearby: {id}, {}× yield, fading in {lifetime} s.",
                    cfg.rich_vein.yield_multiplier
                ));
            }
            if used >= ship_cfg.hold_m3 {
                ch.events.push("Ore hold full.".to_string());
                ship.activity = Activity::Idle;
                ch.advance(TutorialStep::WarpToStation);
            } else if is_empty {
                ch.events.push("The rock is empty.".to_string());
                ship.activity = Activity::Idle;
            }
        }
        Activity::Docking { left } => {
            *left -= 1;
            if *left == 0 {
                ship.activity = Activity::Docked;
                ship.grid = None;
                if ch.step == TutorialStep::Dock {
                    ch.step = TutorialStep::Sell;
                }
            }
        }
    }
}

fn own_ship(cfg: &GameConfig, ship: &Ship) -> pb::OwnShip {
    let (state, timer, target) = match &ship.activity {
        Activity::Docked | Activity::Stowed => (ShipState::Docked, 0, String::new()),
        Activity::Undocking { left } => (ShipState::Undocking, *left, String::new()),
        Activity::Idle => (ShipState::Idle, 0, String::new()),
        Activity::Aligning { left, dest, .. } => (ShipState::Aligning, *left, dest.clone()),
        Activity::Warping { left, dest } => (ShipState::Warping, *left, dest.clone()),
        Activity::Approaching { target } => (ShipState::Approaching, 0, target.clone()),
        Activity::Mining { rock, left } => (ShipState::Mining, *left, rock.clone()),
        Activity::Docking { left } => (ShipState::Docking, *left, String::new()),
    };
    let laser = &cfg.lasers[&ship.laser];
    pb::OwnShip {
        type_id: ship.type_id.clone(),
        state: state as i32,
        grid: ship.grid.clone().unwrap_or_default(),
        x_km: ship.pos[0],
        y_km: ship.pos[1],
        timer_ticks: timer,
        target,
        hold_used_m3: ship.hold_used(),
        hold_capacity_m3: cfg.ships[&ship.type_id].hold_m3,
        hold: ship
            .hold
            .iter()
            .map(|(ore, volume)| pb::HoldItem {
                ore: ore.clone(),
                volume_m3: *volume,
            })
            .collect(),
        laser: ship.laser.clone(),
        laser_yield_m3: laser.yield_m3,
        laser_range_km: laser.range_km,
    }
}

fn station_view(
    cfg: &GameConfig,
    source: &SystemConfig,
    system: &SystemInstance,
    ch: &Character,
) -> pb::StationView {
    let buy_orders = system
        .orders
        .iter()
        .map(|(ore, left)| pb::BuyOrder {
            ore: ore.clone(),
            price_per_m3: cfg.market.ferrite_price,
            volume_left_m3: *left as u32,
        })
        .collect();
    let mut shipyard = Vec::new();
    match ch.ship.as_ref() {
        None => shipyard.extend(cfg.ships.iter().map(|(id, ship)| {
            let laser = &cfg.lasers[&ship.laser];
            pb::ShipyardItem {
                command: format!("buy ship {id}"),
                name: ship.name.clone(),
                price: ship.price,
                summary: format!("{} m³ ore hold, fitted with {}", ship.hold_m3, laser.name),
            }
        })),
        Some(ship) => shipyard.extend(
            cfg.lasers
                .iter()
                .filter(|(id, laser)| laser.price > 0 && **id != ship.laser)
                .map(|(id, laser)| pb::ShipyardItem {
                    command: format!("buy module {id}"),
                    name: laser.name.clone(),
                    price: laser.price,
                    summary: requirement_summary(cfg, laser),
                }),
        ),
    }
    pb::StationView {
        id: source.station.id.clone(),
        name: source.station.name.clone(),
        services: source.station.services.clone(),
        buy_orders,
        shipyard,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn world() -> World {
        let (game, content) = config::parse(
            include_str!("../../config/game.toml"),
            include_str!("../../config/world.toml"),
        )
        .unwrap();
        World::new(game, content)
    }

    fn ok(world: &mut World, account: &str, line: &str) {
        let outcome = world.execute(account, line);
        assert!(outcome.is_ok(), "{line}: {outcome:?}");
    }

    fn refused(world: &mut World, account: &str, line: &str, refusal: Refusal) {
        assert_eq!(world.execute(account, line).refusal, refusal, "{line}");
    }

    fn snap(world: &mut World, account: &str) -> pb::Snapshot {
        world.snapshot(account).unwrap()
    }

    fn state(world: &mut World, account: &str) -> ShipState {
        snap(world, account)
            .ship
            .map_or(ShipState::None, |s| s.state())
    }

    /// Ticks until the ship reaches `want`, failing if it takes over `limit`.
    fn run_until(world: &mut World, account: &str, want: ShipState, limit: u32) -> u32 {
        for elapsed in 0..=limit {
            if state(world, account) == want {
                return elapsed;
            }
            world.tick();
        }
        panic!("{account} never reached {want:?} within {limit} ticks");
    }

    fn nearest_rock(world: &mut World, account: &str) -> pb::Contact {
        snap(world, account)
            .contacts
            .into_iter()
            .filter(|c| c.kind() == ContactKind::Asteroid)
            .min_by(|a, b| a.distance_km.total_cmp(&b.distance_km))
            .unwrap()
    }

    fn to_belt(world: &mut World, account: &str) {
        ok(world, account, "buy ship sparrow");
        ok(world, account, "undock");
        run_until(world, account, ShipState::Idle, 20);
        ok(world, account, "warp belt-1");
        run_until(world, account, ShipState::Idle, 60);
    }

    fn approach_and_mine(world: &mut World, account: &str) {
        let rock = nearest_rock(world, account).id;
        ok(world, account, &format!("approach {rock}"));
        run_until(world, account, ShipState::Idle, 60);
        ok(world, account, &format!("mine {rock}"));
    }

    #[test]
    fn belts_land_outside_laser_range_and_the_station_inside_docking_range() {
        let (game, content) = config::parse(
            include_str!("../../config/game.toml"),
            include_str!("../../config/world.toml"),
        )
        .unwrap();
        let range = game.lasers["mining-laser-1"].range_km;
        for system in &content.systems {
            for grid in &system.grids {
                for rock in &grid.rocks {
                    assert!(
                        distance(grid.arrival, rock.pos) > range,
                        "{} {}",
                        system.id,
                        grid.id
                    );
                }
                if grid.kind != GridKind::Belt {
                    assert!(distance(grid.arrival, [0.0, 0.0]) <= game.ranges.docking_km);
                }
            }
        }
    }

    #[test]
    fn commands_are_refused_with_a_reason_in_the_wrong_state() {
        let mut w = world();
        w.login("a");
        refused(&mut w, "a", "undock", Refusal::NoShip);
        refused(&mut w, "a", "dance", Refusal::UnknownCommand);
        refused(&mut w, "a", "warp", Refusal::BadArgument);
        refused(&mut w, "a", "buy ship dreadnought", Refusal::NotFound);
        ok(&mut w, "a", "buy ship sparrow");
        refused(&mut w, "a", "buy ship sparrow", Refusal::AlreadyOwned);
        refused(&mut w, "a", "mine rock-1", Refusal::WrongState);
        refused(&mut w, "a", "sell ferrite all", Refusal::NothingToSell);
        ok(&mut w, "a", "undock");
        refused(&mut w, "a", "warp belt-1", Refusal::WrongState);
        run_until(&mut w, "a", ShipState::Idle, 20);
        refused(
            &mut w,
            "a",
            "buy module mining-laser-2",
            Refusal::ServiceUnavailable,
        );
        refused(&mut w, "a", "warp station", Refusal::WrongState);
        ok(&mut w, "a", "warp belt-1");
        refused(&mut w, "a", "dock", Refusal::WrongState);
        run_until(&mut w, "a", ShipState::Warping, 10);
        let rock = "rock-1";
        refused(&mut w, "a", &format!("mine {rock}"), Refusal::WrongState);
        run_until(&mut w, "a", ShipState::Idle, 60);
        refused(&mut w, "a", "dock", Refusal::OutOfRange);
    }

    #[test]
    fn mine_needs_range_and_dock_at_a_distance_is_refused() {
        let mut w = world();
        w.login("a");
        to_belt(&mut w, "a");
        let rock = nearest_rock(&mut w, "a").id;
        refused(&mut w, "a", &format!("mine {rock}"), Refusal::OutOfRange);
        ok(&mut w, "a", &format!("approach {rock}"));
        refused(&mut w, "a", "warp station", Refusal::WrongState);
        run_until(&mut w, "a", ShipState::Idle, 60);
        ok(&mut w, "a", &format!("mine {rock}"));
    }

    #[test]
    fn a_full_hold_takes_about_ten_minutes_and_sells_for_4500() {
        let mut w = world();
        w.login("a");
        to_belt(&mut w, "a");
        approach_and_mine(&mut w, "a");
        let rock = snap(&mut w, "a").ship.unwrap().target;
        let ticks = run_until(&mut w, "a", ShipState::Idle, 700);
        assert_eq!(ticks, 600, "20 cycles of 30 s");
        let ship = snap(&mut w, "a").ship.unwrap();
        assert_eq!(ship.hold_used_m3, 500);
        refused(&mut w, "a", &format!("mine {rock}"), Refusal::HoldFull);

        ok(&mut w, "a", "warp station");
        run_until(&mut w, "a", ShipState::Idle, 60);
        ok(&mut w, "a", "dock");
        run_until(&mut w, "a", ShipState::Docked, 10);
        ok(&mut w, "a", "sell ferrite all");
        assert_eq!(snap(&mut w, "a").credits, 8500);
        assert_eq!(snap(&mut w, "a").ship.unwrap().hold_used_m3, 0);
    }

    #[test]
    fn the_rich_vein_appears_on_cue_pays_triple_and_fades() {
        let mut w = world();
        w.login("a");
        to_belt(&mut w, "a");
        approach_and_mine(&mut w, "a");
        // Four cycles of 25 m³ reach the 100 m³ cue.
        for _ in 0..120 {
            w.tick();
        }
        let vein = snap(&mut w, "a")
            .contacts
            .into_iter()
            .find(|c| c.kind() == ContactKind::RichVein)
            .expect("vein appears at the cue");
        ok(&mut w, "a", &format!("mine {}", vein.id));
        for _ in 0..30 {
            w.tick();
        }
        assert_eq!(snap(&mut w, "a").ship.unwrap().hold_used_m3, 175);
        // A second cycle can't finish: the vein fades 15 ticks in.
        run_until(&mut w, "a", ShipState::Idle, 30);
        assert_eq!(snap(&mut w, "a").ship.unwrap().hold_used_m3, 175);
        let contacts = snap(&mut w, "a").contacts;
        assert!(contacts.iter().all(|c| c.kind() != ContactKind::RichVein));
    }

    #[test]
    fn switching_rocks_restarts_the_cycle() {
        let mut w = world();
        w.login("a");
        to_belt(&mut w, "a");
        approach_and_mine(&mut w, "a");
        let rock = snap(&mut w, "a").ship.unwrap().target;
        for _ in 0..20 {
            w.tick();
        }
        ok(&mut w, "a", &format!("mine {rock}"));
        assert_eq!(snap(&mut w, "a").ship.unwrap().timer_ticks, 30);
    }

    #[test]
    fn stop_returns_to_idle_without_paying_the_partial_cycle() {
        let mut w = world();
        w.login("a");
        to_belt(&mut w, "a");
        approach_and_mine(&mut w, "a");
        for _ in 0..29 {
            w.tick();
        }
        ok(&mut w, "a", "stop");
        assert_eq!(state(&mut w, "a"), ShipState::Idle);
        assert_eq!(snap(&mut w, "a").ship.unwrap().hold_used_m3, 0);
        refused(&mut w, "a", "stop", Refusal::WrongState);
    }

    /// Takes a character from a fresh login to sold-and-through-the-gate.
    fn graduate(w: &mut World, account: &str) {
        w.login(account);
        to_belt(w, account);
        approach_and_mine(w, account);
        for _ in 0..30 {
            w.tick();
        }
        ok(w, account, "stop");
        ok(w, account, "warp station");
        run_until(w, account, ShipState::Idle, 60);
        ok(w, account, "dock");
        run_until(w, account, ShipState::Docked, 10);
        ok(w, account, "sell ferrite all");
        ok(w, account, "undock");
        run_until(w, account, ShipState::Idle, 20);
        ok(w, account, "warp exit-gate");
        run_until(w, account, ShipState::Idle, 60);
        ok(w, account, "jump exit");
    }

    #[test]
    fn the_exit_gate_refuses_early_jumps_and_deletes_the_pocket() {
        let mut w = world();
        w.login("a");
        ok(&mut w, "a", "buy ship sparrow");
        ok(&mut w, "a", "undock");
        run_until(&mut w, "a", ShipState::Idle, 20);
        refused(&mut w, "a", "jump exit", Refusal::OutOfRange);
        ok(&mut w, "a", "warp exit-gate");
        run_until(&mut w, "a", ShipState::Idle, 60);
        refused(&mut w, "a", "jump exit", Refusal::TutorialIncomplete);

        let mut w = world();
        graduate(&mut w, "a");
        let s = snap(&mut w, "a");
        assert_eq!(s.system, "Kesh");
        assert_eq!(s.tutorial_step(), TutorialStep::None);
        assert!(!w.systems.contains_key("tutorial:a"));
        refused(&mut w, "a", "jump exit", Refusal::NotFound);
        // Arrives a few kilometres out, so docking needs an approach first.
        refused(&mut w, "a", "dock", Refusal::OutOfRange);
        ok(&mut w, "a", "approach station");
        run_until(&mut w, "a", ShipState::Idle, 30);
        ok(&mut w, "a", "dock");
    }

    #[test]
    fn radar_shows_only_the_players_own_grid_and_pocket() {
        let mut w = world();
        w.login("a");
        w.login("b");
        for name in ["a", "b"] {
            ok(&mut w, name, "buy ship sparrow");
            ok(&mut w, name, "undock");
            run_until(&mut w, name, ShipState::Idle, 20);
        }
        // Same template and grid name, but separate pockets.
        assert!(snap(&mut w, "a")
            .contacts
            .iter()
            .all(|c| c.kind() != ContactKind::PlayerShip));

        let mut w = world();
        graduate(&mut w, "a");
        graduate(&mut w, "b");
        let names = |s: &pb::Snapshot| -> Vec<String> {
            s.contacts
                .iter()
                .filter(|c| c.kind() == ContactKind::PlayerShip)
                .map(|c| c.name.clone())
                .collect()
        };
        assert_eq!(names(&snap(&mut w, "a")), ["b"]);
        assert_eq!(names(&snap(&mut w, "b")), ["a"]);

        ok(&mut w, "a", "warp belt-1");
        run_until(&mut w, "a", ShipState::Warping, 10);
        assert!(
            snap(&mut w, "a").contacts.is_empty(),
            "nothing is visible in warp"
        );
        assert!(
            names(&snap(&mut w, "b")).is_empty(),
            "warping ships have left the grid"
        );
        run_until(&mut w, "a", ShipState::Idle, 60);
        assert!(names(&snap(&mut w, "a")).is_empty());
    }

    #[test]
    fn other_players_see_mining_but_not_yield_or_timers() {
        let mut w = world();
        graduate(&mut w, "a");
        graduate(&mut w, "b");
        for name in ["a", "b"] {
            ok(&mut w, name, "warp belt-1");
            run_until(&mut w, name, ShipState::Idle, 60);
        }
        approach_and_mine(&mut w, "a");
        let seen = snap(&mut w, "b")
            .contacts
            .into_iter()
            .find(|c| c.kind() == ContactKind::PlayerShip)
            .unwrap();
        assert_eq!(seen.activity, "mining");
        assert_eq!(seen.volume_left_m3, 0);
        assert!(seen.ore.is_empty());
    }

    #[test]
    fn kesh_belt_2_has_npc_miners() {
        let mut w = world();
        graduate(&mut w, "a");
        ok(&mut w, "a", "warp belt-2");
        run_until(&mut w, "a", ShipState::Idle, 60);
        let npcs = snap(&mut w, "a")
            .contacts
            .iter()
            .filter(|c| c.kind() == ContactKind::NpcShip && c.activity == "mining")
            .count();
        assert_eq!(npcs, 3);
    }

    #[test]
    fn a_ship_left_in_space_stays_60_seconds_then_goes_to_safety() {
        let mut w = world();
        graduate(&mut w, "a");
        graduate(&mut w, "b");
        let sees_a = |w: &mut World| {
            snap(w, "b")
                .contacts
                .iter()
                .any(|c| c.kind() == ContactKind::PlayerShip)
        };
        w.logout("a");
        for _ in 0..59 {
            w.tick();
        }
        assert!(sees_a(&mut w), "still there at 59 s");
        w.tick();
        assert!(!sees_a(&mut w), "gone at 60 s");

        w.login("a");
        assert_eq!(state(&mut w, "a"), ShipState::Idle);
        assert!(sees_a(&mut w), "back after login");
    }

    #[test]
    fn logging_back_in_within_the_grace_period_keeps_the_ship_in_place() {
        let mut w = world();
        graduate(&mut w, "a");
        let before = snap(&mut w, "a").ship.unwrap();
        w.logout("a");
        for _ in 0..30 {
            w.tick();
        }
        w.login("a");
        for _ in 0..60 {
            w.tick();
        }
        let after = snap(&mut w, "a").ship.unwrap();
        assert_eq!((after.x_km, after.y_km), (before.x_km, before.y_km));
        assert_eq!(after.state(), ShipState::Idle);
    }

    #[test]
    fn docked_ships_are_untouched_by_logout() {
        let mut w = world();
        w.login("a");
        ok(&mut w, "a", "buy ship sparrow");
        w.logout("a");
        for _ in 0..120 {
            w.tick();
        }
        w.login("a");
        assert_eq!(state(&mut w, "a"), ShipState::Docked);
    }

    #[test]
    fn npc_orders_run_out_pay_partially_and_refill_slowly() {
        let mut w = world();
        graduate(&mut w, "a");
        w.systems
            .get_mut("kesh")
            .unwrap()
            .orders
            .insert(FERRITE.to_string(), 10.0);
        let ch = w.characters.get_mut("a").unwrap();
        let ship = ch.ship.as_mut().unwrap();
        ship.hold.insert(FERRITE.to_string(), 30);
        ship.activity = Activity::Docked;
        ship.grid = None;
        let before = snap(&mut w, "a").credits;

        ok(&mut w, "a", "sell ferrite all");
        assert_eq!(snap(&mut w, "a").credits, before + 90);
        assert_eq!(snap(&mut w, "a").ship.unwrap().hold_used_m3, 20);
        refused(&mut w, "a", "sell ferrite all", Refusal::NoDemand);
        refused(&mut w, "a", "sell ferrite nonsense", Refusal::BadArgument);
        refused(&mut w, "a", "sell ferrite 0", Refusal::BadArgument);
        refused(&mut w, "a", "sell siltane all", Refusal::NoDemand);

        // 1,250 m³ an hour is about 0.35 m³ a second.
        for _ in 0..6 {
            w.tick();
        }
        ok(&mut w, "a", "sell ferrite 1");
        assert_eq!(snap(&mut w, "a").ship.unwrap().hold_used_m3, 19);
    }

    #[test]
    fn laser_ii_needs_its_skills_and_price_and_raises_yield() {
        let mut w = world();
        graduate(&mut w, "a");
        let ch = w.characters.get_mut("a").unwrap();
        let ship = ch.ship.as_mut().unwrap();
        ship.activity = Activity::Docked;
        ship.grid = None;
        ch.credits = 1_000_000;
        let outcome = w.execute("a", "buy module mining-laser-2");
        assert_eq!(outcome.refusal, Refusal::MissingSkill);
        assert!(
            outcome.message.contains("Mining V") && outcome.message.contains("Laser Tuning V"),
            "{}",
            outcome.message
        );
        assert_eq!(
            snap(&mut w, "a").credits,
            1_000_000,
            "a refusal costs nothing"
        );

        let ch = w.characters.get_mut("a").unwrap();
        ch.skills.insert("mining".into(), 5);
        ch.skills.insert("laser-tuning".into(), 5);
        ch.credits = 999_999;
        refused(
            &mut w,
            "a",
            "buy module mining-laser-2",
            Refusal::InsufficientCredits,
        );
        w.characters.get_mut("a").unwrap().credits = 1_000_000;
        ok(&mut w, "a", "buy module mining-laser-2");
        let s = snap(&mut w, "a");
        assert_eq!(s.credits, 0);
        assert_eq!(s.ship.unwrap().laser_yield_m3, 35);
        refused(
            &mut w,
            "a",
            "buy module mining-laser-2",
            Refusal::AlreadyOwned,
        );
        refused(&mut w, "a", "buy module mining-laser-1", Refusal::NotFound);
    }

    #[test]
    fn rocks_deplete_and_regrow_in_shared_space() {
        let mut w = world();
        graduate(&mut w, "a");
        ok(&mut w, "a", "warp belt-1");
        run_until(&mut w, "a", ShipState::Idle, 60);
        approach_and_mine(&mut w, "a");
        let rock_id = snap(&mut w, "a").ship.unwrap().target;
        let start = snap(&mut w, "a")
            .contacts
            .iter()
            .find(|c| c.id == rock_id)
            .unwrap()
            .volume_left_m3;
        for _ in 0..30 {
            w.tick();
        }
        let after = snap(&mut w, "a")
            .contacts
            .iter()
            .find(|c| c.id == rock_id)
            .unwrap()
            .volume_left_m3;
        assert!(after < start, "shared rocks lose what is mined");
    }

    #[test]
    fn the_tutorial_tracks_the_core_loop_in_order() {
        let mut w = world();
        w.login("a");
        let step = |w: &mut World| snap(w, "a").tutorial_step();
        assert_eq!(step(&mut w), TutorialStep::BuyShip);
        ok(&mut w, "a", "buy ship sparrow");
        assert_eq!(step(&mut w), TutorialStep::Undock);
        ok(&mut w, "a", "undock");
        run_until(&mut w, "a", ShipState::Idle, 20);
        assert_eq!(step(&mut w), TutorialStep::WarpToBelt);
        ok(&mut w, "a", "warp belt-1");
        run_until(&mut w, "a", ShipState::Idle, 60);
        assert_eq!(step(&mut w), TutorialStep::Approach);
        approach_and_mine(&mut w, "a");
        assert_eq!(step(&mut w), TutorialStep::Mine);
        run_until(&mut w, "a", ShipState::Idle, 700);
        assert_eq!(step(&mut w), TutorialStep::WarpToStation);
        ok(&mut w, "a", "warp station");
        run_until(&mut w, "a", ShipState::Idle, 60);
        assert_eq!(step(&mut w), TutorialStep::Dock);
        ok(&mut w, "a", "dock");
        run_until(&mut w, "a", ShipState::Docked, 10);
        assert_eq!(step(&mut w), TutorialStep::Sell);
        ok(&mut w, "a", "sell ferrite all");
        assert_eq!(step(&mut w), TutorialStep::Exit);
    }

    #[test]
    fn stations_without_the_service_refuse_it() {
        let (game, mut content) = config::parse(
            include_str!("../../config/game.toml"),
            include_str!("../../config/world.toml"),
        )
        .unwrap();
        let pocket = content
            .systems
            .iter_mut()
            .find(|s| s.id == "tutorial")
            .unwrap();
        pocket.station.services.clear();
        let mut w = World::new(game, content);
        w.login("a");
        refused(&mut w, "a", "buy ship sparrow", Refusal::ServiceUnavailable);
        refused(
            &mut w,
            "a",
            "buy module mining-laser-2",
            Refusal::ServiceUnavailable,
        );
    }

    #[test]
    fn commands_are_case_insensitive_and_long_lines_are_refused() {
        let mut w = world();
        w.login("a");
        ok(&mut w, "a", "BUY SHIP Sparrow");
        let long = "x".repeat(MAX_COMMAND_BYTES + 1);
        refused(&mut w, "a", &long, Refusal::BadArgument);
        refused(&mut w, "a", "   ", Refusal::UnknownCommand);
    }

    fn config_pair() -> (GameConfig, WorldConfig) {
        config::parse(
            include_str!("../../config/game.toml"),
            include_str!("../../config/world.toml"),
        )
        .unwrap()
    }

    /// A busy world: one player mid-mining in the pocket, one graduated to Kesh.
    fn busy_world() -> World {
        let mut w = world();
        graduate(&mut w, "b");
        w.login("a");
        to_belt(&mut w, "a");
        approach_and_mine(&mut w, "a");
        for _ in 0..40 {
            w.tick();
        }
        w.characters
            .get_mut("b")
            .unwrap()
            .skills
            .insert("mining".into(), 2);
        w
    }

    #[test]
    fn a_restored_world_matches_the_saved_one() {
        let mut w = busy_world();
        // Restoring marks everyone logged out now, so do the same to compare.
        w.logout("a");
        w.logout("b");
        let saved = w.export();
        let (game, content) = config_pair();
        let restored = World::restore(game, content, saved.clone()).unwrap();
        assert_eq!(restored.export(), saved);
    }

    #[test]
    fn a_restored_world_keeps_playing() {
        let w = busy_world();
        let before_ids = w.next_id;
        let (game, content) = config_pair();
        let mut w = World::restore(game, content, w.export()).unwrap();

        w.login("a");
        let s = snap(&mut w, "a");
        assert_eq!(s.system, "Training Pocket");
        assert_eq!(s.ship.as_ref().unwrap().state(), ShipState::Mining);
        assert_eq!(
            s.ship.as_ref().unwrap().hold_used_m3,
            25,
            "one cycle was paid before the save"
        );
        assert_eq!(s.tutorial_step(), TutorialStep::Mine);
        run_until(&mut w, "a", ShipState::Idle, 700);
        assert!(w.next_id >= before_ids, "ids never repeat after a restart");
        ok(&mut w, "a", "warp station");

        w.login("b");
        assert_eq!(snap(&mut w, "b").system, "Kesh");
        assert_eq!(w.characters["b"].skills["mining"], 2);
    }

    #[test]
    fn ships_in_space_get_the_logout_grace_after_a_restart() {
        let w = busy_world();
        let (game, content) = config_pair();
        let mut w = World::restore(game, content, w.export()).unwrap();
        for _ in 0..60 {
            w.tick();
        }
        assert!(w.characters.values().all(|c| {
            c.ship
                .as_ref()
                .is_some_and(|s| s.activity == Activity::Stowed)
        }));
    }

    #[test]
    fn restoring_fails_loudly_when_content_is_gone() {
        let w = busy_world();
        let saved = w.export();
        let (game, mut content) = config_pair();
        content.systems.retain(|s| s.id != "tutorial");
        content.tutorial_system = "kesh".into();
        let error = World::restore(game, content, saved).err().unwrap();
        assert!(error.to_string().contains("unknown template"), "{error:#}");

        let (mut game, content) = config_pair();
        game.ships.clear();
        let error = World::restore(game, content, w.export()).err().unwrap();
        assert!(error.to_string().contains("unknown ship"), "{error:#}");
    }
}
