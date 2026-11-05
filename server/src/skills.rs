//! Skill rules, as groundwork. Characters store skill levels and modules can
//! require them, but there is no training yet. When training arrives it must run
//! on wall-clock time, so it keeps going while the player is offline and the
//! server is down: that needs persistence first.

use std::collections::BTreeMap;

use crate::config::{GameConfig, Training};

pub const MAX_LEVEL: u8 = 5;

/// Levels a character has, by skill id. A missing skill is level 0.
pub type Levels = BTreeMap<String, u8>;

/// Skill points to go from `level - 1` to `level`. Each level costs about 5.7×
/// the one before, so level V is most of the work.
pub fn points_for_level(training: &Training, rank: u32, level: u8) -> u64 {
    debug_assert!((1..=MAX_LEVEL).contains(&level));
    let scale = training.level_multiplier.powi(i32::from(level) - 1);
    (training.base_points * f64::from(rank) * scale).round() as u64
}

/// Requirements a character does not yet meet, as (skill id, level needed).
pub fn missing(have: &Levels, requires: &Levels) -> Vec<(String, u8)> {
    requires
        .iter()
        .filter(|(skill, level)| have.get(*skill).copied().unwrap_or(0) < **level)
        .map(|(skill, level)| (skill.clone(), *level))
        .collect()
}

/// Every level needed to meet `requires`, including each skill's own
/// prerequisites, keeping the highest level asked for per skill.
pub fn closure(cfg: &GameConfig, requires: &Levels) -> Levels {
    let mut needed = Levels::new();
    let mut pending: Vec<(String, u8)> = requires.iter().map(|(s, l)| (s.clone(), *l)).collect();
    while let Some((skill, level)) = pending.pop() {
        let current = needed.entry(skill.clone()).or_insert(0);
        if *current >= level {
            continue;
        }
        *current = level;
        if let Some(def) = cfg.skills.get(&skill) {
            pending.extend(def.requires.iter().map(|(s, l)| (s.clone(), *l)));
        }
    }
    needed
}

/// Hours of training to meet `requires` from `have`, training one skill at a time.
pub fn hours_to_meet(cfg: &GameConfig, have: &Levels, requires: &Levels) -> f64 {
    let mut points = 0;
    for (skill, target) in closure(cfg, requires) {
        let rank = cfg.skills[&skill].rank;
        let from = have.get(&skill).copied().unwrap_or(0);
        for level in (from + 1)..=target {
            points += points_for_level(&cfg.training, rank, level);
        }
    }
    points as f64 / cfg.training.points_per_hour
}

/// "Mining V, Laser Tuning V", for refusal messages.
pub fn describe(cfg: &GameConfig, levels: &[(String, u8)]) -> String {
    const ROMAN: [&str; 6] = ["0", "I", "II", "III", "IV", "V"];
    levels
        .iter()
        .map(|(skill, level)| {
            let name = cfg
                .skills
                .get(skill)
                .map_or(skill.as_str(), |s| s.name.as_str());
            format!("{name} {}", ROMAN[usize::from(*level).min(5)])
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn cfg() -> GameConfig {
        config::parse(
            include_str!("../../config/game.toml"),
            include_str!("../../config/world.toml"),
        )
        .unwrap()
        .0
    }

    #[test]
    fn levels_follow_the_curve() {
        let cfg = cfg();
        let points: Vec<u64> = (1..=5)
            .map(|l| points_for_level(&cfg.training, 1, l))
            .collect();
        assert_eq!(points, [250, 1414, 8000, 45255, 256000]);
        assert_eq!(points_for_level(&cfg.training, 4, 5), 1_024_000);
    }

    #[test]
    fn mining_laser_ii_takes_a_dedicated_miner_about_a_month() {
        let cfg = cfg();
        let requires = &cfg.lasers["mining-laser-2"].requires;
        let days = hours_to_meet(&cfg, &Levels::new(), requires) / 24.0;
        assert!((25.0..=40.0).contains(&days), "{days:.1} days");
    }

    #[test]
    fn prerequisites_are_included_once_at_the_highest_level() {
        let cfg = cfg();
        let needed = closure(&cfg, &cfg.lasers["mining-laser-2"].requires);
        assert_eq!(
            needed,
            Levels::from([("laser-tuning".into(), 5), ("mining".into(), 5)])
        );
    }

    #[test]
    fn missing_lists_only_unmet_requirements() {
        let have = Levels::from([("mining".into(), 5), ("laser-tuning".into(), 2)]);
        let requires = Levels::from([("mining".into(), 5), ("laser-tuning".into(), 5)]);
        assert_eq!(missing(&have, &requires), [("laser-tuning".to_string(), 5)]);
        assert_eq!(
            describe(&cfg(), &missing(&have, &requires)),
            "Laser Tuning V"
        );
    }
}
