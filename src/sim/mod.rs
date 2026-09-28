//! The authoritative game simulation.
//!
//! Rules for everything in here (they are what make multiplayer and
//! replays possible later):
//! - Runs only in `FixedUpdate`, at a fixed tick rate, never per frame.
//! - Only changes state in response to ticks and `SimCommand`s. Player
//!   input, the Termbot commander and (later) the network all push
//!   commands into the same `CommandQueue`.
//! - All randomness comes from `SimRng`, seeded per match, and is used in
//!   a fixed order (units sorted by `UnitId`).
//! - Rendering reads sim state but never writes it.

pub mod ai;
pub mod classes;
pub mod combat;
pub mod economy;
pub mod fighter;
pub mod gadgets;
#[cfg(test)]
mod gadget_tests;
pub mod perception;
pub mod platoon;
pub mod vehicle;

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::faction::Faction;
use crate::map::{GameMap, TilePos};
use crate::pathfinding::find_path_avoiding;
use classes::FighterClass;
use combat::{MatchOutcome, ShotLog};
use platoon::{Mission, Platoon, PlatoonId};

/// Simulation ticks per second.
pub const SIM_HZ: f64 = 20.0;
/// Seconds per tick. The sim always uses this, never wall-clock time.
pub const DT: f32 = 1.0 / SIM_HZ as f32;

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(SIM_HZ))
            .init_resource::<ShotLog>()
            .init_resource::<platoon::SignalsInFlight>()
            .init_resource::<gadgets::KnownTraps>()
            .init_resource::<gadgets::AirSupport>()
            .init_resource::<gadgets::PendingExplosions>()
            .init_resource::<gadgets::BlastLog>()
            .init_resource::<fighter::XpLog>()
            .configure_sets(
                FixedUpdate,
                (
                    SimSet::Commands,
                    SimSet::Platoons,
                    SimSet::Perception,
                    SimSet::Decide,
                    SimSet::Movement,
                    SimSet::Combat,
                    SimSet::Aftermath,
                    SimSet::Vehicles,
                    SimSet::Intel,
                    SimSet::Territory,
                    SimSet::Record,
                )
                    .chain()
                    .run_if(resource_exists::<GameMap>)
                    .run_if(economy::in_battle)
                    .run_if(match_running),
            )
            .add_systems(
                FixedUpdate,
                economy::process_setup_commands.run_if(resource_exists::<GameMap>).run_if(economy::in_setup),
            )
            .add_systems(
                FixedUpdate,
                (
                    (
                        advance_tick,
                        economy::accrue_supply,
                        process_commands,
                        gadgets::process_gadget_commands,
                        platoon::deliver_signals,
                        economy::process_deployments,
                    )
                        .chain()
                        .in_set(SimSet::Commands),
                    platoon::update_platoons.in_set(SimSet::Platoons),
                    (perception::update_perception, gadgets::detect_gadgets).chain().in_set(SimSet::Perception),
                    (ai::decide, gadgets::update_engineers).chain().in_set(SimSet::Decide),
                    move_units.in_set(SimSet::Movement),
                    (
                        gadgets::resolve_strikes,
                        gadgets::trigger_traps,
                        combat::fire_weapons,
                        gadgets::fire_turrets,
                        gadgets::fire_artillery,
                        gadgets::use_abilities,
                    )
                        .chain()
                        .in_set(SimSet::Combat),
                    (
                        gadgets::resolve_explosions,
                        fighter::apply_experience,
                        gadgets::update_smoke,
                        combat::update_conditions,
                        combat::remove_dead,
                    )
                        .chain()
                        .in_set(SimSet::Aftermath),
                    vehicle::update_vehicles.in_set(SimSet::Vehicles),
                    (economy::update_territory, economy::check_timer).chain().in_set(SimSet::Territory),
                    record_interpolation.in_set(SimSet::Record),
                ),
            )
            .add_systems(Update, apply_interpolation);
    }
}

fn match_running(outcome: Option<Res<MatchOutcome>>) -> bool {
    outcome.is_none_or(|o| !o.ended)
}

/// Stages of one simulation tick, in order.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Apply queued commands (player signals, AI commander decisions).
    Commands,
    /// Platoons: leaders, confusion, delayed orders, assault bounds.
    Platoons,
    /// Each unit works out what it can see.
    Perception,
    /// Each fighter decides what to do (autonomy).
    Decide,
    /// Move units along their paths.
    Movement,
    /// Fighters shoot at what they see.
    Combat,
    /// Morale, suppression recovery, deaths.
    Aftermath,
    /// Vehicles that arrived (or were destroyed) unload their platoon.
    Vehicles,
    /// Update each commander's knowledge (fog) from perception.
    Intel,
    /// Sector control and the match timer.
    Territory,
    /// Snapshot positions for smooth rendering between ticks.
    Record,
}

/// The match's random number generator. All sim randomness (trait rolls,
/// hit chances, decisions) must come from here so matches are reproducible.
#[derive(Resource)]
pub struct SimRng(pub ChaCha8Rng);

/// Number of simulation ticks since the match started.
#[derive(Resource, Default)]
pub struct SimTick(pub u64);

/// Stable unit identifier. Unlike `Entity`, this is the same on every
/// machine, so it's what commands and the network refer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId(pub u32);

/// Lookup from stable ids to local ECS entities.
#[derive(Resource, Default)]
pub struct UnitIndex {
    pub entities: HashMap<UnitId, Entity>,
    pub platoons: HashMap<PlatoonId, Entity>,
    next_id: u32,
    next_platoon: u32,
    platoon_numbers: [u32; 2],
}

impl UnitIndex {
    pub fn allocate(&mut self) -> UnitId {
        let id = UnitId(self.next_id);
        self.next_id += 1;
        id
    }

    /// Per-side platoon counter, for call signs (Alpha, Bravo, ...).
    pub fn next_platoon_number(&mut self, faction: Faction) -> u32 {
        let n = self.platoon_numbers[faction.index()];
        self.platoon_numbers[faction.index()] += 1;
        n
    }

    pub fn allocate_platoon(&mut self) -> PlatoonId {
        let id = PlatoonId(self.next_platoon);
        self.next_platoon += 1;
        id
    }
}

/// Everything that can change the game goes through a command.
#[derive(Clone, Debug)]
pub enum SimCommand {
    /// Move a single unit (the King) to a tile.
    Move { issuer: Faction, unit: UnitId, target: TilePos },
    /// The King signals a platoon: a mission and where to carry it out.
    Signal { issuer: Faction, platoon: PlatoonId, mission: Mission, target: TilePos },
    /// Setup only: buy a platoon (1-5 fighters) and place it at `anchor`,
    /// inside your own half.
    PlacePlatoon { issuer: Faction, classes: Vec<FighterClass>, anchor: TilePos },
    /// Setup only: take back the last placed platoon (refunded).
    UndoPlacement { issuer: Faction },
    /// Setup only: done placing. The battle starts when both sides are ready.
    Ready { issuer: Faction },
    /// Battle: buy a platoon (1-5 fighters) and send it to an explored
    /// tile, on foot from base or by vehicle (costs more).
    Deploy { issuer: Faction, classes: Vec<FighterClass>, vehicle: bool, target: TilePos },
    /// Setup only: place a gadget (mine, bomb, turret, artillery) in your
    /// half. Paid for like a platoon; undo takes it back.
    PlaceGadget { issuer: Faction, kind: gadgets::GadgetKind, at: TilePos },
    /// Battle: signal a platoon's engineer to build a gadget at an explored
    /// tile. Paid when sent; refunded if the build is abandoned.
    Build { issuer: Faction, platoon: PlatoonId, kind: gadgets::GadgetKind, target: TilePos },
    /// Battle: call an air strike on an explored tile (costly, long
    /// cooldown, 4 s warning).
    AirStrike { issuer: Faction, target: TilePos },
    /// Battle: order an artillery piece to shell a tile next.
    ArtilleryFire { issuer: Faction, unit: UnitId, target: TilePos },
}

/// Commands waiting for the next tick. Commands issued while the game is
/// paused wait here and apply on resume.
#[derive(Resource, Default)]
pub struct CommandQueue {
    pending: Vec<SimCommand>,
}

impl CommandQueue {
    pub fn push(&mut self, command: SimCommand) {
        self.pending.push(command);
    }

    pub fn take_all(&mut self) -> Vec<SimCommand> {
        std::mem::take(&mut self.pending)
    }

    /// Remove and return the commands matching `f`, keeping the rest.
    pub fn take_matching(&mut self, f: impl Fn(&SimCommand) -> bool) -> Vec<SimCommand> {
        self.pending.extract_if(.., |c| f(c)).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitClass {
    King,
    Fighter(FighterClass),
    /// A troop transport on its way to a drop-off.
    Vehicle,
    /// A mine, bomb, turret or artillery piece built by engineers.
    Gadget(gadgets::GadgetKind),
}

impl UnitClass {
    pub fn name(self) -> &'static str {
        match self {
            UnitClass::King => "King",
            UnitClass::Fighter(class) => class.stats().name,
            UnitClass::Vehicle => "Troop vehicle",
            UnitClass::Gadget(kind) => kind.stats().name,
        }
    }

    pub fn letter(self) -> &'static str {
        match self {
            UnitClass::King => "K",
            UnitClass::Fighter(class) => class.stats().letter,
            UnitClass::Vehicle => "V",
            UnitClass::Gadget(kind) => kind.stats().letter,
        }
    }
}

#[derive(Component, Debug)]
pub struct Unit {
    pub id: UnitId,
    pub class: UnitClass,
}

/// Tile-to-tile movement. A unit occupies its `TilePos` until it fully
/// reaches the next tile on its path.
#[derive(Component, Debug)]
pub struct Mover {
    /// Remaining tiles to walk, next tile first.
    pub path: VecDeque<TilePos>,
    /// Progress toward `path[0]`, from 0 to 1.
    pub progress: f32,
    /// Tiles per second on open ground.
    pub speed: f32,
    /// Situational multiplier (sprinting, crawling while suppressed, ...).
    pub speed_mult: f32,
}

impl Mover {
    pub fn new(speed: f32) -> Self {
        Self { path: VecDeque::new(), progress: 0.0, speed, speed_mult: 1.0 }
    }

    /// Stop after finishing the current step (so movement stays smooth).
    pub fn stop(&mut self) {
        if self.progress > 0.0 {
            self.path.truncate(1);
        } else {
            self.path.clear();
        }
    }
}

/// Plan a path to `goal`. If the unit is mid-step, it finishes that step
/// first and plans from the tile it's stepping into.
pub fn set_destination(map: &GameMap, pos: TilePos, mover: &mut Mover, goal: TilePos) -> bool {
    set_destination_avoiding(map, pos, mover, goal, &std::collections::HashSet::new())
}

/// Like `set_destination`, stepping around the `avoid` tiles (known traps).
pub fn set_destination_avoiding(
    map: &GameMap,
    pos: TilePos,
    mover: &mut Mover,
    goal: TilePos,
    avoid: &std::collections::HashSet<TilePos>,
) -> bool {
    let mid_step = mover.progress > 0.0 && !mover.path.is_empty();
    let start = if mid_step { mover.path[0] } else { pos };
    match find_path_avoiding(map, start, goal, avoid) {
        Some(mut path) => {
            if mid_step {
                path.push_front(start);
            }
            mover.path = path;
            true
        }
        None => false,
    }
}

/// World position at the previous and current tick, for smooth rendering.
#[derive(Component, Debug)]
pub struct SimInterp {
    pub previous: Vec2,
    pub current: Vec2,
}

impl SimInterp {
    pub fn at(pos: Vec2) -> Self {
        Self { previous: pos, current: pos }
    }
}

fn advance_tick(mut tick: ResMut<SimTick>) {
    tick.0 += 1;
}

pub fn process_commands(
    mut queue: ResMut<CommandQueue>,
    tick: Res<SimTick>,
    index: Res<UnitIndex>,
    map: Res<GameMap>,
    mut units: Query<(&Faction, &TilePos, &mut Mover), With<Unit>>,
    platoons: Query<(Entity, &Faction), With<Platoon>>,
    kings: Query<(&Unit, &Faction, &TilePos)>,
    mut flight: ResMut<platoon::SignalsInFlight>,
) {
    for command in queue.take_matching(|c| matches!(c, SimCommand::Move { .. } | SimCommand::Signal { .. })) {
        match command {
            SimCommand::Move { issuer, unit, target } => {
                let Some(&entity) = index.entities.get(&unit) else { continue };
                let Ok((faction, pos, mut mover)) = units.get_mut(entity) else { continue };
                if *faction != issuer {
                    warn!("{issuer:?} tried to command a {faction:?} unit; ignored");
                    continue;
                }
                if !set_destination(&map, *pos, &mut mover, target) {
                    debug!("no path from {pos:?} to {target:?}");
                }
            }
            SimCommand::Signal { issuer, platoon, mission, target } => {
                let Some(&entity) = index.platoons.get(&platoon) else { continue };
                let Ok((_, faction)) = platoons.get(entity) else { continue };
                if *faction != issuer {
                    warn!("{issuer:?} tried to signal a {faction:?} platoon; ignored");
                    continue;
                }
                // The signal travels to the platoon; it's received (and
                // adds confusion) when it arrives.
                let from = kings
                    .iter()
                    .find(|(u, f, _)| u.class == UnitClass::King && **f == issuer)
                    .map_or(target, |(_, _, p)| *p);
                flight.0.push(platoon::SignalInFlight {
                    issuer,
                    platoon: entity,
                    mission,
                    target,
                    from,
                    sent_tick: tick.0,
                    arrive_tick: tick.0 + platoon::SIGNAL_TRAVEL_TICKS,
                    build: None,
                });
            }
            _ => {}
        }
    }
}

fn move_units(map: Res<GameMap>, mut units: Query<(&mut TilePos, &mut Mover)>) {
    for (mut pos, mut mover) in &mut units {
        let Some(&next) = mover.path.front() else {
            mover.progress = 0.0;
            continue;
        };
        if !map.can_step(*pos, next) {
            // Path went stale; stop and let the unit re-plan.
            mover.path.clear();
            mover.progress = 0.0;
            continue;
        }
        let diagonal = next.x != pos.x && next.y != pos.y;
        let step_length = if diagonal { std::f32::consts::SQRT_2 } else { 1.0 };
        let cost = map.tile(next).move_cost() as f32 * step_length;
        mover.progress += mover.speed * mover.speed_mult * DT / cost;
        if mover.progress >= 1.0 {
            *pos = next;
            mover.path.pop_front();
            mover.progress = if mover.path.is_empty() { 0.0 } else { mover.progress - 1.0 };
        }
    }
}

fn record_interpolation(mut units: Query<(&TilePos, &Mover, &mut SimInterp)>) {
    for (pos, mover, mut interp) in &mut units {
        let here = pos.to_world();
        let current = match mover.path.front() {
            Some(next) => here.lerp(next.to_world(), mover.progress.min(1.0)),
            None => here,
        };
        interp.previous = interp.current;
        interp.current = current;
    }
}

/// Render units between sim ticks, so 20 Hz movement looks smooth at any
/// frame rate.
fn apply_interpolation(fixed: Res<Time<Fixed>>, mut units: Query<(&SimInterp, &mut Transform)>) {
    let t = fixed.overstep_fraction();
    for (interp, mut transform) in &mut units {
        let p = interp.previous.lerp(interp.current, t);
        transform.translation.x = p.x;
        transform.translation.y = p.y;
    }
}
