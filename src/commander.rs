//! Placeholder Termbot commander.
//!
//! Until the real Termbot AI arrives (build step 7), the Termbots follow a
//! fixed opening so there's something to fight: the Rifle and Breach
//! platoons assault the Humans' front line and the Recon platoon moves up
//! into overwatch near the middle. Like any player, it only acts by
//! pushing signals into the command queue.

use bevy::prelude::*;

use crate::faction::Faction;
use crate::map::TilePos;
use crate::sim::platoon::{Mission, Platoon};
use crate::sim::{CommandQueue, SimCommand, SimSet, SimTick};

/// Two seconds in.
const OPENING_TICK: u64 = 40;

pub struct CommanderPlugin;

impl Plugin for CommanderPlugin {
    fn build(&self, app: &mut App) {
        // Runs as part of the Commands stage, before the queue is processed.
        app.add_systems(FixedUpdate, termbot_opening.in_set(SimSet::Commands).before(crate::sim::process_commands));
    }
}

fn termbot_opening(tick: Res<SimTick>, mut queue: ResMut<CommandQueue>, platoons: Query<(&Faction, &Platoon)>) {
    if tick.0 != OPENING_TICK {
        return;
    }
    let mut ours: Vec<&Platoon> = platoons.iter().filter(|(f, _)| **f == Faction::Termbots).map(|(_, p)| p).collect();
    ours.sort_by_key(|p| p.id);
    for platoon in ours {
        let (mission, target) = if platoon.name.starts_with("Rifle") {
            (Mission::Assault, TilePos::new(56, 22))
        } else if platoon.name.starts_with("Breach") {
            (Mission::Assault, TilePos::new(70, 22))
        } else {
            (Mission::Overwatch, TilePos::new(63, 70))
        };
        queue.push(SimCommand::Signal { issuer: Faction::Termbots, platoon: platoon.id, mission, target });
    }
}
