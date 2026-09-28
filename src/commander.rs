//! Placeholder Termbot commander, and the default army.
//!
//! Until the real Termbot AI arrives (build step 7), the Termbots play a
//! fixed script so there's something to fight:
//! - Setup: place the default army (mirrored) and ready up.
//! - Opening: two platoons assault the Humans' front, one overwatches the
//!   middle.
//! - Every ~75 s: buy a reinforcement platoon, alternating on foot and by
//!   vehicle, and send it to assault once it arrives.
//!
//! Like any player, it only acts by pushing commands into the queue.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::faction::Faction;
use crate::map::TilePos;
use crate::sim::classes::FighterClass::{self, *};
use crate::sim::economy::{MatchPhase, Supply, count_classes, in_battle, in_setup, platoon_cost};
use crate::sim::fighter::Fighter;
use crate::sim::platoon::{Mission, Platoon, PlatoonId};
use crate::sim::vehicle::Vehicle;
use crate::sim::{CommandQueue, SimCommand, SimSet, SimTick};

/// The standard starting army (costs 990 of the 1000 starting Supply),
/// as Humans' anchors; Termbots use the mirrored tiles.
pub const DEFAULT_ARMY: [(&[FighterClass], TilePos); 3] = [
    (&[Assaulter, Assaulter, Assaulter, HeavyWeapons], TilePos::new(52, 17)),
    (&[Sniper, Sniper, Assaulter], TilePos::new(63, 20)),
    (&[Juggernaut, Assaulter, Assaulter, HeavyWeapons], TilePos::new(74, 17)),
];

/// Commands that place the default army for a side.
pub fn default_army_commands(faction: Faction) -> Vec<SimCommand> {
    DEFAULT_ARMY
        .iter()
        .map(|(classes, anchor)| SimCommand::PlacePlatoon {
            issuer: faction,
            classes: classes.to_vec(),
            anchor: match faction {
                Faction::Humans => *anchor,
                Faction::Termbots => anchor.mirrored(),
            },
        })
        .collect()
}

const OPENING_TICK: u64 = 40;
const FIRST_REINFORCEMENT_TICK: u64 = 60 * 20;
const REINFORCEMENT_INTERVAL: u64 = 75 * 20;
const REINFORCEMENTS: [&[FighterClass]; 4] = [
    &[Assaulter, Assaulter, HeavyWeapons],
    &[Juggernaut, Assaulter, Assaulter],
    &[Assaulter, Sniper],
    &[Assaulter, Assaulter, Assaulter, HeavyWeapons, Assaulter],
];

#[derive(Resource, Default)]
pub struct CommanderState {
    placed: bool,
    deployed: u32,
    /// Platoons the commander has already given their orders.
    ordered: HashSet<PlatoonId>,
}

pub struct CommanderPlugin;

impl Plugin for CommanderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                termbot_setup
                    .run_if(resource_exists::<CommanderState>)
                    .run_if(in_setup)
                    .before(crate::sim::economy::process_setup_commands),
                termbot_battle
                    .run_if(resource_exists::<CommanderState>)
                    .run_if(in_battle)
                    .in_set(SimSet::Commands)
                    .before(crate::sim::process_commands),
            ),
        );
    }
}

fn termbot_setup(mut state: ResMut<CommanderState>, phase: Res<MatchPhase>, mut queue: ResMut<CommandQueue>) {
    if state.placed || phase.ready[Faction::Termbots.index()] {
        return;
    }
    for command in default_army_commands(Faction::Termbots) {
        queue.push(command);
    }
    queue.push(SimCommand::Ready { issuer: Faction::Termbots });
    state.placed = true;
}

#[allow(clippy::too_many_arguments)]
fn termbot_battle(
    tick: Res<SimTick>,
    supply: Res<Supply>,
    mut state: ResMut<CommanderState>,
    mut queue: ResMut<CommandQueue>,
    platoons: Query<(&Faction, &Platoon)>,
    positions: Query<&TilePos>,
    fighters: Query<(&Faction, &Fighter)>,
    vehicles: Query<(&Faction, &Vehicle)>,
) {
    let me = Faction::Termbots;
    let mut ours: Vec<&Platoon> = platoons.iter().filter(|(f, _)| **f == me).map(|(_, p)| p).collect();
    ours.sort_by_key(|p| p.id);

    // Opening orders for the starting army.
    if tick.0 == OPENING_TICK {
        for (i, platoon) in ours.iter().enumerate().take(3) {
            let (mission, target) = match i {
                0 => (Mission::Assault, TilePos::new(56, 22)),
                1 => (Mission::Overwatch, TilePos::new(63, 70)),
                _ => (Mission::Assault, TilePos::new(70, 22)),
            };
            queue.push(SimCommand::Signal { issuer: me, platoon: platoon.id, mission, target });
            state.ordered.insert(platoon.id);
        }
    }

    // Reinforcements.
    if tick.0 >= FIRST_REINFORCEMENT_TICK && (tick.0 - FIRST_REINFORCEMENT_TICK) % REINFORCEMENT_INTERVAL == 0 {
        let n = state.deployed as usize;
        let classes = REINFORCEMENTS[n % REINFORCEMENTS.len()];
        let vehicle = n % 2 == 1;
        let counts = count_classes(me, fighters.iter(), vehicles.iter());
        if platoon_cost(classes, &counts, vehicle) <= supply.of(me) {
            let target = TilePos::new(40 + 24 * (n as i32 % 3), 74);
            queue.push(SimCommand::Deploy { issuer: me, classes: classes.to_vec(), vehicle, target });
            state.deployed += 1;
        }
    }

    // Once a reinforcement platoon reaches its drop-off, send it to assault.
    for platoon in ours {
        if state.ordered.contains(&platoon.id) {
            continue;
        }
        let arrived = platoon.members.iter().filter_map(|&m| positions.get(m).ok()).any(|p| p.distance(platoon.target) <= 5.0);
        if arrived {
            let target = TilePos::new(63, 22);
            queue.push(SimCommand::Signal { issuer: me, platoon: platoon.id, mission: Mission::Assault, target });
            state.ordered.insert(platoon.id);
        }
    }
}
