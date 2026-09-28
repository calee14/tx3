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
use crate::sim::economy::{MatchPhase, SetupLog, Supply, Territory};
use crate::sim::platoon::spawn_king;
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
    let rng = ChaCha8Rng::seed_from_u64(map.seed ^ 0x5EED_0F_BA77_1E);
    let mut index = UnitIndex::default();
    for faction in Faction::ALL {
        spawn_king(commands, &mut index, &map, faction);
    }
    // Armies are placed during the setup phase.
    commands.insert_resource(MatchPhase::default());
    commands.insert_resource(Supply::default());
    commands.insert_resource(SetupLog::default());
    commands.insert_resource(Territory::new(&map));
    commands.insert_resource(crate::commander::CommanderState::default());
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

    /// A headless match: map generated, both default armies placed and
    /// ready (the battle starts on the first tick).
    fn headless_app(seed: u64) -> App {
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
        let mut queue = world.resource_mut::<CommandQueue>();
        for c in crate::commander::default_army_commands(Faction::Humans) {
            queue.push(c);
        }
        queue.push(crate::sim::SimCommand::Ready { issuer: Faction::Humans });
        app
    }

    /// Run ticks, returning the number of shots fired.
    fn run_ticks(app: &mut App, ticks: u32) -> usize {
        let world = app.world_mut();
        let mut shots = 0;
        for _ in 0..ticks {
            world.run_schedule(FixedUpdate);
            let mut log = world.resource_mut::<crate::sim::combat::ShotLog>();
            shots += log.0.len();
            log.0.clear();
        }
        shots
    }

    /// Run a match headless for `seconds` and return every unit's
    /// (id, position, health) at the end, plus the shots fired.
    fn run_headless(seed: u64, seconds: u32) -> (Vec<(u32, TilePos, i32)>, usize) {
        let mut app = headless_app(seed);
        let shots = run_ticks(&mut app, seconds * crate::sim::SIM_HZ as u32);
        let world = app.world_mut();
        let mut state: Vec<(u32, TilePos, i32)> = world
            .query::<(&Unit, &TilePos, &Health)>()
            .iter(world)
            .map(|(u, p, h)| (u.id.0, *p, (h.current * 100.0) as i32))
            .collect();
        state.sort_by_key(|s| s.0);
        (state, shots)
    }

    #[test]
    fn setup_places_both_armies_then_battle_starts() {
        let mut app = headless_app(7);
        run_ticks(&mut app, 2);
        let world = app.world_mut();
        assert!(world.resource::<MatchPhase>().battle);
        let platoons = world.query::<&crate::sim::platoon::Platoon>().iter(world).count();
        assert_eq!(platoons, 6);
        let supply = world.resource::<Supply>();
        assert!(supply.0[0] < 100.0 && supply.0[0] == supply.0[1], "both spent the same: {:?}", supply.0);
    }

    #[test]
    fn vehicle_delivers_platoon_and_parks_as_obstacle() {
        use crate::map::{Feature, GameMap};
        use crate::sim::classes::FighterClass;
        let mut app = headless_app(42);
        run_ticks(&mut app, 2);
        let target = TilePos::new(40, 30);
        app.world_mut().resource_mut::<Supply>().0[0] += 500.0;
        app.world_mut().resource_mut::<CommandQueue>().push(crate::sim::SimCommand::Deploy {
            issuer: Faction::Humans,
            classes: vec![FighterClass::Assaulter, FighterClass::Sniper],
            vehicle: true,
            target,
        });
        run_ticks(&mut app, 1);
        let world = app.world_mut();
        assert_eq!(world.query::<&crate::sim::vehicle::Vehicle>().iter(world).count(), 1, "vehicle on its way");
        run_ticks(&mut app, 30 * 20);
        let world = app.world_mut();
        assert_eq!(world.query::<&crate::sim::vehicle::Vehicle>().iter(world).count(), 0, "vehicle unloaded");
        let map = world.resource::<GameMap>();
        let parked: Vec<TilePos> = (0..128)
            .flat_map(|y| (0..128).map(move |x| TilePos::new(x, y)))
            .filter(|p| map.tile(*p).feature == Feature::Vehicle)
            .collect();
        assert_eq!(parked.len(), 1);
        assert!(parked[0].distance(target) <= 2.0, "parked at {:?}", parked[0]);
        assert!(map.version > 0);
        let delivered = world
            .query::<&crate::sim::platoon::Platoon>()
            .iter(world)
            .filter(|p| p.members.len() == 2 && p.target.distance(target) <= 2.0)
            .count();
        assert_eq!(delivered, 1);
    }

    #[test]
    fn time_out_ends_the_match_on_territory() {
        let mut app = headless_app(42);
        run_ticks(&mut app, 2);
        app.world_mut().resource_mut::<crate::sim::SimTick>().0 = crate::sim::economy::MATCH_TICKS - 1;
        run_ticks(&mut app, 1);
        let outcome = app.world().resource::<crate::sim::combat::MatchOutcome>();
        assert!(outcome.ended);
        assert!(outcome.reason.starts_with("Time's up"), "{}", outcome.reason);
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
