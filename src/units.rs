//! Spawning units. Placeholder art: a colored square with a class letter.

use bevy::prelude::*;

use crate::faction::Faction;
use crate::game::MatchEntity;
use crate::map::TilePos;
use crate::sim::perception::{Perception, Vision};
use crate::sim::{Mover, SimInterp, Unit, UnitClass, UnitIndex};

pub const Z_UNITS: f32 = 10.0;
pub const UNIT_SPRITE_SIZE: f32 = 13.0;

pub fn spawn_king(commands: &mut Commands, index: &mut UnitIndex, faction: Faction, pos: TilePos) -> Entity {
    let id = index.allocate();
    let world = pos.to_world();
    let entity = commands
        .spawn((
            MatchEntity,
            Unit { id, class: UnitClass::King },
            faction,
            pos,
            Mover::new(2.5),
            Vision { radius: 9.0 },
            Perception::default(),
            SimInterp::at(world),
            Sprite::from_color(faction.color(), Vec2::splat(UNIT_SPRITE_SIZE)),
            Transform::from_translation(world.extend(Z_UNITS)),
            children![(
                Text2d::new("K"),
                TextFont::from_font_size(11.0),
                TextColor(Color::BLACK),
                Transform::from_xyz(0.0, 0.0, 1.0),
            )],
        ))
        .id();
    index.entities.insert(id, entity);
    entity
}
