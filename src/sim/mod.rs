//! The authoritative game simulation.
//!
//! Rules for everything in here (they are what make multiplayer and
//! replays possible later):
//! - Runs only in `FixedUpdate`, at a fixed tick rate, never per frame.
//! - Only changes state in response to ticks and `SimCommand`s. Player
//!   input, the Termbot AI and (later) the network all push commands into
//!   the same `CommandQueue`.
//! - All randomness comes from `SimRng`, seeded per match.
//! - Rendering reads sim state but never writes it.

pub mod perception;

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::faction::Faction;
use crate::map::{GameMap, TilePos};
use crate::pathfinding::find_path;

/// Simulation ticks per second.
pub const SIM_HZ: f64 = 20.0;

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(SIM_HZ))
            .configure_sets(
                FixedUpdate,
                (SimSet::Commands, SimSet::Movement, SimSet::Perception, SimSet::Intel, SimSet::Record)
                    .chain()
                    .run_if(resource_exists::<GameMap>),
            )
            .add_systems(
                FixedUpdate,
                (
                    (advance_tick, process_commands).chain().in_set(SimSet::Commands),
                    move_units.in_set(SimSet::Movement),
                    perception::update_perception.in_set(SimSet::Perception),
                    record_interpolation.in_set(SimSet::Record),
                ),
            )
            .add_systems(Update, apply_interpolation);
    }
}

/// Stages of one simulation tick, in order.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Apply queued commands (player signals, AI decisions).
    Commands,
    /// Move units along their paths.
    Movement,
    /// Each unit works out what it can see.
    Perception,
    /// Update each commander's knowledge (sector fog) from perception.
    Intel,
    /// Snapshot positions for smooth rendering between ticks.
    Record,
}

/// The match's random number generator. All sim randomness (trait rolls,
/// hit chances, ...) must come from here so matches are reproducible.
#[derive(Resource)]
#[allow(dead_code)] // first used by trait rolls in step 2
pub struct SimRng(pub ChaCha8Rng);

/// Number of simulation ticks since the match started.
#[derive(Resource, Default)]
pub struct SimTick(pub u64);

/// Stable unit identifier. Unlike `Entity`, this is the same on every
/// machine, so it's what commands and the network refer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId(pub u32);

/// Lookup from `UnitId` to the local ECS entity.
#[derive(Resource, Default)]
pub struct UnitIndex {
    pub entities: HashMap<UnitId, Entity>,
    next_id: u32,
}

impl UnitIndex {
    pub fn allocate(&mut self) -> UnitId {
        let id = UnitId(self.next_id);
        self.next_id += 1;
        id
    }
}

/// Everything that can change the game goes through a command.
#[derive(Clone, Debug)]
pub enum SimCommand {
    /// Move a unit (or later, a platoon leader) to a tile.
    Move { issuer: Faction, unit: UnitId, target: TilePos },
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitClass {
    King,
}

impl UnitClass {
    pub fn name(self) -> &'static str {
        match self {
            UnitClass::King => "King",
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
}

impl Mover {
    pub fn new(speed: f32) -> Self {
        Self { path: VecDeque::new(), progress: 0.0, speed }
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

fn process_commands(
    mut queue: ResMut<CommandQueue>,
    index: Res<UnitIndex>,
    map: Res<GameMap>,
    mut units: Query<(&Faction, &TilePos, &mut Mover)>,
) {
    for command in queue.pending.drain(..) {
        match command {
            SimCommand::Move { issuer, unit, target } => {
                let Some(&entity) = index.entities.get(&unit) else { continue };
                let Ok((faction, pos, mut mover)) = units.get_mut(entity) else { continue };
                if *faction != issuer {
                    warn!("{issuer:?} tried to command a {faction:?} unit; ignored");
                    continue;
                }
                // If the unit is mid-step, finish that step first so its
                // movement stays smooth, and plan from the tile it's
                // stepping into.
                let mid_step = mover.progress > 0.0 && !mover.path.is_empty();
                let start = if mid_step { mover.path[0] } else { *pos };
                match find_path(&map, start, target) {
                    Some(mut path) => {
                        if mid_step {
                            path.push_front(start);
                        }
                        mover.path = path;
                    }
                    None => debug!("no path from {start:?} to {target:?}"),
                }
            }
        }
    }
}

fn move_units(time: Res<Time>, map: Res<GameMap>, mut units: Query<(&mut TilePos, &mut Mover)>) {
    let dt = time.delta_secs();
    for (mut pos, mut mover) in &mut units {
        let Some(&next) = mover.path.front() else {
            mover.progress = 0.0;
            continue;
        };
        let diagonal = next.x != pos.x && next.y != pos.y;
        let step_length = if diagonal { std::f32::consts::SQRT_2 } else { 1.0 };
        let cost = map.tile(next).move_cost() as f32 * step_length;
        mover.progress += mover.speed * dt / cost;
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
