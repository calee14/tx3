//! Engineer gadgets, strikes and explosions.
//!
//! Gadgets are units that don't move: mines, bombs, turrets and artillery.
//! They're a little stronger than the fighters they stand in for, but easy
//! to destroy once found. All of them start hidden from the enemy:
//!
//! - Mines and bombs (traps) are never shot at. An enemy engineer spots
//!   them from 5 tiles, anyone else only when right next to one. Once a
//!   side knows about a trap, its fighters walk around it and its
//!   engineers defuse it.
//! - Turrets and artillery are revealed when an enemy gets within 6 tiles
//!   with a line of sight, or when they fire.
//!
//! Strikes (artillery shells, air strikes, grenades, rockets, breaching
//! charges) land after a delay. Everyone who can see the impact point sees
//! the warning marker, so fighters get a chance to dodge. Explosions hurt
//! everyone in the radius (friendly fire included), ignore cover, pin
//! fighters down, and break terrain: walls and parked vehicles become
//! rubble, sandbags are flattened, and breaching charges crack rock.

use std::collections::HashSet;

use bevy::prelude::*;
use rand::Rng;

use super::armory::{Armories, Machine};
use super::classes::{ArmorClass, DamageType, Equipment, FighterClass, Loadout, Traits, damage_multiplier};
use super::combat::{ShotLog, ShotRecord};
use super::economy::Supply;
use super::fighter::{Condition, Decision, Fighter, Health, PINNED, SUPPRESSED, Work, WorkKind, XpLog};
use super::perception::{Perception, Vision};
use super::platoon::{Mission, Platoon, nearest_free_tile_where};
use super::vehicle::ParkedVehicle;
use super::{DT, Mover, SIM_HZ, SimInterp, SimRng, SimTick, Unit, UnitClass, UnitId, UnitIndex};
use crate::faction::Faction;
use crate::game::MatchEntity;
use crate::map::{CoverLevel, Feature, GameMap, TilePos};

const fn secs(s: f32) -> u64 {
    (s * SIM_HZ as f32) as u64
}

// --- Traps ---
pub const MINE_RADIUS: f32 = 1.5;
pub const MINE_DAMAGE: f32 = 70.0;
pub const BOMB_TRIGGER: f32 = 2.0;
pub const BOMB_FUSE: f32 = 0.5;
pub const BOMB_RADIUS: f32 = 3.0;
pub const BOMB_DAMAGE: f32 = 90.0;
/// Engineers spot traps from this far; everyone else only when adjacent.
pub const TRAP_DETECT_ENGINEER: f32 = 5.0;
pub const TRAP_DETECT_OTHER: f32 = 1.5;
/// Turrets and artillery are spotted from this far.
pub const STRUCTURE_DETECT: f32 = 6.0;
pub const DEFUSE_SECONDS: f32 = 3.0;
/// Engineers go after known traps this close.
const DEFUSE_SEARCH: f32 = 8.0;

// --- Turret ---
pub const TURRET_RANGE: f32 = 11.0;
pub const TURRET_DAMAGE: f32 = 16.0;
pub const TURRET_INTERVAL: f32 = 0.6;
pub const TURRET_ACCURACY: f32 = 0.6;
pub const TURRET_SUPPRESSION: f32 = 6.0;

/// How a turret shoots, by variant (machine gun, or cannon).
#[derive(Clone, Copy, Debug)]
pub struct TurretProfile {
    pub range: f32,
    pub damage: f32,
    pub interval: f32,
    pub accuracy: f32,
    pub damage_type: DamageType,
    pub suppression: f32,
}

pub fn turret_profile(alt: bool) -> TurretProfile {
    if alt {
        TurretProfile {
            range: 13.0,
            damage: 40.0,
            interval: 2.0,
            accuracy: 0.5,
            damage_type: DamageType::Explosive,
            suppression: 15.0,
        }
    } else {
        TurretProfile {
            range: TURRET_RANGE,
            damage: TURRET_DAMAGE,
            interval: TURRET_INTERVAL,
            accuracy: TURRET_ACCURACY,
            damage_type: DamageType::Standard,
            suppression: TURRET_SUPPRESSION,
        }
    }
}

/// Blast of a mine or bomb, by variant: (radius, damage).
pub fn trap_blast(kind: GadgetKind, alt: bool) -> (f32, f32) {
    match (kind, alt) {
        (GadgetKind::Mine, false) => (MINE_RADIUS, MINE_DAMAGE),
        (GadgetKind::Mine, true) => (2.5, 55.0),
        (GadgetKind::Bomb, false) => (BOMB_RADIUS, BOMB_DAMAGE),
        (GadgetKind::Bomb, true) => (4.0, 110.0),
        _ => (0.0, 0.0),
    }
}

// --- Artillery ---
// Artillery is support fire the King doesn't aim: it shells enemies its
// side can see near its own troops, from long range, slowly, and not very
// accurately. A shell can land square on target, graze it, or miss.
pub const ARTILLERY_DELAY: u64 = secs(3.0);
pub const ARTILLERY_RADIUS: f32 = 2.5;
pub const ARTILLERY_DAMAGE: f32 = 65.0;
pub const ARTILLERY_COOLDOWN: f32 = 25.0;
pub const ARTILLERY_RANGE: f32 = 60.0;
/// Shells land anywhere within this many tiles of the aim point.
pub const ARTILLERY_SCATTER: f32 = 3.0;
/// It only shells enemies within this distance of one of its side's units.
pub const ARTILLERY_SUPPORT_RANGE: f32 = 12.0;
/// It never aims this close to its own units (a stray shell still can).
pub const ARTILLERY_SAFETY: f32 = 2.5;

/// (radius, damage, cooldown, scatter) by variant (field gun, howitzer).
pub fn artillery_profile(alt: bool) -> (f32, f32, f32, f32) {
    if alt {
        (3.5, 85.0, 40.0, 4.0)
    } else {
        (ARTILLERY_RADIUS, ARTILLERY_DAMAGE, ARTILLERY_COOLDOWN, ARTILLERY_SCATTER)
    }
}

// --- Air strike ---
pub const AIR_STRIKE_COST: f32 = 250.0;
pub const PRECISION_STRIKE_COST: f32 = 200.0;
pub const PRECISION_STRIKE_DELAY: u64 = secs(3.0);
pub const PRECISION_STRIKE_RADIUS: f32 = 2.2;
pub const PRECISION_STRIKE_DAMAGE: f32 = 110.0;

pub fn air_strike_cost(alt: bool) -> f32 {
    if alt { PRECISION_STRIKE_COST } else { AIR_STRIKE_COST }
}
pub const AIR_STRIKE_COOLDOWN: u64 = secs(90.0);
pub const AIR_STRIKE_DELAY: u64 = secs(4.0);
pub const AIR_STRIKE_IMPACTS: i32 = 5;
pub const AIR_STRIKE_SPACING: f32 = 2.0;
pub const AIR_STRIKE_RADIUS: f32 = 1.6;
pub const AIR_STRIKE_DAMAGE: f32 = 55.0;
/// The first air strike is available this long into the battle.
pub const AIR_STRIKE_FIRST: u64 = secs(60.0);

// --- Explosions and terrain ---
const EXPLOSION_SUPPRESSION: f32 = 40.0;
const WALL_BREAK_DAMAGE: f32 = 35.0;
const SANDBAG_BREAK_DAMAGE: f32 = 25.0;
const TREE_BURN_DAMAGE: f32 = 60.0;

// --- Loadout abilities ---
const GRENADE_RANGE: f32 = 5.0;
const GRENADE_COOLDOWN: f32 = 20.0;
const GRENADE_DELAY: u64 = secs(1.0);
const GRENADE_RADIUS: f32 = 1.5;
const GRENADE_DAMAGE: f32 = 40.0;
const MEDKIT_HEAL: f32 = 35.0;
const SMOKE_RADIUS: f32 = 2.0;
const SMOKE_SECONDS: f32 = 8.0;
const SMOKE_COOLDOWN: f32 = 40.0;
const ROCKET_COOLDOWN: f32 = 6.0;
const ROCKET_RADIUS: f32 = 1.2;
const ROCKET_DAMAGE: f32 = 50.0;
const BREACH_RANGE: f32 = 6.0;
const BREACH_COOLDOWN: f32 = 25.0;
const BREACH_DELAY: u64 = secs(1.5);
const BREACH_DAMAGE: f32 = 60.0;
const MINE_LAYER_COOLDOWN: f32 = 25.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GadgetKind {
    /// Explodes under the first enemy that steps on it.
    Mine,
    /// Goes off half a second after an enemy comes within 2 tiles; big blast.
    Bomb,
    /// Automatic gun: fast, accurate, tough, never scared.
    Turret,
    /// Lobs shells (3 s warning) at the biggest group of enemies its side
    /// can see, or wherever the King orders.
    Artillery,
}

pub struct GadgetStats {
    pub name: &'static str,
    pub letter: &'static str,
    pub cost: f32,
    /// Seconds for an engineer to build (a Toolkit is 60% faster).
    pub build_seconds: f32,
    pub health: f32,
    /// Fraction of bullet damage ignored.
    pub armor: f32,
    pub armor_class: ArmorClass,
    pub vision: f32,
}

impl GadgetKind {
    pub fn stats(self) -> &'static GadgetStats {
        match self {
            GadgetKind::Mine => &GadgetStats {
                name: "Mine",
                letter: "m",
                cost: 40.0,
                build_seconds: 3.0,
                health: 20.0,
                armor: 0.0,
                armor_class: ArmorClass::Light,
                vision: 0.0,
            },
            GadgetKind::Bomb => &GadgetStats {
                name: "Bomb",
                letter: "b",
                cost: 70.0,
                build_seconds: 4.0,
                health: 30.0,
                armor: 0.0,
                armor_class: ArmorClass::Light,
                vision: 0.0,
            },
            GadgetKind::Turret => &GadgetStats {
                name: "Turret",
                letter: "T",
                cost: 150.0,
                build_seconds: 20.0,
                health: 160.0,
                armor: 0.3,
                armor_class: ArmorClass::Heavy,
                vision: TURRET_RANGE,
            },
            GadgetKind::Artillery => &GadgetStats {
                name: "Artillery",
                letter: "Q",
                cost: 220.0,
                build_seconds: 30.0,
                health: 120.0,
                armor: 0.2,
                armor_class: ArmorClass::Medium,
                vision: 5.0,
            },
        }
    }

    pub fn is_trap(self) -> bool {
        matches!(self, GadgetKind::Mine | GadgetKind::Bomb)
    }
}

/// Bullet armor (fraction ignored) and armor class of any unit.
pub fn unit_armor(class: UnitClass) -> (f32, ArmorClass) {
    match class {
        UnitClass::Fighter(c) => (c.stats().armor, c.stats().armor_class),
        UnitClass::Vehicle => (super::vehicle::VEHICLE_ARMOR, ArmorClass::Heavy),
        UnitClass::King => (0.0, ArmorClass::Light),
        UnitClass::Gadget(k) => (k.stats().armor, k.stats().armor_class),
    }
}

#[derive(Component, Debug)]
pub struct Gadget {
    pub kind: GadgetKind,
    /// Seconds until it can fire again (turret, artillery).
    pub cooldown: f32,
    /// A bomb's countdown once triggered.
    pub fuse: Option<f32>,
    /// The second variant from the armory (shrapnel mine, heavy bomb,
    /// cannon turret, howitzer).
    pub alt: bool,
    pub last_shot_tick: Option<u64>,
}

/// Which sides know this gadget is here. Its owner always does.
#[derive(Component, Debug, Clone, Copy)]
pub struct Concealed {
    pub revealed_to: [bool; 2],
}

impl Concealed {
    pub fn known_to(&self, faction: Faction) -> bool {
        self.revealed_to[faction.index()]
    }
}

/// Tiles of enemy traps each side knows about. Fighters path around them.
#[derive(Resource, Default, Debug)]
pub struct KnownTraps(pub [HashSet<TilePos>; 2]);

impl KnownTraps {
    pub fn of(&self, faction: Faction) -> &HashSet<TilePos> {
        &self.0[faction.index()]
    }
}

/// When each side's next air strike is available.
#[derive(Resource, Debug)]
pub struct AirSupport {
    pub ready_tick: [u64; 2],
}

impl Default for AirSupport {
    fn default() -> Self {
        Self { ready_tick: [AIR_STRIKE_FIRST; 2] }
    }
}

impl AirSupport {
    pub fn ready(&self, faction: Faction, tick: u64) -> bool {
        tick >= self.ready_tick[faction.index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrikeKind {
    Artillery,
    Air,
    Grenade,
    Rocket,
    Breach,
}

/// Something about to explode: one or more impact points, each with the
/// tick it lands. Visible to anyone who can see the impact point.
#[derive(Component, Debug, Clone)]
pub struct Strike {
    pub faction: Faction,
    pub kind: StrikeKind,
    pub impacts: Vec<(TilePos, u64)>,
    pub radius: f32,
    pub damage: f32,
    /// Breaching charges can crack rock.
    pub breach: bool,
    /// The fighter who threw it (earns experience).
    pub source: Option<Entity>,
    pub launched_tick: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Explosion {
    pub at: TilePos,
    pub radius: f32,
    pub damage: f32,
    pub breach: bool,
    pub source: Option<Entity>,
    pub faction: Faction,
}

/// Explosions to resolve this tick.
#[derive(Resource, Default, Debug)]
pub struct PendingExplosions(pub Vec<Explosion>);

/// Explosions that happened, for the renderer. Output only.
#[derive(Resource, Default, Debug)]
pub struct BlastLog(pub Vec<(TilePos, f32)>);

pub fn spawn_gadget(
    commands: &mut Commands,
    index: &mut UnitIndex,
    faction: Faction,
    kind: GadgetKind,
    at: TilePos,
    alt: bool,
) -> Entity {
    let id = index.allocate();
    let stats = kind.stats();
    let mut revealed_to = [false; 2];
    revealed_to[faction.index()] = true;
    let mut e = commands.spawn((
        MatchEntity,
        Unit { id, class: UnitClass::Gadget(kind) },
        faction,
        at,
        Mover::new(0.0),
        SimInterp::at(at.to_world()),
        Transform::from_translation(at.to_world().extend(0.0)),
        Health::new(stats.health),
        Gadget { kind, cooldown: 0.0, fuse: None, alt, last_shot_tick: None },
        Concealed { revealed_to },
    ));
    let vision = if kind == GadgetKind::Turret { turret_profile(alt).range } else { stats.vision };
    if vision > 0.0 {
        e.insert((Vision { radius: vision }, Perception::default()));
    }
    let entity = e.id();
    index.entities.insert(id, entity);
    entity
}

/// Impact points of an air strike: a line of bombs through `target`, along
/// the direction the King is looking (from the King toward the target).
/// A precision strike is a single bomb on the target.
pub fn air_strike_impacts(from: TilePos, target: TilePos, tick: u64, alt: bool) -> Vec<(TilePos, u64)> {
    if alt {
        return vec![(target, tick + PRECISION_STRIKE_DELAY)];
    }
    let dir = Vec2::new((target.x - from.x) as f32, (target.y - from.y) as f32).normalize_or(Vec2::Y);
    (0..AIR_STRIKE_IMPACTS)
        .map(|k| {
            let off = dir * (k - AIR_STRIKE_IMPACTS / 2) as f32 * AIR_STRIKE_SPACING;
            let p = target.offset(off.x.round() as i32, off.y.round() as i32);
            let p = TilePos::new(p.x.clamp(0, crate::map::MAP_W - 1), p.y.clamp(0, crate::map::MAP_H - 1));
            (p, tick + AIR_STRIKE_DELAY + k as u64 * 4)
        })
        .collect()
}

pub fn air_strike(faction: Faction, from: TilePos, target: TilePos, tick: u64, alt: bool) -> Strike {
    Strike {
        faction,
        kind: StrikeKind::Air,
        impacts: air_strike_impacts(from, target, tick, alt),
        radius: if alt { PRECISION_STRIKE_RADIUS } else { AIR_STRIKE_RADIUS },
        damage: if alt { PRECISION_STRIKE_DAMAGE } else { AIR_STRIKE_DAMAGE },
        breach: false,
        source: None,
        launched_tick: tick,
    }
}

/// Every tick: enemies close enough (and with a line of sight) find hidden
/// gadgets. Also refreshes each side's list of known traps.
pub fn detect_gadgets(
    mut known: ResMut<KnownTraps>,
    viewers: Query<(&Faction, &TilePos, &Perception, Option<&Fighter>)>,
    mut gadgets: Query<(&Faction, &TilePos, &Gadget, &mut Concealed)>,
) {
    for (owner, pos, gadget, mut concealed) in &mut gadgets {
        let enemy = owner.opponent();
        if concealed.known_to(enemy) {
            continue;
        }
        let found = viewers.iter().any(|(f, vpos, perception, fighter)| {
            if *f != enemy {
                return false;
            }
            let reach = if gadget.kind.is_trap() {
                if fighter.is_some_and(|f| f.class == FighterClass::Engineer) {
                    TRAP_DETECT_ENGINEER
                } else {
                    TRAP_DETECT_OTHER
                }
            } else {
                STRUCTURE_DETECT
            };
            vpos.distance(*pos) <= reach && perception.sees(*pos)
        });
        if found {
            concealed.revealed_to[enemy.index()] = true;
        }
    }
    let mut next: [HashSet<TilePos>; 2] = Default::default();
    for (owner, pos, gadget, concealed) in &gadgets {
        let enemy = owner.opponent();
        if gadget.kind.is_trap() && concealed.known_to(enemy) {
            next[enemy.index()].insert(*pos);
        }
    }
    if known.0 != next {
        known.0 = next;
    }
}

/// Strikes whose time has come explode.
pub fn resolve_strikes(
    tick: Res<SimTick>,
    mut commands: Commands,
    mut pending: ResMut<PendingExplosions>,
    mut strikes: Query<(Entity, &mut Strike)>,
) {
    let mut order: Vec<Entity> = strikes.iter().map(|(e, _)| e).collect();
    order.sort();
    for entity in order {
        let Ok((_, mut strike)) = strikes.get_mut(entity) else { continue };
        let now = tick.0;
        let (due, later): (Vec<_>, Vec<_>) = strike.impacts.iter().partition(|(_, t)| *t <= now);
        for (at, _) in due {
            pending.0.push(Explosion {
                at,
                radius: strike.radius,
                damage: strike.damage,
                breach: strike.breach,
                source: strike.source,
                faction: strike.faction,
            });
        }
        strike.impacts = later;
        if strike.impacts.is_empty() {
            commands.entity(entity).despawn();
        }
    }
}

type TrapQuery<'a> = (Entity, &'a Unit, &'a Faction, &'a TilePos, &'a mut Gadget, &'a Concealed);

/// Mines go off under enemies; bombs start a short fuse when enemies come
/// near. A side that knows about a bomb only sets it off by stepping on it.
pub fn trigger_traps(
    mut commands: Commands,
    mut index: ResMut<UnitIndex>,
    mut pending: ResMut<PendingExplosions>,
    mut traps: Query<TrapQuery>,
    walkers: Query<(&Unit, &Faction, &TilePos), Without<Gadget>>,
) {
    let mut order: Vec<(UnitId, Entity)> = traps.iter().map(|t| (t.1.id, t.0)).collect();
    order.sort();
    for (id, entity) in order {
        let Ok((_, _, owner, pos, mut gadget, concealed)) = traps.get_mut(entity) else { continue };
        let enemy = owner.opponent();
        let explode = match gadget.kind {
            GadgetKind::Mine => {
                let stepped = walkers.iter().any(|(_, f, p)| *f == enemy && *p == *pos);
                stepped.then_some(trap_blast(GadgetKind::Mine, gadget.alt))
            }
            GadgetKind::Bomb => {
                if gadget.fuse.is_none() {
                    let reach = if concealed.known_to(enemy) { 0.5 } else { BOMB_TRIGGER };
                    if walkers.iter().any(|(_, f, p)| *f == enemy && p.distance(*pos) <= reach) {
                        gadget.fuse = Some(BOMB_FUSE);
                    }
                }
                match gadget.fuse.as_mut() {
                    Some(fuse) => {
                        *fuse -= DT;
                        (*fuse <= 0.0).then_some(trap_blast(GadgetKind::Bomb, gadget.alt))
                    }
                    None => None,
                }
            }
            _ => None,
        };
        if let Some((radius, damage)) = explode {
            pending.0.push(Explosion { at: *pos, radius, damage, breach: false, source: None, faction: *owner });
            index.entities.remove(&id);
            commands.entity(entity).despawn();
        }
    }
}

type TurretQuery<'a> =
    (Entity, &'a Unit, &'a Faction, &'a TilePos, &'a Perception, &'a mut Gadget, &'a mut Concealed);
type TargetQuery<'a> = (
    &'a Unit,
    &'a Faction,
    &'a TilePos,
    &'a Mover,
    &'a mut Health,
    Option<&'a mut Condition>,
    Option<&'a Traits>,
);

/// Turrets shoot the nearest enemy they can see. Firing gives them away.
pub fn fire_turrets(
    tick: Res<SimTick>,
    map: Res<GameMap>,
    mut rng: ResMut<SimRng>,
    mut log: ResMut<ShotLog>,
    mut turrets: Query<TurretQuery>,
    mut targets: Query<TargetQuery>,
) {
    let mut order: Vec<(UnitId, Entity)> = turrets.iter().map(|t| (t.1.id, t.0)).collect();
    order.sort();
    // (target, damage, suppression)
    let mut effects: Vec<(Entity, f32, f32)> = Vec::new();
    for (_, entity) in order {
        let Ok((_, _, faction, pos, perception, mut gadget, _)) = turrets.get_mut(entity) else { continue };
        if gadget.kind != GadgetKind::Turret {
            continue;
        }
        gadget.cooldown -= DT;
        if gadget.cooldown > 0.0 {
            continue;
        }
        let gun = turret_profile(gadget.alt);
        let mut best: Option<(f32, UnitId, Entity, TilePos, bool, ArmorClass, f32)> = None;
        for &t in &perception.seen_enemies {
            let Ok((u, _, p, m, _, _, _)) = targets.get(t) else { continue };
            let (id, tpos, moving, class) = (u.id, *p, !m.path.is_empty(), u.class);
            let d = pos.distance(tpos);
            if d > gun.range {
                continue;
            }
            let (armor, armor_class) = unit_armor(class);
            if best.is_none_or(|b| (d, id) < (b.0, b.1)) {
                best = Some((d, id, t, tpos, moving, armor_class, armor));
            }
        }
        let Some((d, _, target, tpos, moving, armor_class, armor)) = best else { continue };
        let mut p = gun.accuracy * (1.0 - 0.5 * (d / gun.range).powi(2));
        p *= match map.cover_against(tpos, *pos) {
            CoverLevel::None => 1.0,
            CoverLevel::Light => 0.7,
            CoverLevel::Heavy => 0.45,
        };
        if moving {
            p *= 0.85;
        }
        let hit = rng.0.random::<f32>() < p;
        let damage = if hit {
            gun.damage
                * rng.0.random_range(0.8..1.2)
                * damage_multiplier(gun.damage_type, armor_class)
                * (1.0 - armor)
        } else {
            0.0
        };
        effects.push((target, damage, gun.suppression * if hit { 1.5 } else { 1.0 }));
        gadget.cooldown = gun.interval * rng.0.random_range(0.9..1.1);
        gadget.last_shot_tick = Some(tick.0);
        log.0.push(ShotRecord { from: *pos, to: tpos, hit, shooter: UnitClass::Gadget(GadgetKind::Turret) });
        // Muzzle flashes give it away to whoever it shoots at.
        let enemy = faction.opponent();
        if let Ok((_, _, _, _, _, _, mut concealed)) = turrets.get_mut(entity) {
            concealed.revealed_to[enemy.index()] = true;
        }
    }
    for (target, damage, suppression) in effects {
        let Ok((_, _, _, _, mut health, cond, traits)) = targets.get_mut(target) else { continue };
        health.current -= damage;
        if let (Some(mut cond), Some(traits)) = (cond, traits) {
            let steadiness = 1.0 - 0.4 * traits.brav();
            cond.suppression = (cond.suppression + suppression * steadiness).min(100.0);
            cond.morale = (cond.morale - if damage > 0.0 { 4.0 } else { 1.0 } * steadiness).max(0.0);
        }
    }
}

type ArtilleryQuery<'a> = (Entity, &'a Unit, &'a Faction, &'a TilePos, &'a mut Gadget, &'a mut Concealed);

/// Artillery fires on its own whenever it's ready: at enemies its side can
/// see that are near its own troops (support fire), preferring the biggest
/// group, never aiming right on top of its own units. Shells scatter, so
/// they can hit, graze or miss. The King can't aim it.
pub fn fire_artillery(
    tick: Res<SimTick>,
    mut rng: ResMut<SimRng>,
    mut commands: Commands,
    mut guns: Query<ArtilleryQuery>,
    viewers: Query<(&Faction, &Perception)>,
    units: Query<(&Unit, &Faction, &TilePos), Without<Gadget>>,
) {
    let mut order: Vec<(UnitId, Entity)> =
        guns.iter().filter(|g| g.4.kind == GadgetKind::Artillery).map(|g| (g.1.id, g.0)).collect();
    order.sort();
    for (_, entity) in order {
        let Ok((_, _, faction, pos, mut gun, mut concealed)) = guns.get_mut(entity) else { continue };
        gun.cooldown -= DT;
        if gun.cooldown > 0.0 {
            continue;
        }
        let (radius, damage, cooldown, scatter) = artillery_profile(gun.alt);
        // Everything this side sees right now.
        let seen: HashSet<Entity> = viewers
            .iter()
            .filter(|(f, _)| *f == faction)
            .flat_map(|(_, p)| p.seen_enemies.iter().copied())
            .collect();
        let mut enemies: Vec<(UnitId, TilePos)> = seen
            .iter()
            .filter_map(|e| units.get(*e).ok())
            .filter(|(_, f, _)| **f != *faction)
            .map(|(u, _, p)| (u.id, *p))
            .collect();
        enemies.sort();
        let friends: Vec<TilePos> = units.iter().filter(|(_, f, _)| *f == faction).map(|(_, _, p)| *p).collect();
        let target = enemies
            .iter()
            .filter(|(_, p)| p.distance(*pos) <= ARTILLERY_RANGE)
            .filter(|(_, p)| {
                let nearest = friends.iter().map(|f| f.distance(*p)).fold(f32::MAX, f32::min);
                nearest <= ARTILLERY_SUPPORT_RANGE && nearest > ARTILLERY_SAFETY
            })
            .map(|(id, p)| (enemies.iter().filter(|(_, q)| q.distance(*p) <= radius).count(), *id, *p))
            .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
            .map(|(_, _, p)| p);
        let Some(target) = target else {
            gun.cooldown = 1.0; // look again in a second
            continue;
        };
        // Scatter: anywhere in a disk around the aim point.
        let angle = rng.0.random_range(0.0..std::f32::consts::TAU);
        let dist = scatter * rng.0.random::<f32>().sqrt();
        let at = target.offset((angle.cos() * dist).round() as i32, (angle.sin() * dist).round() as i32);
        let at = if at.in_bounds() { at } else { target };
        commands.spawn((
            MatchEntity,
            Strike {
                faction: *faction,
                kind: StrikeKind::Artillery,
                impacts: vec![(at, tick.0 + ARTILLERY_DELAY)],
                radius,
                damage,
                breach: false,
                source: None,
                launched_tick: tick.0,
            },
        ));
        gun.cooldown = cooldown;
        gun.last_shot_tick = Some(tick.0);
        // The boom gives its position away.
        concealed.revealed_to[faction.opponent().index()] = true;
    }
}

type BlastTarget<'a> = (
    Entity,
    &'a Unit,
    &'a Faction,
    &'a TilePos,
    &'a mut Health,
    Option<&'a mut Condition>,
    Option<&'a Traits>,
);

/// Explosions: damage and pin down everyone in the radius, and break
/// terrain.
#[allow(clippy::too_many_arguments)]
pub fn resolve_explosions(
    mut commands: Commands,
    mut map: ResMut<GameMap>,
    mut pending: ResMut<PendingExplosions>,
    mut blasts: ResMut<BlastLog>,
    mut xp: ResMut<XpLog>,
    mut units: Query<BlastTarget>,
    parked: Query<(Entity, &TilePos), With<ParkedVehicle>>,
) {
    if pending.0.is_empty() {
        return;
    }
    let explosions = std::mem::take(&mut pending.0);
    let mut order: Vec<(UnitId, Entity)> = units.iter().map(|u| (u.1.id, u.0)).collect();
    order.sort();
    for ex in explosions {
        blasts.0.push((ex.at, ex.radius));
        for &(_, entity) in &order {
            let Ok((_, unit, faction, pos, mut health, cond, traits)) = units.get_mut(entity) else { continue };
            let d = pos.distance(ex.at);
            if d > ex.radius || health.current <= 0.0 {
                continue;
            }
            let falloff = 1.0 - 0.5 * d / ex.radius;
            let (armor, armor_class) = unit_armor(unit.class);
            // Blasts get around bullet armor better than bullets do.
            let damage =
                ex.damage * falloff * damage_multiplier(DamageType::Explosive, armor_class) * (1.0 - 0.5 * armor);
            let was_alive = health.current > 0.0;
            health.current -= damage;
            if let (Some(mut cond), Some(traits)) = (cond, traits) {
                let steadiness = 1.0 - 0.4 * traits.brav();
                cond.suppression = (cond.suppression + EXPLOSION_SUPPRESSION * falloff * steadiness).min(100.0);
                cond.morale = (cond.morale - 8.0 * steadiness).max(0.0);
            }
            if let Some(src) = ex.source
                && *faction != ex.faction
            {
                let kill = if was_alive && health.current <= 0.0 { 4.0 } else { 0.0 };
                xp.0.push((src, damage / 10.0 + kill));
            }
        }
        // Terrain.
        let r = ex.radius.ceil() as i32;
        let mut changed = false;
        for dy in -r..=r {
            for dx in -r..=r {
                let t = ex.at.offset(dx, dy);
                let d = t.distance(ex.at);
                if !t.in_bounds() || d > ex.radius {
                    continue;
                }
                let dmg = ex.damage * (1.0 - 0.5 * d / ex.radius);
                let feature = map.tile(t).feature;
                let new = match feature {
                    Feature::Wall | Feature::Vehicle if dmg >= WALL_BREAK_DAMAGE => Some(Feature::LightCover),
                    Feature::Rock if ex.breach && dmg >= WALL_BREAK_DAMAGE => Some(Feature::LightCover),
                    Feature::LightCover if dmg >= SANDBAG_BREAK_DAMAGE && d > 0.0 || dmg >= SANDBAG_BREAK_DAMAGE * 1.6 => {
                        Some(Feature::None)
                    }
                    Feature::Trees if dmg >= TREE_BURN_DAMAGE => Some(Feature::None),
                    _ => None,
                };
                if let Some(new) = new {
                    // Rubble from a wall still gives light cover; a crater
                    // where sandbags were is open ground.
                    map.tile_mut(t).feature = new;
                    map.terrain_log.push(t);
                    changed = true;
                    if feature == Feature::Vehicle {
                        for (e, p) in &parked {
                            if *p == t {
                                commands.entity(e).despawn();
                            }
                        }
                    }
                }
            }
        }
        if changed {
            map.version += 1;
        }
    }
}

/// Smoke clears after a while.
pub fn update_smoke(tick: Res<SimTick>, mut map: ResMut<GameMap>) {
    if map.smoke.is_empty() {
        return;
    }
    let before = map.smoke.len();
    let now = tick.0;
    map.smoke.retain(|_, until| *until > now);
    if map.smoke.len() != before {
        map.version += 1;
    }
}

/// Mark a loadout as known to the enemy.
fn reveal(equipment: &mut Equipment, faction: Faction) {
    equipment.known_to[faction.opponent().index()] = true;
}

type AbilityUser<'a> = (
    Entity,
    &'a Unit,
    &'a Faction,
    &'a TilePos,
    &'a Fighter,
    &'a Perception,
    &'a Condition,
    &'a mut Equipment,
);

/// Loadout specials: grenades, medkits, smoke, rockets, breaching charges.
/// Using one gives it away to the enemy (if they can see it happen).
#[allow(clippy::too_many_arguments)]
pub fn use_abilities(
    tick: Res<SimTick>,
    mut map: ResMut<GameMap>,
    mut rng: ResMut<SimRng>,
    mut commands: Commands,
    platoons: Query<&Platoon>,
    others: Query<(&Unit, &Faction, &TilePos)>,
    viewers: Query<(&Faction, &Perception)>,
    mut fighters: Query<AbilityUser>,
    mut healths: Query<&mut Health, With<Fighter>>,
) {
    let mut order: Vec<(UnitId, Entity)> = fighters.iter().map(|f| (f.1.id, f.0)).collect();
    order.sort();
    for (_, entity) in order {
        let Ok((_, _, faction, pos, fighter, perception, cond, mut eq)) = fighters.get_mut(entity) else { continue };
        eq.cooldown -= DT;
        if eq.cooldown > 0.0 || cond.decision == Decision::Retreating {
            continue;
        }
        let seen_by_enemy =
            || viewers.iter().any(|(f, p)| *f != *faction && p.seen_enemies.contains(&entity));
        let enemies: Vec<(Entity, UnitClass, TilePos)> = perception
            .seen_enemies
            .iter()
            .filter_map(|e| others.get(*e).ok().map(|(u, _, p)| (*e, u.class, *p)))
            .collect();
        let strike = |kind, at: TilePos, delay: u64, radius: f32, damage: f32, breach: bool| Strike {
            faction: *faction,
            kind,
            impacts: vec![(at, tick.0 + delay)],
            radius,
            damage,
            breach,
            source: Some(entity),
            launched_tick: tick.0,
        };
        match eq.loadout {
            Loadout::Grenades => {
                // At the nearest dug-in enemy in throwing range.
                let target = enemies
                    .iter()
                    .filter(|(_, _, p)| p.distance(*pos) <= GRENADE_RANGE && p.distance(*pos) > GRENADE_RADIUS)
                    .filter(|(_, _, p)| map.cover_against(*p, *pos) != CoverLevel::None)
                    .min_by(|a, b| a.2.distance(*pos).total_cmp(&b.2.distance(*pos)).then(a.2.cmp(&b.2)));
                if let Some(&(_, _, at)) = target {
                    commands.spawn((MatchEntity, strike(StrikeKind::Grenade, at, GRENADE_DELAY, GRENADE_RADIUS, GRENADE_DAMAGE, false)));
                    eq.cooldown = GRENADE_COOLDOWN;
                    reveal(&mut eq, *faction);
                }
            }
            Loadout::Medkit if eq.charges > 0 => {
                // Patch up the most hurt platoon-mate within reach (or self).
                let platoon = platoons.get(fighter.platoon).ok();
                let candidates: Vec<Entity> = platoon.map_or(vec![entity], |p| p.members.clone());
                let mut best: Option<(f32, Entity)> = None;
                for m in candidates {
                    let Ok((_, _, p)) = others.get(m) else { continue };
                    if p.distance(*pos) > 2.0 {
                        continue;
                    }
                    let Ok(h) = healths.get(m) else { continue };
                    if h.frac() < 0.5 && best.is_none_or(|b| h.frac() < b.0) {
                        best = Some((h.frac(), m));
                    }
                }
                if let Some((_, m)) = best
                    && let Ok(mut h) = healths.get_mut(m)
                {
                    h.current = (h.current + MEDKIT_HEAL).min(h.max);
                    eq.charges -= 1;
                    eq.cooldown = 5.0;
                    if seen_by_enemy() {
                        reveal(&mut eq, *faction);
                    }
                }
            }
            Loadout::SmokeKit => {
                let caught = cond.suppression >= PINNED || (cond.suppression >= SUPPRESSED && cond.cover == CoverLevel::None);
                if caught {
                    let r = SMOKE_RADIUS.ceil() as i32;
                    let until = tick.0 + secs(SMOKE_SECONDS);
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let t = pos.offset(dx, dy);
                            if t.in_bounds() && t.distance(*pos) <= SMOKE_RADIUS {
                                map.smoke.insert(t, until);
                            }
                        }
                    }
                    map.version += 1;
                    eq.cooldown = SMOKE_COOLDOWN;
                    if seen_by_enemy() {
                        reveal(&mut eq, *faction);
                    }
                }
            }
            Loadout::Rockets => {
                // Armored targets and structures, or someone behind a wall.
                let range = fighter.class.stats().range;
                let target = enemies
                    .iter()
                    .filter(|(_, _, p)| p.distance(*pos) <= range && p.distance(*pos) > ROCKET_RADIUS + 0.5)
                    .filter(|(_, class, p)| {
                        unit_armor(*class).1 == ArmorClass::Heavy
                            || matches!(class, UnitClass::Gadget(_))
                            || map.cover_against(*p, *pos) == CoverLevel::Heavy
                    })
                    .min_by(|a, b| a.2.distance(*pos).total_cmp(&b.2.distance(*pos)).then(a.2.cmp(&b.2)));
                if let Some(&(_, _, at)) = target {
                    let p = 0.75 * (1.0 - 0.3 * at.distance(*pos) / range);
                    let at = if rng.0.random::<f32>() < p {
                        at
                    } else {
                        let off = at.offset(rng.0.random_range(-2..=2), rng.0.random_range(-2..=2));
                        if off.in_bounds() { off } else { at }
                    };
                    commands.spawn((MatchEntity, strike(StrikeKind::Rocket, at, 2, ROCKET_RADIUS, ROCKET_DAMAGE, false)));
                    eq.cooldown = ROCKET_COOLDOWN;
                    reveal(&mut eq, *faction);
                }
            }
            Loadout::BreachingCharge => {
                // Blow the obstacle an enemy is sheltering behind.
                let target = enemies
                    .iter()
                    .filter(|(_, _, p)| p.distance(*pos) <= BREACH_RANGE)
                    .filter_map(|(_, _, p)| sheltering_obstacle(&map, *p, *pos))
                    .min_by(|a, b| a.distance(*pos).total_cmp(&b.distance(*pos)).then(a.cmp(b)));
                if let Some(at) = target {
                    commands.spawn((MatchEntity, strike(StrikeKind::Breach, at, BREACH_DELAY, 1.3, BREACH_DAMAGE, true)));
                    eq.cooldown = BREACH_COOLDOWN;
                    reveal(&mut eq, *faction);
                }
            }
            _ => {}
        }
    }
}

/// The solid tile next to `target` that shields it from `from`, if any.
pub fn sheltering_obstacle(map: &GameMap, target: TilePos, from: TilePos) -> Option<TilePos> {
    let toward = Vec2::new((from.x - target.x) as f32, (from.y - target.y) as f32).normalize_or_zero();
    crate::map::DIRS8
        .iter()
        .map(|&(dx, dy)| (target.offset(dx, dy), Vec2::new(dx as f32, dy as f32).normalize()))
        .filter(|(n, _)| n.in_bounds() && map.tile(*n).feature.is_solid())
        .max_by(|a, b| a.1.dot(toward).total_cmp(&b.1.dot(toward)))
        .filter(|(_, d)| d.dot(toward) >= 0.7)
        .map(|(n, _)| n)
}

type EngineerQuery<'a> = (
    Entity,
    &'a Unit,
    &'a Faction,
    &'a TilePos,
    &'a Fighter,
    &'a Mover,
    &'a Perception,
    &'a mut Condition,
    &'a mut Equipment,
);

/// Build speed of one engineer (a Toolkit works 60% faster).
pub fn work_speed(loadout: Loadout) -> f32 {
    if loadout == Loadout::Toolkit { 1.6 } else { 1.0 }
}

/// Engineers at work: building at the site, defusing next to a trap.
/// Engineers from the same platoon help with a build: each one working
/// next to the site adds its own speed, so two engineers build twice as
/// fast, three three times as fast. Also
/// hands out their own jobs: defusing known traps nearby, and (Mine
/// layers) planting mines in front of a holding platoon.
#[allow(clippy::too_many_arguments)]
pub fn update_engineers(
    map: Res<GameMap>,
    mut commands: Commands,
    mut index: ResMut<UnitIndex>,
    mut supply: ResMut<Supply>,
    known: Res<KnownTraps>,
    armories: Res<Armories>,
    gadgets: Query<(Entity, &Unit, &Faction, &TilePos, &Gadget)>,
    viewers: Query<(&Faction, &Perception)>,
    platoons: Query<&Platoon>,
    mut engineers: Query<EngineerQuery>,
) {
    let mut order: Vec<(UnitId, Entity)> = engineers
        .iter()
        .filter(|e| e.4.class == FighterClass::Engineer)
        .map(|e| (e.1.id, e.0))
        .collect();
    order.sort();
    let mut claimed: HashSet<Entity> = engineers
        .iter()
        .filter_map(|e| e.7.work.and_then(|w| if let WorkKind::Defuse(t) = w.kind { Some(t) } else { None }))
        .collect();
    let mut sites: HashSet<TilePos> = gadgets.iter().map(|g| *g.3).collect();
    for e in engineers.iter() {
        if let Some(Work { kind: WorkKind::Build(_), tile, .. }) = e.7.work {
            sites.insert(tile);
        }
    }
    // Help each lead engineer is getting right now: helpers next to the
    // site, standing still and not pinned.
    let mut help: std::collections::HashMap<Entity, f32> = std::collections::HashMap::new();
    for e in engineers.iter() {
        let (pos, mover, cond, eq) = (e.3, e.5, &e.7, &e.8);
        if let Some(Work { kind: WorkKind::Assist(lead), tile, .. }) = cond.work
            && pos.distance(tile) <= 1.5
            && mover.path.is_empty()
            && cond.suppression < PINNED
            && cond.decision != Decision::Retreating
        {
            *help.entry(lead).or_default() += work_speed(eq.loadout);
        }
    }
    // Leads and their jobs, so helpers know when to stop.
    let leads: std::collections::HashMap<Entity, TilePos> = engineers
        .iter()
        .filter_map(|e| match e.7.work {
            Some(Work { kind: WorkKind::Build(_), tile, .. }) => Some((e.0, tile)),
            _ => None,
        })
        .collect();

    for (_, entity) in order {
        let Ok((_, _, faction, pos, fighter, mover, perception, mut cond, mut eq)) = engineers.get_mut(entity) else {
            continue;
        };
        let speed = work_speed(eq.loadout);
        let seen_by_enemy = viewers.iter().any(|(f, p)| *f != *faction && p.seen_enemies.contains(&entity));

        if let Some(mut work) = cond.work {
            let cancel = |work: &Work, supply: &mut Supply| {
                supply.0[faction.index()] += work.paid;
            };
            if cond.decision == Decision::Retreating {
                cancel(&work, &mut supply);
                cond.work = None;
                continue;
            }
            match work.kind {
                WorkKind::Build(kind) => {
                    let blocked = gadgets.iter().any(|g| *g.3 == work.tile) || !map.tile(work.tile).passable();
                    if blocked {
                        cancel(&work, &mut supply);
                        cond.work = None;
                        continue;
                    }
                    let at_site = *pos == work.tile && mover.path.is_empty();
                    if at_site {
                        if cond.suppression < PINNED {
                            let team = speed + help.get(&entity).copied().unwrap_or(0.0);
                            work.progress += DT * team / kind.stats().build_seconds;
                        }
                    } else if mover.path.is_empty() && cond.decision == Decision::Building {
                        work.idle += DT; // can't get there
                    }
                    if work.idle > 4.0 {
                        cancel(&work, &mut supply);
                        cond.work = None;
                        continue;
                    }
                    if work.progress >= 1.0 {
                        let alt = armories.of(*faction).gadget_variant(kind).unwrap_or(false);
                        spawn_gadget(&mut commands, &mut index, *faction, kind, work.tile, alt);
                        cond.work = None;
                        if matches!(eq.loadout, Loadout::Toolkit | Loadout::MineLayer) && seen_by_enemy {
                            reveal(&mut eq, *faction);
                        }
                    } else {
                        cond.work = Some(work);
                    }
                }
                WorkKind::Assist(lead) => {
                    // Done when the lead's build is finished or abandoned.
                    if leads.get(&lead) != Some(&work.tile) {
                        cond.work = None;
                        continue;
                    }
                    if pos.distance(work.tile) > 1.5 && mover.path.is_empty() && cond.decision == Decision::Building {
                        work.idle += DT;
                    }
                    cond.work = if work.idle > 4.0 { None } else { Some(work) };
                }
                WorkKind::Defuse(trap) => {
                    let Ok((_, trap_unit, _, _, _)) = gadgets.get(trap) else {
                        cond.work = None;
                        continue;
                    };
                    let near = pos.distance(work.tile) <= 1.5 && mover.path.is_empty();
                    if near {
                        if cond.suppression < PINNED {
                            work.progress += DT * speed / DEFUSE_SECONDS;
                        }
                    } else if mover.path.is_empty() && cond.decision == Decision::Defusing {
                        work.idle += DT;
                    }
                    if work.idle > 4.0 {
                        cond.work = None;
                        continue;
                    }
                    if work.progress >= 1.0 {
                        index.entities.remove(&trap_unit.id);
                        commands.entity(trap).despawn();
                        cond.work = None;
                    } else {
                        cond.work = Some(work);
                    }
                }
            }
            continue;
        }

        // No job: look for one.
        let busy = matches!(cond.decision, Decision::Retreating | Decision::Frozen | Decision::Dodging)
            || cond.suppression >= SUPPRESSED;
        if busy {
            continue;
        }
        let trap = known
            .of(*faction)
            .iter()
            .filter(|t| t.distance(*pos) <= DEFUSE_SEARCH)
            .filter_map(|t| {
                gadgets
                    .iter()
                    .find(|g| *g.3 == *t && *g.2 != *faction && g.4.kind.is_trap())
                    .map(|g| (g.0, *t))
            })
            .filter(|(e, _)| !claimed.contains(e))
            .min_by(|a, b| a.1.distance(*pos).total_cmp(&b.1.distance(*pos)).then(a.1.cmp(&b.1)));
        if let Some((trap, tile)) = trap {
            claimed.insert(trap);
            cond.work = Some(Work { kind: WorkKind::Defuse(trap), tile, progress: 0.0, paid: 0.0, idle: 0.0 });
            continue;
        }
        // Mine layers plant mines ahead of a platoon that's holding a spot.
        let holding = platoons.get(fighter.platoon).is_ok_and(|p| p.mission == Mission::Overwatch && p.pending.is_none());
        if eq.loadout == Loadout::MineLayer
            && eq.charges > 0
            && eq.cooldown <= 0.0
            && holding
            && cond.decision == Decision::Holding
            && perception.seen_enemies.is_empty()
        {
            let enemy_home = map.spawn(faction.opponent());
            let dir = Vec2::new((enemy_home.x - pos.x) as f32, (enemy_home.y - pos.y) as f32).normalize_or_zero() * 4.0;
            let ahead = pos.offset(dir.x.round() as i32, dir.y.round() as i32);
            if let Some(tile) = nearest_free_tile_where(&map, &sites, ahead, |t| t.distance(ahead) <= 3.0) {
                sites.insert(tile);
                eq.charges -= 1;
                eq.cooldown = MINE_LAYER_COOLDOWN;
                cond.work = Some(Work { kind: WorkKind::Build(GadgetKind::Mine), tile, progress: 0.0, paid: 0.0, idle: 0.0 });
            }
        }
    }
}

/// A free tile next to `tile` for an engineer to work from, nearest to
/// `from`.
pub fn work_spot(map: &GameMap, tile: TilePos, from: TilePos, avoid: &HashSet<TilePos>) -> Option<TilePos> {
    crate::map::DIRS8
        .iter()
        .map(|&(dx, dy)| tile.offset(dx, dy))
        .filter(|n| n.in_bounds() && map.tile(*n).passable() && !avoid.contains(n))
        .min_by(|a, b| a.distance(from).total_cmp(&b.distance(from)).then(a.cmp(b)))
}

/// Impact points a fighter should get away from: all of its own side's
/// strikes, and enemy strikes it can see.
pub fn danger_zones(strikes: &Query<&Strike>) -> Vec<(TilePos, f32, Faction)> {
    let mut out: Vec<(TilePos, f32, Faction)> =
        strikes.iter().flat_map(|s| s.impacts.iter().map(move |(t, _)| (*t, s.radius, s.faction))).collect();
    out.sort_by_key(|z| z.0);
    out
}

/// The nearest walkable tile clear of every danger zone.
pub fn escape_tile(
    map: &GameMap,
    from: TilePos,
    zones: &[(TilePos, f32)],
    taken: &HashSet<TilePos>,
) -> Option<TilePos> {
    let r = 6;
    let mut best: Option<(f32, TilePos)> = None;
    for dy in -r..=r {
        for dx in -r..=r {
            let t = from.offset(dx, dy);
            if !t.in_bounds() || !map.tile(t).passable() || taken.contains(&t) {
                continue;
            }
            if zones.iter().any(|(z, radius)| z.distance(t) <= radius + 0.8) {
                continue;
            }
            let d = from.distance(t);
            if best.is_none_or(|b| (d, t) < b) {
                best = Some((d, t));
            }
        }
    }
    best.map(|b| b.1)
}


/// Battle commands for gadgets: engineer build orders (sent as signals)
/// and air strikes. Only for what the side brought in its armory.
#[allow(clippy::too_many_arguments)]
pub fn process_gadget_commands(
    mut commands: Commands,
    mut queue: ResMut<super::CommandQueue>,
    tick: Res<SimTick>,
    index: Res<UnitIndex>,
    map: Res<GameMap>,
    fog: Option<Res<crate::fog::FogOfWar>>,
    mut supply: ResMut<Supply>,
    mut air: ResMut<AirSupport>,
    mut flight: ResMut<super::platoon::SignalsInFlight>,
    armories: Res<Armories>,
    platoons: Query<(&Faction, &Platoon)>,
    fighters: Query<&Fighter>,
    kings: Query<(&Unit, &Faction, &TilePos), Without<Gadget>>,
    sites: Query<&TilePos, With<Gadget>>,
) {
    use super::SimCommand;
    let explored = |f: Faction, t: TilePos| t.in_bounds() && fog.as_ref().is_none_or(|fog| fog.is_explored(f, t));
    let king_pos = |f: Faction| {
        kings.iter().find(|(u, uf, _)| u.class == UnitClass::King && **uf == f).map(|(_, _, p)| *p)
    };
    let cmds = queue.take_matching(|c| {
        matches!(c, SimCommand::Build { .. } | SimCommand::AirStrike { .. })
    });
    for command in cmds {
        match command {
            SimCommand::Build { issuer, platoon, kind, target } => {
                let Some(&entity) = index.platoons.get(&platoon) else { continue };
                let Ok((faction, p)) = platoons.get(entity) else { continue };
                let has_engineer =
                    p.members.iter().any(|m| fighters.get(*m).is_ok_and(|f| f.class == FighterClass::Engineer));
                let occupied = sites.iter().any(|gp| *gp == target);
                if *faction != issuer
                    || !has_engineer
                    || !explored(issuer, target)
                    || !map.tile(target).passable()
                    || occupied
                {
                    debug!("{issuer:?} build order rejected");
                    continue;
                }
                let Some(cost) = armories.of(issuer).gadget_price(kind) else {
                    debug!("{issuer:?} didn't bring {kind:?}");
                    continue;
                };
                if supply.0[issuer.index()] < cost {
                    continue;
                }
                supply.0[issuer.index()] -= cost;
                flight.0.push(super::platoon::SignalInFlight {
                    issuer,
                    platoon: entity,
                    mission: p.pending.map_or(p.mission, |s| s.mission),
                    target,
                    from: king_pos(issuer).unwrap_or(target),
                    sent_tick: tick.0,
                    arrive_tick: tick.0 + super::platoon::SIGNAL_TRAVEL_TICKS,
                    build: Some((kind, cost)),
                });
            }
            SimCommand::AirStrike { issuer, target } => {
                let i = issuer.index();
                let Some(alt) = armories.of(issuer).variant(Machine::AirStrike) else { continue };
                let cost = air_strike_cost(alt);
                if !explored(issuer, target) || !air.ready(issuer, tick.0) || supply.0[i] < cost {
                    continue;
                }
                supply.0[i] -= cost;
                air.ready_tick[i] = tick.0 + AIR_STRIKE_COOLDOWN;
                let from = king_pos(issuer).unwrap_or(target);
                commands.spawn((MatchEntity, air_strike(issuer, from, target, tick.0, alt)));
            }
            _ => {}
        }
    }
}
