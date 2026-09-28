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
use crate::sim::combat::MatchOutcome;
use crate::sim::platoon::spawn_forces;
use crate::sim::{CommandQueue, SimRng, SimTick, UnitIndex};

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
    let map = setup_simulation(commands, seed);
    info!("Starting match on map seed {}", map.seed);
    spawn_map_visuals(commands, images, &map);
    spawn_fog_overlay(commands, images);
    commands.insert_resource(CameraFocusRequest(Some(map.spawn(local).to_world())));
    commands.insert_resource(Selection::default());
    commands.insert_resource(map);
}

/// Everything the simulation needs for a match, with no rendering, so it
/// can also run headless (tests, and later a dedicated server).
pub fn setup_simulation(commands: &mut Commands, seed: u64) -> crate::map::GameMap {
    let map = generate_map(seed);
    // Separate stream from the map generator, but still derived from the seed.
    let mut rng = ChaCha8Rng::seed_from_u64(map.seed ^ 0x5EED_0F_BA77_1E);
    let mut index = UnitIndex::default();
    for faction in Faction::ALL {
        spawn_forces(commands, &mut index, &mut rng, &map, faction);
    }
    commands.insert_resource(SimRng(rng));
    commands.insert_resource(MatchOutcome::default());
    commands.insert_resource(SimTick::default());
    commands.insert_resource(CommandQueue::default());
    commands.insert_resource(FogOfWar::new(&map));
    commands.insert_resource(index);
    map
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::TilePos;
    use crate::sim::fighter::Health;
    use crate::sim::{SimPlugin, Unit};

    /// Run a match headless for `seconds` and return every unit's
    /// (id, position, health) at the end, plus the shots fired.
    fn run_headless(seed: u64, seconds: u32) -> (Vec<(u32, TilePos, i32)>, usize) {
        let mut app = App::new();
        app.add_plugins((SimPlugin, crate::fog::FogPlugin, crate::commander::CommanderPlugin));
        let world = app.world_mut();
        let map = {
            let mut queue = bevy::ecs::world::CommandQueue::default();
            let mut commands = Commands::new(&mut queue, world);
            let map = setup_simulation(&mut commands, seed);
            queue.apply(world);
            map
        };
        world.insert_resource(map);
        let mut shots = 0;
        for _ in 0..seconds * crate::sim::SIM_HZ as u32 {
            world.run_schedule(FixedUpdate);
            let mut log = world.resource_mut::<crate::sim::combat::ShotLog>();
            shots += log.0.len();
            log.0.clear();
        }
        let mut state: Vec<(u32, TilePos, i32)> = world
            .query::<(&Unit, &TilePos, &Health)>()
            .iter(world)
            .map(|(u, p, h)| (u.id.0, *p, (h.current * 100.0) as i32))
            .collect();
        state.sort_by_key(|s| s.0);
        (state, shots)
    }

    /// Balance check: `cargo test battle_report -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn battle_report() {
        for seed in [42u64, 7, 1234] {
            for secs in [60u32, 120, 180] {
                let (state, shots) = run_headless(seed, secs);
                let alive = state.iter().filter(|s| s.2 > 0).count();
                let hurt = state.iter().filter(|s| s.2 > 0 && s.2 < 10000).count();
                let humans = state.iter().filter(|s| s.1.y < 64).count();
                eprintln!("seed {seed} t={secs}s shots={shots} units={} alive={alive} hurt={hurt} in-humans-half={humans}", state.len());
            }
        }
    }

    #[test]
    fn battle_happens_and_is_deterministic() {
        let (a, shots_a) = run_headless(42, 120);
        let (b, shots_b) = run_headless(42, 120);
        assert!(shots_a > 20, "expected a firefight, got {shots_a} shots");
        assert_eq!(shots_a, shots_b);
        assert_eq!(a, b, "same seed and orders must replay identically");
    }
}
