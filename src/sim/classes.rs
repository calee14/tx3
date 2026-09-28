//! Fighter classes: stats and trait ranges.
//!
//! Classes are plain data. Everything a class does comes from these
//! numbers plus the shared systems (movement, AI, combat), so a new class
//! is a new table entry. (This table can move to RON data files later
//! without changing any systems.)

use rand::Rng;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FighterClass {
    /// Pawn: cheap line infantry that takes and holds ground.
    Assaulter,
    /// Bishop: long sight lanes, deadly from cover, can't shoot on the move.
    Sniper,
    /// Rook: suppression across a lane.
    HeavyWeapons,
    /// Knight: armored breacher, short range, hard to scare.
    Juggernaut,
}

/// Inclusive ranges that traits are rolled in, 0-100.
#[derive(Clone, Copy, Debug)]
pub struct TraitRanges {
    pub accuracy: (u8, u8),
    pub iq: (u8, u8),
    pub tactics: (u8, u8),
    pub bravery: (u8, u8),
    pub leadership: (u8, u8),
}

#[derive(Clone, Copy, Debug)]
pub struct ClassStats {
    pub name: &'static str,
    /// Letter drawn on the placeholder sprite.
    pub letter: &'static str,
    pub max_health: f32,
    /// Fraction of incoming damage ignored (0 = none, 0.4 = 40% less).
    pub armor: f32,
    /// Tiles per second on open ground.
    pub speed: f32,
    /// Sight radius in tiles.
    pub vision: f32,
    /// Weapon range in tiles.
    pub range: f32,
    pub damage: f32,
    /// Seconds between shots.
    pub fire_interval: f32,
    /// Hit chance at close range against an exposed, standing target,
    /// before traits and conditions.
    pub base_accuracy: f32,
    /// Accuracy multiplier when firing while moving. 0 = can't fire on
    /// the move at all.
    pub moving_accuracy: f32,
    /// Suppression each shot puts on its target (more if it hits).
    pub suppression: f32,
    pub traits: TraitRanges,
}

impl FighterClass {
    #[allow(dead_code)] // used by tests; handy for UI later
    pub const ALL: [FighterClass; 4] =
        [FighterClass::Assaulter, FighterClass::Sniper, FighterClass::HeavyWeapons, FighterClass::Juggernaut];

    pub fn stats(self) -> &'static ClassStats {
        match self {
            FighterClass::Assaulter => &ASSAULTER,
            FighterClass::Sniper => &SNIPER,
            FighterClass::HeavyWeapons => &HEAVY_WEAPONS,
            FighterClass::Juggernaut => &JUGGERNAUT,
        }
    }
}

const ASSAULTER: ClassStats = ClassStats {
    name: "Assaulter",
    letter: "A",
    max_health: 100.0,
    armor: 0.0,
    speed: 3.0,
    vision: 10.0,
    range: 7.0,
    damage: 14.0,
    fire_interval: 1.1,
    base_accuracy: 0.55,
    moving_accuracy: 0.6,
    suppression: 6.0,
    traits: TraitRanges {
        accuracy: (35, 75),
        iq: (30, 70),
        tactics: (30, 70),
        bravery: (40, 80),
        leadership: (20, 70),
    },
};

const SNIPER: ClassStats = ClassStats {
    name: "Sniper",
    letter: "S",
    max_health: 70.0,
    armor: 0.0,
    speed: 2.6,
    vision: 14.0,
    range: 15.0,
    damage: 55.0,
    fire_interval: 3.2,
    base_accuracy: 0.8,
    moving_accuracy: 0.0,
    suppression: 10.0,
    traits: TraitRanges {
        accuracy: (65, 95),
        iq: (50, 90),
        tactics: (50, 90),
        bravery: (30, 70),
        leadership: (10, 50),
    },
};

const HEAVY_WEAPONS: ClassStats = ClassStats {
    name: "Heavy Weapons",
    letter: "H",
    max_health: 130.0,
    armor: 0.1,
    speed: 2.0,
    vision: 10.0,
    range: 10.0,
    damage: 7.0,
    fire_interval: 0.3,
    base_accuracy: 0.35,
    moving_accuracy: 0.3,
    suppression: 14.0,
    traits: TraitRanges {
        accuracy: (30, 65),
        iq: (30, 65),
        tactics: (30, 70),
        bravery: (50, 90),
        leadership: (20, 60),
    },
};

const JUGGERNAUT: ClassStats = ClassStats {
    name: "Juggernaut",
    letter: "J",
    max_health: 240.0,
    armor: 0.4,
    speed: 2.2,
    vision: 8.0,
    range: 4.0,
    damage: 22.0,
    fire_interval: 1.3,
    base_accuracy: 0.6,
    moving_accuracy: 0.8,
    suppression: 8.0,
    traits: TraitRanges {
        accuracy: (35, 70),
        iq: (20, 55),
        tactics: (20, 60),
        bravery: (70, 100),
        leadership: (30, 70),
    },
};

/// A fighter's rolled traits, 0-100. Fixed for life (veterancy will
/// improve them later).
#[derive(bevy::prelude::Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traits {
    pub accuracy: u8,
    pub iq: u8,
    pub tactics: u8,
    pub bravery: u8,
    pub leadership: u8,
}

impl Traits {
    /// Roll traits within a class's ranges, like Pokemon IVs.
    pub fn roll(ranges: &TraitRanges, rng: &mut impl Rng) -> Self {
        let mut roll = |(lo, hi): (u8, u8)| rng.random_range(lo..=hi);
        Self {
            accuracy: roll(ranges.accuracy),
            iq: roll(ranges.iq),
            tactics: roll(ranges.tactics),
            bravery: roll(ranges.bravery),
            leadership: roll(ranges.leadership),
        }
    }

    /// 0-1 versions, for formulas.
    pub fn acc(&self) -> f32 {
        self.accuracy as f32 / 100.0
    }
    pub fn iq(&self) -> f32 {
        self.iq as f32 / 100.0
    }
    pub fn tac(&self) -> f32 {
        self.tactics as f32 / 100.0
    }
    pub fn brav(&self) -> f32 {
        self.bravery as f32 / 100.0
    }
    pub fn lead(&self) -> f32 {
        self.leadership as f32 / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn traits_roll_within_class_ranges() {
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        for class in FighterClass::ALL {
            let r = class.stats().traits;
            for _ in 0..200 {
                let t = Traits::roll(&r, &mut rng);
                assert!((r.accuracy.0..=r.accuracy.1).contains(&t.accuracy));
                assert!((r.iq.0..=r.iq.1).contains(&t.iq));
                assert!((r.tactics.0..=r.tactics.1).contains(&t.tactics));
                assert!((r.bravery.0..=r.bravery.1).contains(&t.bravery));
                assert!((r.leadership.0..=r.leadership.1).contains(&t.leadership));
            }
        }
    }
}
