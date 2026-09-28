//! Match setup and teardown.

use bevy::prelude::*;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::camera::CameraFocusRequest;
use crate::faction::{Faction, LocalPlayer};
use crate::fog::{FogOfWar, spawn_fog_overlay};
use crate::input::Selection;
use crate::map::generate::generate_map;
use crate::map::render::spawn_map_visuals;
use crate::sim::{CommandQueue, SimRng, SimTick, UnitIndex};
use crate::units::spawn_king;

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, start_first_match)
            .add_systems(Update, regenerate_on_key);
    }
}

/// Tag for everything that belongs to the current match, so it can all be
/// cleared when a new match starts.
#[derive(Component)]
pub struct MatchEntity;

/// Seed from the command line (`cargo run -- 12345`), or from the clock.
fn initial_seed() -> u64 {
    std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() % 100_000)
                .unwrap_or(0)
        })
}

fn start_first_match(mut commands: Commands, mut images: ResMut<Assets<Image>>, local: Res<LocalPlayer>) {
    start_match(&mut commands, &mut images, initial_seed(), local.0);
}

pub fn start_match(commands: &mut Commands, images: &mut Assets<Image>, seed: u64, local: Faction) {
    let map = generate_map(seed);
    info!("Starting match on map seed {}", map.seed);

    spawn_map_visuals(commands, images, &map);
    spawn_fog_overlay(commands, images);

    let mut index = UnitIndex::default();
    for faction in Faction::ALL {
        spawn_king(commands, &mut index, faction, map.spawn(faction));
    }

    commands.insert_resource(CameraFocusRequest(Some(map.spawn(local).to_world())));
    // Separate stream from the map generator, but still derived from the seed.
    commands.insert_resource(SimRng(ChaCha8Rng::seed_from_u64(map.seed ^ 0x5EED_0F_BA77_1E)));
    commands.insert_resource(SimTick::default());
    commands.insert_resource(CommandQueue::default());
    commands.insert_resource(FogOfWar::new(&map));
    commands.insert_resource(Selection::default());
    commands.insert_resource(index);
    commands.insert_resource(map);
}

/// N: throw away the current match and generate a new map (next seed).
fn regenerate_on_key(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    map: Option<Res<crate::map::GameMap>>,
    local: Res<LocalPlayer>,
    entities: Query<Entity, With<MatchEntity>>,
) {
    if !keys.just_pressed(KeyCode::KeyN) {
        return;
    }
    let next_seed = map.map(|m| m.seed + 1).unwrap_or_else(initial_seed);
    for entity in &entities {
        commands.entity(entity).despawn();
    }
    start_match(&mut commands, &mut images, next_seed, local.0);
}
