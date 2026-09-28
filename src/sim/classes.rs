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
    /// Builds mines, bombs, turrets and artillery; finds and defuses the
    /// enemy's hidden traps.
    Engineer,
}

/// What a weapon does best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DamageType {
    /// Plain bullets.
    Standard,
    /// Strong against light armor, weak against heavy.
    Piercing,
    /// Strong against heavy armor; blasts ignore cover and break walls.
    Explosive,
    /// Weaker hits, but pins targets down with heavy suppression.
    Suppression,
}

impl DamageType {
    pub fn name(self) -> &'static str {
        match self {
            DamageType::Standard => "Standard",
            DamageType::Piercing => "Piercing",
            DamageType::Explosive => "Explosive",
            DamageType::Suppression => "Suppression",
        }
    }
}

/// How well protected a unit's body is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArmorClass {
    Light,
    Medium,
    Heavy,
}

impl ArmorClass {
    pub fn name(self) -> &'static str {
        match self {
            ArmorClass::Light => "Light",
            ArmorClass::Medium => "Medium",
            ArmorClass::Heavy => "Heavy",
        }
    }
}

/// Damage multiplier for a damage type against an armor class.
pub fn damage_multiplier(damage: DamageType, armor: ArmorClass) -> f32 {
    use ArmorClass::*;
    use DamageType::*;
    match (damage, armor) {
        (Standard, _) => 1.0,
        (Piercing, Light) => 1.3,
        (Piercing, Medium) => 1.0,
        (Piercing, Heavy) => 0.8,
        (Explosive, Light) => 0.9,
        (Explosive, Medium) => 1.2,
        (Explosive, Heavy) => 1.5,
        (Suppression, _) => 0.8,
    }
}

/// Extra suppression a damage type puts on targets.
pub fn suppression_multiplier(damage: DamageType) -> f32 {
    match damage {
        DamageType::Suppression => 1.6,
        DamageType::Explosive => 1.5,
        _ => 1.0,
    }
}

/// A fighter's hidden equipment, chosen per class in the armory before
/// the match. The enemy only learns it when it's used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Loadout {
    /// Assaulter: nothing special.
    Rifle,
    /// Assaulter: throws a grenade (Explosive, ignores cover) at dug-in
    /// enemies within 5 tiles, every 20 s.
    Grenades,
    /// Assaulter: patches up itself or a hurt neighbor (+35 health), twice.
    Medkit,
    /// Sniper: armor-piercing rounds (Piercing damage).
    PiercingRounds,
    /// Sniper: pops smoke (blocks sight, radius 2, 8 s) when pinned in the
    /// open, every 40 s.
    SmokeKit,
    /// Heavy Weapons: machine gun (Suppression damage).
    MachineGun,
    /// Heavy Weapons: fires rockets (Explosive) at armored targets and
    /// structures, every 6 s; the machine gun fires slower.
    Rockets,
    /// Juggernaut: shotgun (Standard).
    Shotgun,
    /// Juggernaut: blows up the wall or rock an enemy is sheltering behind,
    /// every 25 s.
    BreachingCharge,
    /// Engineer: builds and defuses 60% faster.
    Toolkit,
    /// Engineer: carries 2 mines and plants them on its own while holding.
    MineLayer,
}

impl Loadout {
    pub fn name(self) -> &'static str {
        match self {
            Loadout::Rifle => "Rifle",
            Loadout::Grenades => "Grenades",
            Loadout::Medkit => "Medkit",
            Loadout::PiercingRounds => "Piercing rounds",
            Loadout::SmokeKit => "Smoke kit",
            Loadout::MachineGun => "Machine gun",
            Loadout::Rockets => "Rockets",
            Loadout::Shotgun => "Shotgun",
            Loadout::BreachingCharge => "Breaching charge",
            Loadout::Toolkit => "Toolkit",
            Loadout::MineLayer => "Mine layer",
        }
    }

    /// The variants a class can roll.
    pub fn options(class: FighterClass) -> &'static [Loadout] {
        match class {
            FighterClass::Assaulter => &[Loadout::Rifle, Loadout::Grenades, Loadout::Medkit],
            FighterClass::Sniper => &[Loadout::PiercingRounds, Loadout::SmokeKit],
            FighterClass::HeavyWeapons => &[Loadout::MachineGun, Loadout::Rockets],
            FighterClass::Juggernaut => &[Loadout::Shotgun, Loadout::BreachingCharge],
            FighterClass::Engineer => &[Loadout::Toolkit, Loadout::MineLayer],
        }
    }

    pub fn roll(class: FighterClass, rng: &mut impl Rng) -> Loadout {
        let options = Loadout::options(class);
        options[rng.random_range(0..options.len())]
    }
}

/// A fighter's weapon damage type, which can depend on its loadout.
pub fn weapon_damage_type(class: FighterClass, loadout: Loadout) -> DamageType {
    match (class, loadout) {
        (FighterClass::Sniper, Loadout::PiercingRounds) => DamageType::Piercing,
        (FighterClass::HeavyWeapons, Loadout::MachineGun) => DamageType::Suppression,
        _ => DamageType::Standard,
    }
}

/// The equipment component: which loadout, and which sides know about it.
#[derive(bevy::prelude::Component, Clone, Copy, Debug)]
pub struct Equipment {
    pub loadout: Loadout,
    /// The class upgrade from the armory (hidden from the enemy).
    pub modifier: super::armory::Modifier,
    /// Per faction index: has that side seen it used?
    pub known_to: [bool; 2],
    /// Seconds until the special ability can be used again.
    pub cooldown: f32,
    /// Uses left, for limited abilities (medkit, mines).
    pub charges: u8,
}

impl Equipment {
    pub fn new(loadout: Loadout, modifier: super::armory::Modifier, owner_index: usize) -> Self {
        let mut known_to = [false; 2];
        known_to[owner_index] = true;
        let charges = match loadout {
            Loadout::Medkit | Loadout::MineLayer => 2,
            _ => 0,
        };
        Self { loadout, modifier, known_to, cooldown: 0.0, charges }
    }
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
    pub armor_class: ArmorClass,
    pub traits: TraitRanges,
}

impl FighterClass {
    /// Position in `ALL`.
    pub fn index(self) -> usize {
        match self {
            FighterClass::Assaulter => 0,
            FighterClass::Sniper => 1,
            FighterClass::HeavyWeapons => 2,
            FighterClass::Juggernaut => 3,
            FighterClass::Engineer => 4,
        }
    }

    pub const ALL: [FighterClass; 5] = [
        FighterClass::Assaulter,
        FighterClass::Sniper,
        FighterClass::HeavyWeapons,
        FighterClass::Juggernaut,
        FighterClass::Engineer,
    ];

    pub fn stats(self) -> &'static ClassStats {
        match self {
            FighterClass::Assaulter => &ASSAULTER,
            FighterClass::Sniper => &SNIPER,
            FighterClass::HeavyWeapons => &HEAVY_WEAPONS,
            FighterClass::Juggernaut => &JUGGERNAUT,
            FighterClass::Engineer => &ENGINEER,
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
    armor_class: ArmorClass::Light,
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
    armor_class: ArmorClass::Light,
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
    armor_class: ArmorClass::Medium,
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
    armor_class: ArmorClass::Heavy,
    traits: TraitRanges {
        accuracy: (35, 70),
        iq: (20, 55),
        tactics: (20, 60),
        bravery: (70, 100),
        leadership: (30, 70),
    },
};

const ENGINEER: ClassStats = ClassStats {
    name: "Engineer",
    letter: "E",
    max_health: 90.0,
    armor: 0.0,
    speed: 2.8,
    vision: 11.0,
    range: 6.0,
    damage: 10.0,
    fire_interval: 1.2,
    base_accuracy: 0.5,
    moving_accuracy: 0.6,
    suppression: 5.0,
    armor_class: ArmorClass::Light,
    traits: TraitRanges {
        accuracy: (30, 60),
        iq: (55, 90),
        tactics: (45, 85),
        bravery: (30, 70),
        leadership: (20, 60),
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
    fn damage_types_beat_the_right_armor() {
        use ArmorClass::*;
        use DamageType::*;
        assert!(damage_multiplier(Piercing, Light) > damage_multiplier(Piercing, Heavy));
        assert!(damage_multiplier(Explosive, Heavy) > damage_multiplier(Explosive, Light));
        assert!(damage_multiplier(Explosive, Heavy) > damage_multiplier(Standard, Heavy));
        assert!(suppression_multiplier(Suppression) > suppression_multiplier(Standard));
    }

    #[test]
    fn loadouts_roll_from_class_options() {
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        for class in FighterClass::ALL {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..60 {
                let l = Loadout::roll(class, &mut rng);
                assert!(Loadout::options(class).contains(&l));
                seen.insert(l);
            }
            assert_eq!(seen.len(), Loadout::options(class).len(), "{class:?} rolls every option");
        }
    }

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
