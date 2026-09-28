//! Shooting, damage, morale and deaths.
//!
//! Fighters fire on their own at enemies they can see and reach. Hit
//! chance combines the shooter's class and Accuracy trait with its current
//! condition (moving, suppressed, injured) and the target's situation
//! (cover, moving, height), plus the mission matchup:
//!
//! - Overwatch firing at an advancing Assault: reaction fire bonus.
//! - Assault firing at a Reposition caught in the open: bonus.
//! - Anyone firing at a sprinting Reposition: penalty (dodging). This is
//!   what lets Reposition beat Overwatch.

use std::collections::HashMap;

use bevy::prelude::*;
use rand::Rng;

use super::classes::{
    ArmorClass, DamageType, Equipment, FighterClass, Loadout, Traits, damage_multiplier, suppression_multiplier,
    weapon_damage_type,
};
use super::fighter::{Condition, Decision, Fighter, Health, XpLog, morale_baseline};
use super::gadgets::unit_armor;
use super::perception::Perception;
use super::platoon::{Mission, Platoon};
use super::{DT, Mover, SimRng, Unit, UnitClass, UnitId, UnitIndex};
use crate::faction::Faction;
use crate::map::{CoverLevel, GameMap, TilePos};

/// A shot, recorded for the renderer (tracers). Output only.
#[derive(Clone, Copy, Debug)]
pub struct ShotRecord {
    pub from: TilePos,
    pub to: TilePos,
    pub hit: bool,
    pub shooter: UnitClass,
}

#[derive(Resource, Default)]
pub struct ShotLog(pub Vec<ShotRecord>);

/// Set when the match ends (a King dies, or time runs out). The
/// simulation stops. `winner` is None for a draw.
#[derive(Resource, Default)]
pub struct MatchOutcome {
    pub ended: bool,
    pub winner: Option<Faction>,
    pub reason: String,
}

/// Everything that affects one shot's hit chance.
#[derive(Clone, Copy, Debug)]
pub struct ShotContext {
    pub class: FighterClass,
    pub accuracy_trait: f32,
    pub shooter_moving: bool,
    pub shooter_suppression: f32,
    pub shooter_health: f32,
    pub shooter_mission: Option<Mission>,
    pub shooter_elevation: u8,
    pub target_cover: CoverLevel,
    pub target_moving: bool,
    pub target_mission: Option<Mission>,
    pub target_elevation: u8,
    pub distance: f32,
}

pub fn hit_chance(c: &ShotContext) -> f32 {
    let stats = c.class.stats();
    if c.distance > stats.range {
        return 0.0;
    }
    let mut p = stats.base_accuracy * (0.6 + 0.8 * c.accuracy_trait);

    // Falloff toward max range.
    p *= 1.0 - 0.5 * (c.distance / stats.range).powi(2);

    // Shooter's condition.
    if c.shooter_moving {
        p *= stats.moving_accuracy;
    }
    p *= 1.0 - c.shooter_suppression / 200.0;
    if c.shooter_health < 0.5 {
        p *= 0.75;
    }

    // Target's situation.
    p *= match c.target_cover {
        CoverLevel::None => 1.0,
        CoverLevel::Light => 0.7,
        CoverLevel::Heavy => 0.45,
    };
    if c.target_moving {
        p *= 0.85;
    }
    match c.shooter_elevation.cmp(&c.target_elevation) {
        std::cmp::Ordering::Greater => p *= 1.15,
        std::cmp::Ordering::Less => p *= 0.85,
        std::cmp::Ordering::Equal => {}
    }

    p *= mission_matchup(c);
    p.clamp(0.0, 0.95)
}

/// The rock-paper-scissors layer between missions.
pub fn mission_matchup(c: &ShotContext) -> f32 {
    use Mission::*;
    match (c.shooter_mission, c.target_mission) {
        // Reaction fire: Overwatch is waiting for exactly this.
        (Some(Overwatch), Some(Assault)) if c.target_moving => 1.35,
        // Caught in the open mid-reposition.
        (Some(Assault), Some(Reposition)) if c.target_cover == CoverLevel::None => 1.35,
        // Sprinting between cover, out of the watched lane.
        (_, Some(Reposition)) if c.target_moving => 0.6,
        _ => 1.0,
    }
}

#[derive(Clone, Copy)]
struct Snapshot {
    id: UnitId,
    pos: TilePos,
    moving: bool,
    mission: Option<Mission>,
    health: f32,
    armor: f32,
    armor_class: ArmorClass,
    is_king: bool,
}

type UnitQuery<'a> = (
    Entity,
    &'a Unit,
    &'a Faction,
    &'a TilePos,
    &'a Mover,
    Option<&'a Fighter>,
    Option<&'a Traits>,
    Option<&'a Perception>,
    &'a mut Health,
    Option<&'a mut Condition>,
    Option<&'a mut Equipment>,
);

pub fn fire_weapons(
    tick: Res<super::SimTick>,
    map: Res<GameMap>,
    mut rng: ResMut<SimRng>,
    mut log: ResMut<ShotLog>,
    mut xp: ResMut<XpLog>,
    platoons: Query<&Platoon>,
    mut units: Query<UnitQuery>,
) {
    let mission_of = |f: Option<&Fighter>| f.and_then(|f| platoons.get(f.platoon).ok()).map(|p| p.mission);

    let mut snaps: HashMap<Entity, Snapshot> = HashMap::new();
    for (e, unit, _, pos, mover, fighter, _, _, health, _, _) in &units {
        let (armor, armor_class) = unit_armor(unit.class);
        snaps.insert(
            e,
            Snapshot {
                id: unit.id,
                pos: *pos,
                moving: !mover.path.is_empty(),
                mission: mission_of(fighter),
                health: health.frac(),
                armor,
                armor_class,
                is_king: unit.class == UnitClass::King,
            },
        );
    }

    let mut shooters: Vec<(UnitId, Entity)> =
        snaps.iter().filter(|(_, s)| !s.is_king).map(|(e, s)| (s.id, *e)).collect();
    shooters.sort();

    // (shooter, target, damage, suppression, hit)
    let mut effects: Vec<(Entity, Entity, f32, f32, bool)> = Vec::new();

    for (_, entity) in shooters {
        let Ok((_, _, faction, pos, _, Some(fighter), Some(traits), Some(perception), _, Some(mut cond), mut equipment)) =
            units.get_mut(entity)
        else {
            continue;
        };
        let loadout = equipment.as_ref().map_or(Loadout::Rifle, |e| e.loadout);
        let damage_type = weapon_damage_type(fighter.class, loadout);
        cond.fire_cooldown -= DT;
        if cond.fire_cooldown > 0.0 || cond.decision == Decision::Retreating {
            continue;
        }
        let me = snaps[&entity];
        let stats = fighter.class.stats();
        if me.moving && stats.moving_accuracy == 0.0 {
            continue; // e.g. snipers can't shoot on the move
        }

        let mut options: Vec<(Entity, f32, f32)> = Vec::new(); // (target, hit chance, distance)
        for &target in &perception.seen_enemies {
            let Some(t) = snaps.get(&target) else { continue };
            let distance = pos.distance(t.pos);
            let ctx = ShotContext {
                class: fighter.class,
                accuracy_trait: traits.acc(),
                shooter_moving: me.moving,
                shooter_suppression: cond.suppression,
                shooter_health: me.health,
                shooter_mission: me.mission,
                shooter_elevation: map.tile(*pos).elevation,
                target_cover: map.cover_against(t.pos, *pos),
                target_moving: t.moving,
                target_mission: t.mission,
                target_elevation: map.tile(t.pos).elevation,
                distance,
            };
            let p = hit_chance(&ctx);
            if p > 0.0 {
                options.push((target, p, distance));
            }
        }
        if options.is_empty() {
            continue;
        }

        // Smart fighters pick the best target (likely hits, wounded
        // enemies, the King); less smart ones often just shoot the nearest.
        let iq = traits.iq();
        let pick = if rng.0.random::<f32>() < (1.0 - iq) * 0.5 {
            options.iter().min_by(|a, b| a.2.total_cmp(&b.2))
        } else {
            options.iter().max_by(|a, b| {
                let value = |o: &(Entity, f32, f32)| {
                    let t = &snaps[&o.0];
                    o.1 * (1.0 + 0.6 * iq * (1.0 - t.health)) + if t.is_king { 0.3 * iq } else { 0.0 }
                };
                value(a).total_cmp(&value(b)).then(snaps[&b.0].id.cmp(&snaps[&a.0].id))
            })
        };
        let Some(&(target, p, _)) = pick else { continue };

        // Repositioning fighters only take shots worth slowing down for.
        if me.mission == Some(Mission::Reposition) && me.moving && p < 0.35 {
            continue;
        }

        let hit = rng.0.random::<f32>() < p;
        let t = snaps[&target];
        let damage = if hit {
            stats.damage
                * rng.0.random_range(0.8..1.2)
                * damage_multiplier(damage_type, t.armor_class)
                * (1.0 - t.armor)
        } else {
            0.0
        };
        let suppression = stats.suppression * suppression_multiplier(damage_type) * if hit { 1.5 } else { 1.0 };
        effects.push((entity, target, damage, suppression, hit));

        // A machine gun or armor-piercing hits give the loadout away.
        if damage_type != DamageType::Standard
            && (damage_type == DamageType::Suppression || hit)
            && let Some(eq) = equipment.as_mut()
        {
            eq.known_to[faction.opponent().index()] = true;
        }

        let ready_bonus = if me.mission == Some(Mission::Overwatch) && !me.moving { 0.8 } else { 1.0 };
        // Carrying rockets means less machine-gun ammo: slower fire.
        let loadout_mult = if loadout == Loadout::Rockets { 1.3 } else { 1.0 };
        cond.fire_cooldown = stats.fire_interval * rng.0.random_range(0.9..1.1) * ready_bonus * loadout_mult;
        cond.fire_target = Some(target);
        cond.last_shot_tick = Some(tick.0);
        log.0.push(ShotRecord { from: *pos, to: t.pos, hit, shooter: UnitClass::Fighter(fighter.class) });
    }

    for (shooter, target, damage, suppression, hit) in effects {
        let Ok((_, _, _, _, _, _, traits, _, mut health, cond, _)) = units.get_mut(target) else { continue };
        let was_alive = health.current > 0.0;
        health.current -= damage;
        // Experience: damage dealt, plus a bonus for a kill.
        let kill = if was_alive && health.current <= 0.0 { 4.0 } else { 0.0 };
        if damage > 0.0 {
            xp.0.push((shooter, damage / 10.0 + kill));
        }
        if let (Some(mut cond), Some(traits)) = (cond, traits) {
            let steadiness = 1.0 - 0.4 * traits.brav();
            cond.suppression = (cond.suppression + suppression * steadiness).min(100.0);
            cond.morale = (cond.morale - if hit { 5.0 } else { 1.0 } * steadiness).max(0.0);
        }
    }
}

/// Suppression wears off; morale drifts toward a level set by bravery,
/// health, incoming fire and whether the platoon leader is nearby.
pub fn update_conditions(
    territory: Option<Res<super::economy::Territory>>,
    platoons: Query<&Platoon>,
    leaders: Query<(&TilePos, &Traits)>,
    mut fighters: Query<(&TilePos, &Faction, &Fighter, &Traits, &Health, &mut Condition)>,
) {
    for (pos, faction, fighter, traits, health, mut cond) in &mut fighters {
        let decay = if cond.cover != CoverLevel::None { 28.0 } else { 18.0 };
        cond.suppression = (cond.suppression - decay * DT).max(0.0);

        let mut target = morale_baseline(traits) - cond.suppression * 0.3 - (1.0 - health.frac()) * 20.0
            + territory.as_ref().map_or(0.0, |t| t.morale_bonus(*faction));
        if let Some((leader_pos, leader_traits)) = platoons
            .get(fighter.platoon)
            .ok()
            .and_then(|p| p.leader)
            .and_then(|l| leaders.get(l).ok())
            && leader_pos.distance(*pos) <= 8.0
        {
            target += 10.0 * leader_traits.lead();
        }
        let rate = if target < cond.morale { 6.0 } else { 3.0 };
        let step = (target - cond.morale).clamp(-rate * DT, rate * DT);
        cond.morale = (cond.morale + step).clamp(0.0, 100.0);
    }
}

/// Remove the dead. Nearby allies lose morale (more if it was their
/// leader). A dead King ends the match. A destroyed vehicle is handed to
/// `update_vehicles`, which bails its passengers out.
#[allow(clippy::too_many_arguments)]
pub fn remove_dead(
    mut commands: Commands,
    mut index: ResMut<UnitIndex>,
    mut outcome: ResMut<MatchOutcome>,
    platoons: Query<&Platoon>,
    dead: Query<(Entity, &Unit, &Faction, &TilePos, &Health, Option<&Fighter>)>,
    mut vehicles: Query<&mut super::vehicle::Vehicle>,
    mut fighters: Query<(&TilePos, &Faction, &Fighter, &Traits, &mut Condition)>,
) {
    for (entity, unit, faction, pos, health, fighter) in &dead {
        if health.current > 0.0 {
            continue;
        }
        if let Ok(mut vehicle) = vehicles.get_mut(entity) {
            vehicle.destroyed = true;
            continue;
        }
        commands.entity(entity).despawn();
        index.entities.remove(&unit.id);
        if matches!(unit.class, UnitClass::Gadget(_)) {
            continue; // nobody mourns a turret
        }
        if unit.class == UnitClass::King {
            outcome.ended = true;
            outcome.winner = Some(match faction {
                Faction::Humans => Faction::Termbots,
                Faction::Termbots => Faction::Humans,
            });
            outcome.reason = format!("The {} King is down", faction.name());
            info!("{} King killed", faction.name());
            continue;
        }
        let was_leader = fighter
            .and_then(|f| platoons.get(f.platoon).ok().map(|p| (f.platoon, p.leader == Some(entity))))
            .filter(|(_, l)| *l)
            .map(|(p, _)| p);
        for (ally_pos, ally_faction, ally, traits, mut cond) in &mut fighters {
            if ally_faction != faction {
                continue;
            }
            let mut shock = 0.0;
            if ally_pos.distance(*pos) <= 8.0 {
                shock += 15.0;
            }
            if was_leader == Some(ally.platoon) {
                shock += 10.0;
            }
            cond.morale = (cond.morale - shock * (1.0 - 0.6 * traits.brav())).max(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ShotContext {
        ShotContext {
            class: FighterClass::Assaulter,
            accuracy_trait: 0.5,
            shooter_moving: false,
            shooter_suppression: 0.0,
            shooter_health: 1.0,
            shooter_mission: None,
            shooter_elevation: 0,
            target_cover: CoverLevel::None,
            target_moving: false,
            target_mission: None,
            target_elevation: 0,
            distance: 3.0,
        }
    }

    #[test]
    fn cover_moving_and_suppression_lower_hit_chance() {
        let base = hit_chance(&ctx());
        assert!(hit_chance(&ShotContext { target_cover: CoverLevel::Light, ..ctx() }) < base);
        assert!(
            hit_chance(&ShotContext { target_cover: CoverLevel::Heavy, ..ctx() })
                < hit_chance(&ShotContext { target_cover: CoverLevel::Light, ..ctx() })
        );
        assert!(hit_chance(&ShotContext { shooter_moving: true, ..ctx() }) < base);
        assert!(hit_chance(&ShotContext { shooter_suppression: 80.0, ..ctx() }) < base);
        assert!(hit_chance(&ShotContext { shooter_elevation: 1, ..ctx() }) > base);
        assert_eq!(hit_chance(&ShotContext { distance: 50.0, ..ctx() }), 0.0);
    }

    #[test]
    fn snipers_cannot_shoot_while_moving() {
        let c = ShotContext { class: FighterClass::Sniper, shooter_moving: true, ..ctx() };
        assert_eq!(hit_chance(&c), 0.0);
    }

    #[test]
    fn missions_form_a_loop() {
        let base = hit_chance(&ShotContext { target_moving: true, ..ctx() });
        // Overwatch beats a moving Assault.
        let ow = ShotContext {
            shooter_mission: Some(Mission::Overwatch),
            target_mission: Some(Mission::Assault),
            target_moving: true,
            ..ctx()
        };
        assert!(hit_chance(&ow) > base);
        // Reposition slips Overwatch.
        let rep = ShotContext { target_mission: Some(Mission::Reposition), ..ow };
        assert!(hit_chance(&rep) < base);
        // Assault catches Reposition in the open.
        let asl = ShotContext { shooter_mission: Some(Mission::Assault), ..rep };
        assert!(hit_chance(&asl) > base);
    }
}
