//! The default army (the "Place default army" button in setup).
//! The Termbot AI commander lives in `termbot/`.

use crate::faction::Faction;
use crate::map::TilePos;
use crate::sim::SimCommand;
use crate::sim::classes::FighterClass::{self, *};

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

