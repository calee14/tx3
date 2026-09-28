//! The default army (the "Place default army" button in setup).
//! The Termbot AI commander lives in `termbot/`.

use crate::faction::Faction;
use crate::map::TilePos;
use crate::sim::SimCommand;
use crate::sim::armory::Armory;
use crate::sim::economy::fit_army;
use crate::sim::classes::FighterClass::{self, *};

/// The standard starting army (990 of the 1000 starting Supply with free kits),
/// as Humans' anchors; Termbots use the mirrored tiles.
pub const DEFAULT_ARMY: [(&[FighterClass], TilePos); 3] = [
    (&[Assaulter, Assaulter, Assaulter, HeavyWeapons], TilePos::new(52, 17)),
    (&[Sniper, Sniper, Assaulter], TilePos::new(63, 20)),
    (&[Juggernaut, Assaulter, Assaulter, HeavyWeapons], TilePos::new(74, 17)),
];

/// Commands that place the default army for a side, trimmed to what
/// `supply` can pay for with this armory's kits.
pub fn default_army_commands(faction: Faction, armory: &Armory, supply: f32) -> Vec<SimCommand> {
    let army = fit_army(DEFAULT_ARMY.iter().map(|(c, _)| c.to_vec()).collect(), supply, armory);
    DEFAULT_ARMY
        .iter()
        .zip(army)
        .map(|((_, anchor), classes)| SimCommand::PlacePlatoon {
            issuer: faction,
            classes,
            anchor: match faction {
                Faction::Humans => *anchor,
                Faction::Termbots => anchor.mirrored(),
            },
        })
        .collect()
}

