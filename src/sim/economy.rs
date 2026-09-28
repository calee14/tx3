//! Supply, buying fighters, the setup phase, reinforcements, territory
//! and the match timer.
//!
//! - Both sides earn Supply at the same steady rate. Territory gives no
//!   income, so neither side can snowball economically.
//! - Each fighter costs its class's base price, raised 25% for every
//!   living fighter of that class the side already has (including ones
//!   riding in vehicles), so no one can field only snipers.
//! - Setup phase: each side spends its starting Supply placing platoons in
//!   its own half, hidden from the enemy. The battle starts when both are
//!   ready.
//! - During battle, new platoons (1-5 fighters) either walk from base
//!   (then reposition to their target), or ride a vehicle there for an
//!   extra cost (see `vehicle.rs`).
//! - The match lasts 15 minutes. If no King has died by then, the side
//!   holding the most sectors wins.

use std::collections::HashSet;

use bevy::prelude::*;

use super::classes::FighterClass;
use super::combat::MatchOutcome;
use super::fighter::Fighter;
use super::platoon::{MAX_PLATOON_SIZE, Mission, NewPlatoon, Platoon, nearest_free_tile, roll_recruits, spawn_platoon};
use super::vehicle::{Vehicle, spawn_vehicle};
use super::{CommandQueue, DT, SIM_HZ, SimCommand, SimRng, SimTick, Unit, UnitIndex};
use crate::faction::Faction;
use crate::fog::FogOfWar;
use crate::map::{GameMap, SECTOR_COUNT, TilePos, sector_index};
use crate::pathfinding::find_path;

pub const STARTING_SUPPLY: f32 = 1000.0;
pub const SUPPLY_PER_SECOND: f32 = 3.0;
/// Each living fighter of a class raises that class's price by this much.
pub const ESCALATION: f32 = 0.25;
/// Extra cost to deliver a platoon by vehicle.
pub const VEHICLE_COST: f32 = 150.0;
pub const MATCH_SECONDS: u64 = 15 * 60;
pub const MATCH_TICKS: u64 = MATCH_SECONDS * SIM_HZ as u64;
/// Territory gives a small morale boost: this much per sector held beyond
/// half the map, up to `MAX_TERRITORY_MORALE`.
pub const TERRITORY_MORALE_PER_SECTOR: f32 = 2.0;
pub const MAX_TERRITORY_MORALE: f32 = 8.0;

pub fn base_cost(class: FighterClass) -> f32 {
    match class {
        FighterClass::Assaulter => 50.0,
        FighterClass::Sniper => 90.0,
        FighterClass::HeavyWeapons => 80.0,
        FighterClass::Juggernaut => 120.0,
    }
}

fn class_index(class: FighterClass) -> usize {
    match class {
        FighterClass::Assaulter => 0,
        FighterClass::Sniper => 1,
        FighterClass::HeavyWeapons => 2,
        FighterClass::Juggernaut => 3,
    }
}

/// How many fighters of each class a side currently has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassCounts([u32; 4]);

impl ClassCounts {
    pub fn get(&self, class: FighterClass) -> u32 {
        self.0[class_index(class)]
    }
    pub fn add(&mut self, class: FighterClass) {
        self.0[class_index(class)] += 1;
    }
}

/// Price of the next fighter of `class`, given what the side already has.
pub fn fighter_cost(class: FighterClass, counts: &ClassCounts) -> f32 {
    base_cost(class) * (1.0 + ESCALATION * counts.get(class) as f32)
}

/// Total price of a new platoon (each fighter raises the price of the next
/// one of its class), plus the vehicle if it rides in.
pub fn platoon_cost(classes: &[FighterClass], counts: &ClassCounts, vehicle: bool) -> f32 {
    let mut counts = *counts;
    let mut total = 0.0;
    for &class in classes {
        total += fighter_cost(class, &counts);
        counts.add(class);
    }
    total + if vehicle { VEHICLE_COST } else { 0.0 }
}

/// Count a side's fighters, including those riding in vehicles.
pub fn count_classes<'a>(
    faction: Faction,
    fighters: impl Iterator<Item = (&'a Faction, &'a Fighter)>,
    vehicles: impl Iterator<Item = (&'a Faction, &'a Vehicle)>,
) -> ClassCounts {
    let mut counts = ClassCounts::default();
    for (f, fighter) in fighters {
        if *f == faction {
            counts.add(fighter.class);
        }
    }
    for (f, vehicle) in vehicles {
        if *f == faction {
            for r in &vehicle.cargo {
                counts.add(r.class);
            }
        }
    }
    counts
}

#[derive(Resource, Debug)]
pub struct Supply(pub [f32; 2]);

impl Default for Supply {
    fn default() -> Self {
        Self([STARTING_SUPPLY; 2])
    }
}

impl Supply {
    pub fn of(&self, faction: Faction) -> f32 {
        self.0[faction.index()]
    }
}

/// Setup (placing armies) or battle.
#[derive(Resource, Debug, Default)]
pub struct MatchPhase {
    pub battle: bool,
    pub ready: [bool; 2],
}

pub fn in_setup(phase: Option<Res<MatchPhase>>) -> bool {
    phase.is_some_and(|p| !p.battle)
}

pub fn in_battle(phase: Option<Res<MatchPhase>>) -> bool {
    phase.is_some_and(|p| p.battle)
}

/// Platoons placed during setup, in order, so the last can be undone.
#[derive(Resource, Debug, Default)]
pub struct SetupLog(pub [Vec<Entity>; 2]);

/// Who controls each sector. A sector changes hands when one side has
/// fighters in it and the other has none.
#[derive(Resource, Debug)]
pub struct Territory {
    pub owner: [Option<Faction>; SECTOR_COUNT],
}

impl Territory {
    /// Each side starts owning its own half.
    pub fn new(map: &GameMap) -> Self {
        let mut owner = [None; SECTOR_COUNT];
        for s in &map.sectors {
            owner[s.id] = Some(s.home_of);
        }
        Self { owner }
    }

    pub fn held(&self, faction: Faction) -> usize {
        self.owner.iter().filter(|o| **o == Some(faction)).count()
    }

    /// Morale bonus for holding more than half the map.
    pub fn morale_bonus(&self, faction: Faction) -> f32 {
        let extra = self.held(faction).saturating_sub(SECTOR_COUNT / 2) as f32;
        (extra * TERRITORY_MORALE_PER_SECTOR).min(MAX_TERRITORY_MORALE)
    }
}

/// Where a side's on-foot reinforcements (and vehicles) start.
pub fn base_entry(map: &GameMap, faction: Faction) -> TilePos {
    let spawn = map.spawn(faction);
    let forward = match faction {
        Faction::Humans => spawn.offset(0, 4),
        Faction::Termbots => spawn.offset(0, -4),
    };
    nearest_free_tile(map, &HashSet::new(), forward).unwrap_or(spawn)
}

fn valid_platoon(classes: &[FighterClass]) -> bool {
    (1..=MAX_PLATOON_SIZE).contains(&classes.len())
}

/// Setup phase: place platoons, undo, and ready up.
#[allow(clippy::too_many_arguments)]
pub fn process_setup_commands(
    mut commands: Commands,
    mut queue: ResMut<CommandQueue>,
    mut phase: ResMut<MatchPhase>,
    mut supply: ResMut<Supply>,
    mut index: ResMut<UnitIndex>,
    mut log: ResMut<SetupLog>,
    mut rng: ResMut<SimRng>,
    map: Res<GameMap>,
    positions: Query<&TilePos, With<Unit>>,
    fighters: Query<(&Faction, &Fighter)>,
    vehicles: Query<(&Faction, &Vehicle)>,
    platoons: Query<&Platoon>,
    units: Query<&Unit>,
) {
    let mut taken: HashSet<TilePos> = positions.iter().copied().collect();
    let mut placed_now = [ClassCounts::default(); 2];

    for command in queue.take_all() {
        match command {
            SimCommand::PlacePlatoon { issuer, classes, anchor } => {
                let i = issuer.index();
                if phase.ready[i] || !valid_platoon(&classes) {
                    continue;
                }
                if !anchor.in_bounds() || !map.in_home_half(issuer, anchor) {
                    debug!("{issuer:?} tried to place outside its half");
                    continue;
                }
                // Clicked on a rock or wall: use the nearest open tile.
                let Some(anchor) =
                    super::platoon::nearest_free_tile_where(&map, &HashSet::new(), anchor, |p| map.in_home_half(issuer, p))
                else {
                    continue;
                };
                let mut counts = count_classes(issuer, fighters.iter(), vehicles.iter());
                for class in FighterClass::ALL {
                    for _ in 0..placed_now[i].get(class) {
                        counts.add(class);
                    }
                }
                let cost = platoon_cost(&classes, &counts, false);
                if cost > supply.0[i] {
                    continue;
                }
                supply.0[i] -= cost;
                for &c in &classes {
                    placed_now[i].add(c);
                }
                let recruits = roll_recruits(&classes, &mut rng.0);
                let (platoon, _) = spawn_platoon(
                    &mut commands,
                    &mut index,
                    &map,
                    &mut taken,
                    NewPlatoon {
                        faction: issuer,
                        recruits: &recruits,
                        anchor,
                        mission: Mission::Overwatch,
                        target: anchor,
                        paid: cost,
                    },
                    |p| map.in_home_half(issuer, p),
                );
                log.0[i].push(platoon);
            }
            SimCommand::UndoPlacement { issuer } => {
                let i = issuer.index();
                if phase.ready[i] {
                    continue;
                }
                let Some(entity) = log.0[i].pop() else { continue };
                if let Ok(platoon) = platoons.get(entity) {
                    supply.0[i] += platoon.paid;
                    for &m in &platoon.members {
                        if let Ok(unit) = units.get(m) {
                            index.entities.remove(&unit.id);
                        }
                        commands.entity(m).despawn();
                    }
                    index.platoons.remove(&platoon.id);
                }
                commands.entity(entity).despawn();
            }
            SimCommand::Ready { issuer } => {
                phase.ready[issuer.index()] = true;
            }
            other => debug!("ignored during setup: {other:?}"),
        }
    }

    if phase.ready.iter().all(|&r| r) {
        phase.battle = true;
        info!("Battle begins");
    }
}

/// Battle phase: buy and deploy reinforcements.
#[allow(clippy::too_many_arguments)]
pub fn process_deployments(
    mut commands: Commands,
    mut queue: ResMut<CommandQueue>,
    mut supply: ResMut<Supply>,
    mut index: ResMut<UnitIndex>,
    mut rng: ResMut<SimRng>,
    map: Res<GameMap>,
    fog: Res<FogOfWar>,
    positions: Query<&TilePos, With<Unit>>,
    fighters: Query<(&Faction, &Fighter)>,
    vehicles: Query<(&Faction, &Vehicle)>,
) {
    let mut taken: HashSet<TilePos> = positions.iter().copied().collect();
    for command in queue.take_matching(|c| matches!(c, SimCommand::Deploy { .. })) {
        let SimCommand::Deploy { issuer, classes, vehicle, target } = command else { continue };
        let i = issuer.index();
        if !valid_platoon(&classes)
            || !target.in_bounds()
            || !map.tile(target).passable()
            || !fog.is_explored(issuer, target)
        {
            debug!("{issuer:?} deployment rejected: bad target {target:?}");
            continue;
        }
        let counts = count_classes(issuer, fighters.iter(), vehicles.iter());
        let cost = platoon_cost(&classes, &counts, vehicle);
        if cost > supply.0[i] {
            continue;
        }
        let entry = base_entry(&map, issuer);
        let Some(path) = find_path(&map, entry, target) else { continue };
        supply.0[i] -= cost;
        let recruits = roll_recruits(&classes, &mut rng.0);

        if vehicle {
            spawn_vehicle(&mut commands, &mut index, issuer, entry, recruits, path, cost);
        } else {
            // On foot: they appear at base and reposition to the target.
            spawn_platoon(
                &mut commands,
                &mut index,
                &map,
                &mut taken,
                NewPlatoon { faction: issuer, recruits: &recruits, anchor: entry, mission: Mission::Reposition, target, paid: cost },
                |_| true,
            );
        }
    }
}

pub fn accrue_supply(mut supply: ResMut<Supply>) {
    for s in &mut supply.0 {
        *s += SUPPLY_PER_SECOND * DT;
    }
}

/// Once a second: sectors change hands where only one side has fighters.
pub fn update_territory(
    tick: Res<SimTick>,
    mut territory: ResMut<Territory>,
    fighters: Query<(&Faction, &TilePos), With<Fighter>>,
) {
    if tick.0 % SIM_HZ as u64 != 0 {
        return;
    }
    let mut presence = [[0u32; 2]; SECTOR_COUNT];
    for (faction, pos) in &fighters {
        presence[sector_index(*pos)][faction.index()] += 1;
    }
    for (s, counts) in presence.iter().enumerate() {
        let owner = match (counts[0] > 0, counts[1] > 0) {
            (true, false) => Some(Faction::Humans),
            (false, true) => Some(Faction::Termbots),
            _ => continue,
        };
        if territory.owner[s] != owner {
            territory.owner[s] = owner;
        }
    }
}

/// Time's up: the side with more sectors wins; equal is a draw.
pub fn check_timer(tick: Res<SimTick>, territory: Res<Territory>, mut outcome: ResMut<MatchOutcome>) {
    if tick.0 < MATCH_TICKS || outcome.ended {
        return;
    }
    let h = territory.held(Faction::Humans);
    let t = territory.held(Faction::Termbots);
    outcome.ended = true;
    outcome.winner = match h.cmp(&t) {
        std::cmp::Ordering::Greater => Some(Faction::Humans),
        std::cmp::Ordering::Less => Some(Faction::Termbots),
        std::cmp::Ordering::Equal => None,
    };
    outcome.reason = format!("Time's up: sectors held {h} to {t}");
}

/// Seconds of battle left.
pub fn seconds_left(tick: u64) -> u64 {
    MATCH_TICKS.saturating_sub(tick) / SIM_HZ as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_escalate_per_class() {
        let empty = ClassCounts::default();
        assert_eq!(platoon_cost(&[FighterClass::Assaulter], &empty, false), 50.0);
        // 50 + 62.5 + 75
        assert_eq!(platoon_cost(&[FighterClass::Assaulter; 3], &empty, false), 187.5);
        // Other classes don't raise each other's price.
        assert_eq!(platoon_cost(&[FighterClass::Assaulter, FighterClass::Sniper], &empty, false), 140.0);
        let mut two_snipers = ClassCounts::default();
        two_snipers.add(FighterClass::Sniper);
        two_snipers.add(FighterClass::Sniper);
        assert_eq!(fighter_cost(FighterClass::Sniper, &two_snipers), 135.0);
        assert_eq!(platoon_cost(&[FighterClass::Assaulter], &empty, true), 50.0 + VEHICLE_COST);
    }
}
