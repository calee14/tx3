//! Scenario tests for step 5 and 6: gadgets, strikes, explosions,
//! loadouts. Each runs in a sandbox: a flat, open map, battle already
//! started, no AI commander, and units placed by hand.

use std::collections::HashSet;

use bevy::ecs::world::CommandQueue as EcsQueue;
use bevy::prelude::*;

use super::classes::{Equipment, FighterClass, Loadout};
use super::combat::ShotLog;
use super::economy::{MatchPhase, SetupLog, Supply};
use super::fighter::{Condition, Health};
use super::gadgets::*;
use super::platoon::{Mission, NewPlatoon, Platoon, roll_recruits, spawn_platoon};
use super::*;
use crate::faction::Faction;
use crate::map::{Feature, GameMap, Tile, TilePos};

const H: Faction = Faction::Humans;
const T: Faction = Faction::Termbots;

fn sandbox() -> App {
    let mut app = App::new();
    app.add_plugins((SimPlugin, crate::fog::FogPlugin));
    let world = app.world_mut();
    let mut map = {
        let mut queue = EcsQueue::default();
        let mut commands = Commands::new(&mut queue, world);
        let map = crate::game::setup_simulation(&mut commands, 42);
        queue.apply(world);
        map
    };
    for t in &mut map.tiles {
        *t = Tile::default();
    }
    world.insert_resource(map);
    world.remove_resource::<crate::termbot::AiCommanders>();
    world.resource_mut::<MatchPhase>().battle = true;
    world.resource_mut::<Supply>().0 = [5000.0; 2];
    app
}

fn run(app: &mut App, ticks: u32) {
    for _ in 0..ticks {
        app.world_mut().run_schedule(FixedUpdate);
    }
}

fn squad(app: &mut App, faction: Faction, classes: &[FighterClass], anchor: TilePos, mission: Mission) -> Vec<Entity> {
    let world = app.world_mut();
    let recruits = roll_recruits(classes, &mut world.resource_mut::<SimRng>().0);
    let map = world.resource::<GameMap>().clone();
    let mut queue = EcsQueue::default();
    let members = world.resource_scope(|world, mut index: Mut<UnitIndex>| {
        let mut commands = Commands::new(&mut queue, world);
        let new = NewPlatoon { faction, recruits: &recruits, anchor, mission, target: anchor, paid: 0.0 };
        spawn_platoon(&mut commands, &mut index, &map, &mut HashSet::new(), new, |_| true).1
    });
    queue.apply(world);
    members
}

fn gadget(app: &mut App, faction: Faction, kind: GadgetKind, at: TilePos) -> Entity {
    let world = app.world_mut();
    let mut queue = EcsQueue::default();
    let e = world.resource_scope(|world, mut index: Mut<UnitIndex>| {
        let mut commands = Commands::new(&mut queue, world);
        spawn_gadget(&mut commands, &mut index, faction, kind, at)
    });
    queue.apply(world);
    e
}

fn king(app: &mut App, faction: Faction) -> Entity {
    let world = app.world_mut();
    world
        .query::<(Entity, &Unit, &Faction)>()
        .iter(world)
        .find(|(_, u, f)| u.class == UnitClass::King && **f == faction)
        .unwrap()
        .0
}

fn exists(app: &App, e: Entity) -> bool {
    app.world().get_entity(e).is_ok()
}

fn push(app: &mut App, command: SimCommand) {
    app.world_mut().resource_mut::<super::CommandQueue>().push(command);
}

#[test]
fn mine_goes_off_under_an_enemy() {
    let mut app = sandbox();
    let mine = gadget(&mut app, H, GadgetKind::Mine, TilePos::new(60, 60));
    let k = king(&mut app, T);
    *app.world_mut().get_mut::<TilePos>(k).unwrap() = TilePos::new(60, 63);
    let id = app.world().get::<Unit>(k).unwrap().id;
    push(&mut app, SimCommand::Move { issuer: T, unit: id, target: TilePos::new(60, 56) });
    run(&mut app, 80);
    assert!(!exists(&app, mine), "the mine went off");
    let health = app.world().get::<Health>(k).unwrap();
    assert!(health.current < health.max - 40.0, "the King was hurt: {health:?}");
}

#[test]
fn traps_stay_hidden_until_an_engineer_finds_and_defuses_them() {
    let mut app = sandbox();
    let mine = gadget(&mut app, H, GadgetKind::Mine, TilePos::new(60, 60));
    // An assaulter 3 tiles away doesn't notice it, and never targets it.
    squad(&mut app, T, &[FighterClass::Assaulter], TilePos::new(60, 63), Mission::Overwatch);
    run(&mut app, 20);
    assert!(app.world().resource::<KnownTraps>().of(T).is_empty(), "only adjacent fighters spot traps");
    // An engineer 5 tiles away does.
    let eng = squad(&mut app, T, &[FighterClass::Engineer], TilePos::new(55, 60), Mission::Overwatch)[0];
    run(&mut app, 4);
    assert!(app.world().resource::<KnownTraps>().of(T).contains(&TilePos::new(60, 60)));
    // ...and defuses it, without setting it off.
    run(&mut app, 20 * 12);
    assert!(!exists(&app, mine), "defused");
    let h = app.world().get::<Health>(eng).unwrap();
    assert_eq!(h.current, h.max, "no explosion");
    assert!(app.world().resource::<BlastLog>().0.is_empty());
}

#[test]
fn engineer_builds_a_turret_when_signalled() {
    let mut app = sandbox();
    let members = squad(&mut app, H, &[FighterClass::Engineer, FighterClass::Assaulter], TilePos::new(40, 30), Mission::Overwatch);
    let world = app.world_mut();
    let platoon = world.get::<super::fighter::Fighter>(members[0]).unwrap().platoon;
    let id = world.get::<Platoon>(platoon).unwrap().id;
    let before = world.resource::<Supply>().0[0];
    push(&mut app, SimCommand::Build { issuer: H, platoon: id, kind: GadgetKind::Turret, target: TilePos::new(43, 33) });
    run(&mut app, 2);
    assert_eq!(app.world().resource::<Supply>().0[0], before - 150.0 + 3.0 * 2.0 * DT, "paid when sent");
    run(&mut app, 20 * 16);
    let world = app.world_mut();
    let turrets: Vec<(Faction, TilePos)> =
        world.query::<(&Faction, &TilePos, &Gadget)>().iter(world).map(|(f, p, _)| (*f, *p)).collect();
    assert_eq!(turrets, vec![(H, TilePos::new(43, 33))]);
}

#[test]
fn turrets_hide_until_close_or_firing_then_shoot() {
    let mut app = sandbox();
    let turret = gadget(&mut app, H, GadgetKind::Turret, TilePos::new(60, 60));
    let enemy = squad(&mut app, T, &[FighterClass::Juggernaut], TilePos::new(60, 73), Mission::Overwatch)[0];
    run(&mut app, 20);
    assert!(!app.world().get::<Concealed>(turret).unwrap().known_to(T), "out of sight and out of range");
    // Bring the enemy into range (but not within 6 tiles).
    *app.world_mut().get_mut::<TilePos>(enemy).unwrap() = TilePos::new(60, 69);
    let mut shots = 0;
    for _ in 0..60 {
        run(&mut app, 1);
        let mut log = app.world_mut().resource_mut::<ShotLog>();
        shots += log.0.iter().filter(|s| s.shooter == UnitClass::Gadget(GadgetKind::Turret)).count();
        log.0.clear();
    }
    assert!(shots >= 3, "turret fired {shots} times");
    assert!(app.world().get::<Concealed>(turret).unwrap().known_to(T), "firing gave it away");
}

#[test]
fn artillery_shells_a_visible_group_with_a_warning() {
    let mut app = sandbox();
    gadget(&mut app, H, GadgetKind::Artillery, TilePos::new(60, 30));
    squad(&mut app, H, &[FighterClass::Sniper], TilePos::new(60, 52), Mission::Overwatch);
    squad(&mut app, T, &[FighterClass::Assaulter; 3], TilePos::new(60, 60), Mission::Overwatch);
    let mut strike = None;
    for _ in 0..40 {
        run(&mut app, 1);
        let world = app.world_mut();
        if let Some(s) = world.query::<&Strike>().iter(world).next() {
            strike = Some(s.clone());
            break;
        }
    }
    let strike = strike.expect("artillery fired");
    assert_eq!(strike.kind, StrikeKind::Artillery);
    let (at, due) = strike.impacts[0];
    assert!(at.distance(TilePos::new(60, 58)) <= 5.0, "aimed at the group: {at:?}");
    assert_eq!(due, strike.launched_tick + ARTILLERY_DELAY, "3 s warning");
    run(&mut app, ARTILLERY_DELAY as u32 + 2);
    assert!(!app.world().resource::<BlastLog>().0.is_empty(), "the shell landed");
}

#[test]
fn fighters_dodge_strikes_they_can_see() {
    let mut app = sandbox();
    let members = squad(&mut app, T, &[FighterClass::Assaulter; 3], TilePos::new(60, 60), Mission::Overwatch);
    run(&mut app, 10);
    let world = app.world_mut();
    let tick = world.resource::<SimTick>().0;
    let center = *world.get::<TilePos>(members[0]).unwrap();
    world.spawn(Strike {
        faction: H,
        kind: StrikeKind::Artillery,
        impacts: vec![(center, tick + ARTILLERY_DELAY)],
        radius: ARTILLERY_RADIUS,
        damage: ARTILLERY_DAMAGE,
        breach: false,
        source: None,
        launched_tick: tick,
    });
    run(&mut app, ARTILLERY_DELAY as u32 - 1);
    let clear = members
        .iter()
        .filter(|m| app.world().get::<TilePos>(**m).unwrap().distance(center) > ARTILLERY_RADIUS)
        .count();
    assert!(clear >= 2, "only {clear} of 3 got clear");
}

#[test]
fn air_strikes_cost_supply_and_have_a_cooldown() {
    let mut app = sandbox();
    let target = TilePos::new(60, 60);
    push(&mut app, SimCommand::AirStrike { issuer: H, target });
    run(&mut app, 1);
    let count = |app: &mut App| {
        let world = app.world_mut();
        world.query::<&Strike>().iter(world).count()
    };
    assert_eq!(count(&mut app), 0, "not available in the first minute");
    app.world_mut().resource_mut::<AirSupport>().ready_tick = [0; 2];
    let before = app.world().resource::<Supply>().0[0];
    push(&mut app, SimCommand::AirStrike { issuer: H, target });
    run(&mut app, 1);
    assert_eq!(count(&mut app), 1);
    assert!(app.world().resource::<Supply>().0[0] < before - AIR_STRIKE_COST + 1.0);
    push(&mut app, SimCommand::AirStrike { issuer: H, target });
    run(&mut app, 1);
    assert_eq!(count(&mut app), 1, "on cooldown");
    let world = app.world_mut();
    let s = world.query::<&Strike>().iter(world).next().unwrap().clone();
    assert_eq!(s.impacts.len(), AIR_STRIKE_IMPACTS as usize);
    assert!(s.impacts.iter().all(|(p, _)| p.distance(target) <= 5.0));
}

#[test]
fn explosions_break_walls_and_flatten_sandbags() {
    let mut app = sandbox();
    {
        let mut map = app.world_mut().resource_mut::<GameMap>();
        map.tile_mut(TilePos::new(70, 70)).feature = Feature::Wall;
        map.tile_mut(TilePos::new(72, 70)).feature = Feature::LightCover;
        map.tile_mut(TilePos::new(70, 72)).feature = Feature::Rock;
    }
    let version = app.world().resource::<GameMap>().version;
    app.world_mut().resource_mut::<PendingExplosions>().0.push(Explosion {
        at: TilePos::new(70, 70),
        radius: 2.5,
        damage: 65.0,
        breach: false,
        source: None,
        faction: H,
    });
    run(&mut app, 1);
    let map = app.world().resource::<GameMap>();
    assert_eq!(map.tile(TilePos::new(70, 70)).feature, Feature::LightCover, "wall to rubble");
    assert_eq!(map.tile(TilePos::new(72, 70)).feature, Feature::None, "sandbags flattened");
    assert_eq!(map.tile(TilePos::new(70, 72)).feature, Feature::Rock, "rock needs a breaching charge");
    assert!(map.version > version);
    assert!(map.terrain_log.contains(&TilePos::new(70, 70)));
}

#[test]
fn loadouts_stay_secret_until_used() {
    let mut app = sandbox();
    let gunner = squad(&mut app, H, &[FighterClass::HeavyWeapons], TilePos::new(60, 50), Mission::Overwatch)[0];
    let rifle = squad(&mut app, H, &[FighterClass::Assaulter], TilePos::new(64, 50), Mission::Overwatch)[0];
    *app.world_mut().get_mut::<Equipment>(gunner).unwrap() = Equipment::new(Loadout::MachineGun, 0);
    *app.world_mut().get_mut::<Equipment>(rifle).unwrap() = Equipment::new(Loadout::Rifle, 0);
    assert!(!app.world().get::<Equipment>(gunner).unwrap().known_to[1]);
    squad(&mut app, T, &[FighterClass::Assaulter], TilePos::new(60, 58), Mission::Overwatch);
    run(&mut app, 60);
    assert!(app.world().get::<Equipment>(gunner).unwrap().known_to[1], "machine-gun fire gives it away");
    assert!(!app.world().get::<Equipment>(rifle).unwrap().known_to[1], "a plain rifle tells nothing");
}

#[test]
fn smoke_kit_pops_smoke_when_pinned() {
    let mut app = sandbox();
    let sniper = squad(&mut app, H, &[FighterClass::Sniper], TilePos::new(60, 50), Mission::Overwatch)[0];
    *app.world_mut().get_mut::<Equipment>(sniper).unwrap() = Equipment::new(Loadout::SmokeKit, 0);
    app.world_mut().get_mut::<Condition>(sniper).unwrap().suppression = 90.0;
    run(&mut app, 2);
    let map = app.world().resource::<GameMap>();
    assert!(map.smoky(TilePos::new(60, 50)));
    run(&mut app, 20 * 9);
    assert!(!app.world().resource::<GameMap>().smoky(TilePos::new(60, 50)), "smoke clears");
}

#[test]
fn setup_gadgets_can_be_undone() {
    let mut app = sandbox();
    app.world_mut().resource_mut::<MatchPhase>().battle = false;
    let before = app.world().resource::<Supply>().0[0];
    push(&mut app, SimCommand::PlaceGadget { issuer: H, kind: GadgetKind::Turret, at: TilePos::new(60, 30) });
    push(&mut app, SimCommand::PlaceGadget { issuer: H, kind: GadgetKind::Mine, at: TilePos::new(60, 100) });
    run(&mut app, 1);
    assert_eq!(app.world().resource::<SetupLog>().0[0].len(), 1, "can't place in the enemy's half");
    assert_eq!(app.world().resource::<Supply>().0[0], before - 150.0);
    push(&mut app, SimCommand::UndoPlacement { issuer: H });
    run(&mut app, 1);
    let world = app.world_mut();
    assert_eq!(world.query::<&Gadget>().iter(world).count(), 0);
    assert_eq!(world.resource::<Supply>().0[0], before);
}

#[test]
fn fighting_earns_experience() {
    let mut app = sandbox();
    let shooters = squad(&mut app, H, &[FighterClass::Sniper, FighterClass::Assaulter], TilePos::new(60, 50), Mission::Overwatch);
    squad(&mut app, T, &[FighterClass::Assaulter; 2], TilePos::new(60, 58), Mission::Overwatch);
    run(&mut app, 20 * 20);
    let xp: f32 = shooters
        .iter()
        .filter_map(|s| app.world().get::<super::fighter::Veterancy>(*s))
        .map(|v| v.xp)
        .sum();
    assert!(xp > 5.0, "xp earned: {xp}");
}

/// One `class` with `loadout` for Humans at (60, 50), facing a Termbot
/// `target` at `at`; runs `secs` and returns (Humans fighter, blasts seen).
fn ability_duel(
    class: FighterClass,
    loadout: Loadout,
    target: FighterClass,
    at: TilePos,
    obstacle: Option<(TilePos, Feature)>,
    secs: u32,
) -> (App, Entity, usize) {
    let mut app = sandbox();
    if let Some((p, f)) = obstacle {
        app.world_mut().resource_mut::<GameMap>().tile_mut(p).feature = f;
    }
    let me = squad(&mut app, H, &[class], TilePos::new(60, 50), Mission::Overwatch)[0];
    *app.world_mut().get_mut::<Equipment>(me).unwrap() = Equipment::new(loadout, 0);
    squad(&mut app, T, &[target], at, Mission::Overwatch);
    run(&mut app, secs * 20);
    let blasts = app.world().resource::<BlastLog>().0.len();
    (app, me, blasts)
}

#[test]
fn grenades_go_after_dug_in_enemies() {
    let (app, me, blasts) = ability_duel(
        FighterClass::Assaulter,
        Loadout::Grenades,
        FighterClass::Assaulter,
        TilePos::new(60, 54),
        Some((TilePos::new(60, 53), Feature::LightCover)),
        4,
    );
    assert!(blasts >= 1, "a grenade went off");
    assert!(app.world().get::<Equipment>(me).unwrap().known_to[1]);
}

#[test]
fn rockets_hit_armored_targets() {
    let (app, me, blasts) =
        ability_duel(FighterClass::HeavyWeapons, Loadout::Rockets, FighterClass::Juggernaut, TilePos::new(60, 57), None, 4);
    assert!(blasts >= 1, "a rocket went off");
    assert!(app.world().get::<Equipment>(me).unwrap().known_to[1]);
}

#[test]
fn breaching_charges_crack_the_rock_an_enemy_hides_behind() {
    let rock = TilePos::new(60, 54);
    let (app, _, _) = ability_duel(
        FighterClass::Juggernaut,
        Loadout::BreachingCharge,
        FighterClass::Sniper,
        TilePos::new(60, 55),
        Some((rock, Feature::Rock)),
        4,
    );
    assert_ne!(app.world().resource::<GameMap>().tile(rock).feature, Feature::Rock, "rock breached");
}

/// Ticks from the build order until the turret stands, for a platoon of
/// `n` engineers (all with the same loadout, so only numbers differ).
fn turret_build_ticks(n: usize) -> u32 {
    let mut app = sandbox();
    let members = squad(&mut app, H, &vec![FighterClass::Engineer; n], TilePos::new(40, 30), Mission::Overwatch);
    for m in &members {
        *app.world_mut().get_mut::<Equipment>(*m).unwrap() = Equipment::new(Loadout::Toolkit, 0);
    }
    let platoon = app.world().get::<super::fighter::Fighter>(members[0]).unwrap().platoon;
    let id = app.world().get::<Platoon>(platoon).unwrap().id;
    push(&mut app, SimCommand::Build { issuer: H, platoon: id, kind: GadgetKind::Turret, target: TilePos::new(41, 33) });
    for t in 1..=20 * 30 {
        run(&mut app, 1);
        let world = app.world_mut();
        if world.query::<&Gadget>().iter(world).count() > 0 {
            return t;
        }
    }
    panic!("turret never built with {n} engineers");
}

#[test]
fn more_engineers_build_faster() {
    let one = turret_build_ticks(1);
    let three = turret_build_ticks(3);
    // Turret: 8 s of work, Toolkit x1.6 -> 5 s alone, about 1.7 s for three
    // (plus walking to the site either way).
    assert!(three + 40 < one, "1 engineer: {one} ticks, 3 engineers: {three} ticks");
    eprintln!("turret build: 1 engineer {one} ticks, 3 engineers {three} ticks");
    assert!(one >= 100, "a lone engineer takes at least 5 s: {one}");
}
