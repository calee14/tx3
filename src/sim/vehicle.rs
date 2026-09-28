//! Troop vehicles.
//!
//! A reinforcement platoon can ride a vehicle instead of walking from
//! base. The vehicle drives fast (6 tiles/s) to the drop-off, unloads the
//! platoon there in Overwatch, and stays put as a solid obstacle that
//! gives heavy cover, like a rock.
//!
//! Vehicles are armored (50%) but can be shot while driving. If one is
//! destroyed on the way, the passengers bail out at the wreck, hurt and
//! shaken, and the wreck becomes the obstacle instead.
//!
//! A vehicle only becomes an obstacle if that keeps every walkable tile
//! reachable and nobody else is standing on its tile; otherwise it drives
//! off after unloading.

use std::collections::{HashSet, VecDeque};

use bevy::prelude::*;

use super::fighter::{Condition, Health};
use super::perception::{Perception, Vision};
use super::platoon::{Mission, NewPlatoon, Recruit, spawn_platoon};
use super::{Mover, SimInterp, Unit, UnitClass, UnitId, UnitIndex};
use crate::faction::Faction;
use crate::game::MatchEntity;
use crate::map::generate::reachable_from;
use crate::map::{Feature, GameMap, MAP_H, MAP_W, TilePos};

pub const VEHICLE_SPEED: f32 = 6.0;
pub const VEHICLE_HEALTH: f32 = 300.0;
pub const VEHICLE_ARMOR: f32 = 0.5;
pub const VEHICLE_VISION: f32 = 8.0;

#[derive(Component, Debug)]
pub struct Vehicle {
    /// The platoon riding inside (traits rolled at purchase).
    pub cargo: Vec<Recruit>,
    pub destination: TilePos,
    /// Set when its health runs out; handled by `update_vehicles`.
    pub destroyed: bool,
    pub paid: f32,
}

/// A vehicle that has unloaded and now stands as an obstacle.
#[derive(Component, Debug)]
pub struct ParkedVehicle {
    pub wrecked: bool,
}

pub fn spawn_vehicle(
    commands: &mut Commands,
    index: &mut UnitIndex,
    faction: Faction,
    at: TilePos,
    cargo: Vec<Recruit>,
    path: VecDeque<TilePos>,
    paid: f32,
) -> Entity {
    let id = index.allocate();
    let destination = path.back().copied().unwrap_or(at);
    let mut mover = Mover::new(VEHICLE_SPEED);
    mover.path = path;
    let entity = commands
        .spawn((
            MatchEntity,
            Unit { id, class: UnitClass::Vehicle },
            faction,
            at,
            mover,
            Vision { radius: VEHICLE_VISION },
            Perception::default(),
            SimInterp::at(at.to_world()),
            Transform::from_translation(at.to_world().extend(0.0)),
            Health::new(VEHICLE_HEALTH),
            Vehicle { cargo, destination, destroyed: false, paid },
        ))
        .id();
    index.entities.insert(id, entity);
    entity
}

/// Is every walkable tile still reachable?
fn fully_connected(map: &GameMap) -> bool {
    let reach = reachable_from(map, map.spawns[0]);
    (0..MAP_H).all(|y| {
        (0..MAP_W).all(|x| {
            let p = TilePos::new(x, y);
            reach[(y * MAP_W + x) as usize] || !map.tile(p).passable()
        })
    })
}

pub fn update_vehicles(
    mut commands: Commands,
    mut map: ResMut<GameMap>,
    mut index: ResMut<UnitIndex>,
    positions: Query<&TilePos, With<Unit>>,
    vehicles: Query<(Entity, &Unit, &Faction, &TilePos, &Mover, &Vehicle)>,
) {
    let mut order: Vec<(UnitId, Entity)> = vehicles.iter().map(|v| (v.1.id, v.0)).collect();
    order.sort();

    for (id, entity) in order {
        let Ok((_, _, faction, pos, mover, vehicle)) = vehicles.get(entity) else { continue };
        // Unload when it arrives (or its path got blocked), or when destroyed.
        if !vehicle.destroyed && !mover.path.is_empty() {
            continue;
        }

        let mut taken: HashSet<TilePos> = positions.iter().copied().collect();
        taken.insert(*pos);
        let (_, members) = spawn_platoon(
            &mut commands,
            &mut index,
            &map,
            &mut taken,
            NewPlatoon {
                faction: *faction,
                recruits: &vehicle.cargo,
                anchor: *pos,
                mission: Mission::Overwatch,
                target: *pos,
                paid: vehicle.paid,
            },
            |_| true,
        );

        if vehicle.destroyed {
            // Bailed out of a burning wreck: hurt, suppressed and shaken.
            for (member, recruit) in members.iter().zip(&vehicle.cargo) {
                let max = recruit.max_health();
                let mut condition = Condition::new(&recruit.traits);
                condition.suppression = 60.0;
                condition.morale = (condition.morale - 25.0).max(0.0);
                commands.entity(*member).insert((Health { current: max * 0.6, max }, condition));
            }
        }

        // Park as an obstacle, if nobody else is on the tile and the map
        // stays connected.
        let others_here = positions.iter().filter(|p| **p == *pos).count() > 1;
        if !others_here && map.tile(*pos).feature != Feature::Vehicle {
            let before = map.tile(*pos).feature;
            map.tile_mut(*pos).feature = Feature::Vehicle;
            if fully_connected(&map) {
                map.version += 1;
                map.terrain_log.push(*pos);
                commands.spawn((
                    MatchEntity,
                    ParkedVehicle { wrecked: vehicle.destroyed },
                    *faction,
                    *pos,
                    Transform::from_translation(pos.to_world().extend(0.0)),
                ));
            } else {
                map.tile_mut(*pos).feature = before;
            }
        }

        index.entities.remove(&id);
        commands.entity(entity).despawn();
    }
}
