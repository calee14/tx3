//! The armory: what each side brings to a match, chosen in the menu
//! before it starts.
//!
//! - **Fighters:** for each class, one loadout and one modifier. Every
//!   fighter of that class comes with them. The first loadout of each class
//!   is free; the others, and every modifier, add to that fighter's price.
//! - **Machine weaponry:** up to `MACHINE_SLOTS` of mines, bombs, turrets,
//!   artillery and air strikes, each in one of two variants. Only what a
//!   side brought can be placed in setup, built by its engineers, or called
//!   in.
//!
//! Both are hidden from the enemy, who has to work out what you brought
//! from what they see in battle.

use bevy::prelude::*;
use rand::Rng;

use super::classes::{FighterClass, Loadout};
use super::gadgets::GadgetKind;

/// How many kinds of machine weaponry a side can bring.
pub const MACHINE_SLOTS: usize = 3;

/// A fighter upgrade, bought for a whole class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Modifier {
    None,
    /// +25% health, 10% slower.
    BodyArmor,
    /// +15 Accuracy.
    Marksman,
    /// 15% faster, +1 vision.
    LightKit,
}

impl Modifier {
    pub const ALL: [Modifier; 4] = [Modifier::None, Modifier::BodyArmor, Modifier::Marksman, Modifier::LightKit];

    pub fn name(self) -> &'static str {
        match self {
            Modifier::None => "None",
            Modifier::BodyArmor => "Body armor",
            Modifier::Marksman => "Marksman training",
            Modifier::LightKit => "Light kit",
        }
    }

    pub fn cost(self) -> f32 {
        match self {
            Modifier::None => 0.0,
            Modifier::BodyArmor => 20.0,
            Modifier::Marksman => 15.0,
            Modifier::LightKit => 10.0,
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Modifier::None => "No upgrade.",
            Modifier::BodyArmor => "+25% health, but 10% slower.",
            Modifier::Marksman => "+15 Accuracy.",
            Modifier::LightKit => "Moves 15% faster and sees 1 tile further.",
        }
    }

    pub fn health_mult(self) -> f32 {
        if self == Modifier::BodyArmor { 1.25 } else { 1.0 }
    }

    pub fn speed_mult(self) -> f32 {
        match self {
            Modifier::BodyArmor => 0.9,
            Modifier::LightKit => 1.15,
            _ => 1.0,
        }
    }

    pub fn vision_bonus(self) -> f32 {
        if self == Modifier::LightKit { 1.0 } else { 0.0 }
    }

    pub fn accuracy_bonus(self) -> u8 {
        if self == Modifier::Marksman { 15 } else { 0 }
    }
}

/// Extra price of a loadout on top of the fighter (the first option of
/// each class is free).
pub fn loadout_cost(loadout: Loadout) -> f32 {
    match loadout {
        Loadout::Rifle | Loadout::PiercingRounds | Loadout::MachineGun | Loadout::Shotgun | Loadout::Toolkit => 0.0,
        Loadout::Medkit | Loadout::SmokeKit => 10.0,
        Loadout::Grenades => 15.0,
        Loadout::Rockets | Loadout::BreachingCharge | Loadout::MineLayer => 20.0,
    }
}

pub fn loadout_blurb(loadout: Loadout) -> &'static str {
    match loadout {
        Loadout::Rifle => "Standard rifle. Tells the enemy nothing.",
        Loadout::Grenades => "Throws a grenade at a dug-in enemy within 5 tiles, every 20 s.",
        Loadout::Medkit => "Heals itself or a hurt neighbor (+35 health), twice.",
        Loadout::PiercingRounds => "Piercing damage: +30% against light armor, -20% against heavy.",
        Loadout::SmokeKit => "Pops smoke (blocks sight, 8 s) when pinned in the open, every 40 s.",
        Loadout::MachineGun => "Suppression damage: weaker hits, far more suppression.",
        Loadout::Rockets => "Rockets at armor, gadgets and people behind walls, every 6 s. Gun fires slower.",
        Loadout::Shotgun => "Standard shotgun. Tells the enemy nothing.",
        Loadout::BreachingCharge => "Blows up the wall or rock an enemy hides behind, every 25 s.",
        Loadout::Toolkit => "Builds and defuses 60% faster.",
        Loadout::MineLayer => "Carries 2 mines and plants them on its own ahead of a holding platoon.",
    }
}

/// Machine weaponry that can be brought to a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Machine {
    Mine,
    Bomb,
    Turret,
    Artillery,
    AirStrike,
}

impl Machine {
    pub const ALL: [Machine; 5] = [Machine::Mine, Machine::Bomb, Machine::Turret, Machine::Artillery, Machine::AirStrike];

    pub fn name(self) -> &'static str {
        match self {
            Machine::Mine => "Mines",
            Machine::Bomb => "Bombs",
            Machine::Turret => "Turrets",
            Machine::Artillery => "Artillery",
            Machine::AirStrike => "Air strikes",
        }
    }

    /// The gadget it builds (air strikes are called in, not built).
    pub fn gadget(self) -> Option<GadgetKind> {
        match self {
            Machine::Mine => Some(GadgetKind::Mine),
            Machine::Bomb => Some(GadgetKind::Bomb),
            Machine::Turret => Some(GadgetKind::Turret),
            Machine::Artillery => Some(GadgetKind::Artillery),
            Machine::AirStrike => None,
        }
    }

    pub fn of_gadget(kind: GadgetKind) -> Machine {
        match kind {
            GadgetKind::Mine => Machine::Mine,
            GadgetKind::Bomb => Machine::Bomb,
            GadgetKind::Turret => Machine::Turret,
            GadgetKind::Artillery => Machine::Artillery,
        }
    }

    /// Name of a variant (`alt` = the second one).
    pub fn variant_name(self, alt: bool) -> &'static str {
        match (self, alt) {
            (Machine::Mine, false) => "Contact mine",
            (Machine::Mine, true) => "Shrapnel mine",
            (Machine::Bomb, false) => "Proximity bomb",
            (Machine::Bomb, true) => "Heavy bomb",
            (Machine::Turret, false) => "Machine-gun turret",
            (Machine::Turret, true) => "Cannon turret",
            (Machine::Artillery, false) => "Field gun",
            (Machine::Artillery, true) => "Heavy howitzer",
            (Machine::AirStrike, false) => "Carpet bombing",
            (Machine::AirStrike, true) => "Precision strike",
        }
    }

    pub fn variant_blurb(self, alt: bool) -> &'static str {
        match (self, alt) {
            (Machine::Mine, false) => "Explodes under the first enemy to step on it: 70 damage, radius 1.5.",
            (Machine::Mine, true) => "Weaker blast (55) over a wider area (radius 2.5).",
            (Machine::Bomb, false) => "Enemy within 2 tiles starts a 0.5 s fuse: 90 damage, radius 3.",
            (Machine::Bomb, true) => "Bigger blast: 110 damage, radius 4.",
            (Machine::Turret, false) => "Fast automatic gun: 16 damage every 0.6 s, range 11. Never scared.",
            (Machine::Turret, true) => "Explosive shell every 2 s: 40 damage, range 13. Cracks armor.",
            (Machine::Artillery, false) => {
                "Fires on its own at enemies near your troops. Range 60, every 25 s, 65 damage, radius 2.5. Inaccurate."
            }
            (Machine::Artillery, true) => "Bigger, slower shells: 85 damage, radius 3.5, every 40 s. Even less accurate.",
            (Machine::AirStrike, false) => "You call it: 5 bombs in a line (55 each), 4 s warning, 90 s cooldown.",
            (Machine::AirStrike, true) => "You call it: one big bomb (110, radius 2.2), 3 s warning, 90 s cooldown.",
        }
    }

    /// Price of one (built, placed or called).
    pub fn price(self, alt: bool) -> f32 {
        match self.gadget() {
            Some(kind) => kind.stats().cost + if alt { gadget_variant_extra(kind) } else { 0.0 },
            None => super::gadgets::air_strike_cost(alt),
        }
    }
}

/// Extra price of a gadget's second variant.
pub fn gadget_variant_extra(kind: GadgetKind) -> f32 {
    match kind {
        GadgetKind::Mine => 10.0,
        GadgetKind::Bomb => 30.0,
        GadgetKind::Turret => 40.0,
        GadgetKind::Artillery => 60.0,
    }
}

/// What every fighter of one class comes with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kit {
    pub loadout: Loadout,
    pub modifier: Modifier,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Armory {
    /// One kit per class, in `FighterClass::ALL` order.
    pub kits: [Kit; 5],
    /// Machine weaponry brought, each with its variant (`true` = the
    /// second one).
    pub machines: Vec<(Machine, bool)>,
}

impl Default for Armory {
    /// Free loadouts, no modifiers; mines, turrets and air strikes.
    fn default() -> Self {
        Self {
            kits: FighterClass::ALL.map(|c| Kit { loadout: Loadout::options(c)[0], modifier: Modifier::None }),
            machines: vec![(Machine::Mine, false), (Machine::Turret, false), (Machine::AirStrike, false)],
        }
    }
}

impl Armory {
    /// Everything, standard variants (for tests and sandboxes).
    #[allow(dead_code)]
    pub fn everything() -> Self {
        Self { machines: Machine::ALL.iter().map(|m| (*m, false)).collect(), ..Default::default() }
    }

    pub fn kit(&self, class: FighterClass) -> Kit {
        self.kits[class.index()]
    }

    pub fn kit_mut(&mut self, class: FighterClass) -> &mut Kit {
        &mut self.kits[class.index()]
    }

    /// Price added to each fighter of this class by its kit.
    pub fn surcharge(&self, class: FighterClass) -> f32 {
        let kit = self.kit(class);
        loadout_cost(kit.loadout) + kit.modifier.cost()
    }

    /// The variant brought, if this machine was brought at all.
    pub fn variant(&self, machine: Machine) -> Option<bool> {
        self.machines.iter().find(|(m, _)| *m == machine).map(|(_, alt)| *alt)
    }

    pub fn brings(&self, machine: Machine) -> bool {
        self.variant(machine).is_some()
    }

    /// The variant of a gadget, if brought.
    pub fn gadget_variant(&self, kind: GadgetKind) -> Option<bool> {
        self.variant(Machine::of_gadget(kind))
    }

    /// Price of one gadget of this kind as brought (None if not brought).
    pub fn gadget_price(&self, kind: GadgetKind) -> Option<f32> {
        self.gadget_variant(kind).map(|alt| Machine::of_gadget(kind).price(alt))
    }

    /// Bring or leave a machine (up to `MACHINE_SLOTS`). Returns false if
    /// the slots are full.
    pub fn toggle(&mut self, machine: Machine) -> bool {
        if let Some(i) = self.machines.iter().position(|(m, _)| *m == machine) {
            self.machines.remove(i);
            true
        } else if self.machines.len() < MACHINE_SLOTS {
            self.machines.push((machine, false));
            true
        } else {
            false
        }
    }

    pub fn set_variant(&mut self, machine: Machine, alt: bool) {
        for m in &mut self.machines {
            if m.0 == machine {
                m.1 = alt;
            }
        }
    }

    /// A random armory for an AI commander, flavored by how it plays.
    pub fn random(machines: Vec<(Machine, bool)>, favored: &[(FighterClass, Modifier)], rng: &mut impl Rng) -> Self {
        let mut armory = Armory { machines, ..Default::default() };
        for class in FighterClass::ALL {
            armory.kit_mut(class).loadout = Loadout::roll(class, rng);
        }
        for &(class, modifier) in favored {
            armory.kit_mut(class).modifier = modifier;
        }
        armory
    }
}

/// Each side's armory for this match (by `Faction::index()`).
#[derive(Resource, Clone, Debug, Default)]
pub struct Armories(pub [Armory; 2]);

impl Armories {
    pub fn of(&self, faction: crate::faction::Faction) -> &Armory {
        &self.0[faction.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_slots_are_limited() {
        let mut a = Armory::default();
        assert_eq!(a.machines.len(), MACHINE_SLOTS);
        assert!(!a.toggle(Machine::Bomb), "slots full");
        assert!(a.toggle(Machine::Mine), "leave mines");
        assert!(a.toggle(Machine::Bomb));
        a.set_variant(Machine::Bomb, true);
        assert_eq!(a.gadget_price(GadgetKind::Bomb), Some(100.0));
        assert_eq!(a.gadget_price(GadgetKind::Mine), None);
    }

    #[test]
    fn kits_add_to_the_price() {
        let mut a = Armory::default();
        assert_eq!(a.surcharge(FighterClass::Assaulter), 0.0);
        *a.kit_mut(FighterClass::Assaulter) = Kit { loadout: Loadout::Grenades, modifier: Modifier::BodyArmor };
        assert_eq!(a.surcharge(FighterClass::Assaulter), 35.0);
    }
}
