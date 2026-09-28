//! Fighter autonomy.
//!
//! Every fighter makes its own decisions, a few times per second, within
//! its platoon's mission. It scores its options (advance, fight, take
//! cover, hold) from what it sees and how it's doing, tilted by its traits:
//!
//! - Tactics: how much it values cover, and how well it picks spots.
//! - IQ: how often it re-thinks and how erratic its choices are.
//! - Bravery: willingness to push forward under fire.
//! - Accuracy: willingness to stand and shoot.
//!
//! Overrides come first: broken fighters flee, pinned fighters stay down,
//! and a frozen platoon stops following orders. Shooting is separate (see
//! `combat.rs`): fighters shoot at what they see whenever they can,
//! including on the move.

use std::collections::HashSet;

use bevy::prelude::*;
use rand::Rng;

use super::classes::Traits;
use super::fighter::{BROKEN, Condition, Decision, Fighter, Health, PINNED};
use super::perception::Perception;
use super::platoon::{Mission, Platoon, slot_tile};
use super::{Mover, SimRng, SimTick, Unit, set_destination};
use crate::faction::Faction;
use crate::map::{CoverLevel, GameMap, TilePos};

fn cover_value(level: CoverLevel) -> f32 {
    match level {
        CoverLevel::None => 0.0,
        CoverLevel::Light => 1.0,
        CoverLevel::Heavy => 2.0,
    }
}

/// Best free tile within `radius` of `center`, weighing cover, distance
/// from `center` and distance from the fighter's current position.
#[allow(clippy::too_many_arguments)]
fn best_tile_near(
    map: &GameMap,
    center: TilePos,
    radius: i32,
    from: TilePos,
    taken: &HashSet<TilePos>,
    cover_weight: f32,
    height_weight: f32,
    from_weight: f32,
) -> Option<TilePos> {
    let mut best: Option<(f32, TilePos)> = None;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let t = center.offset(dx, dy);
            if !t.in_bounds() || !map.tile(t).passable() || (taken.contains(&t) && t != from) {
                continue;
            }
            let d = center.distance(t);
            if d > radius as f32 {
                continue;
            }
            let score = cover_value(map.cover_at(t)) * cover_weight + map.tile(t).elevation as f32 * height_weight
                - d * 0.35
                - from.distance(t) * from_weight;
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, t));
            }
        }
    }
    best.map(|(_, t)| t)
}

pub fn decide(
    tick: Res<SimTick>,
    map: Res<GameMap>,
    mut rng: ResMut<SimRng>,
    platoons: Query<&Platoon>,
    kings: Query<(&Faction, &TilePos), (With<Unit>, Without<Fighter>)>,
    positions: Query<&TilePos, With<Unit>>,
    mut fighters: Query<(
        Entity,
        &Unit,
        &Faction,
        &TilePos,
        &Fighter,
        &Traits,
        &Health,
        &Perception,
        &mut Condition,
        &mut Mover,
    )>,
) {
    // Tiles already occupied or claimed by fighters, so they spread out.
    let mut taken: HashSet<TilePos> = HashSet::new();
    for (_, _, _, pos, _, _, _, _, cond, _) in &fighters {
        taken.insert(*pos);
        if let Some(goal) = cond.goal {
            taken.insert(goal);
        }
    }

    // Fixed order (by stable id) so random rolls are reproducible.
    let mut order: Vec<(super::UnitId, Entity)> = fighters.iter().map(|f| (f.1.id, f.0)).collect();
    order.sort();

    for (_, entity) in order {
        let Ok((_, _, faction, pos, fighter, traits, health, perception, mut cond, mut mover)) = fighters.get_mut(entity)
        else {
            continue;
        };
        if tick.0 < cond.next_think {
            continue;
        }
        // Smarter fighters re-think more often (every 0.3 - 0.8 s).
        cond.next_think = tick.0 + 6 + ((1.0 - traits.iq()) * 10.0) as u64 + rng.0.random_range(0..3);

        let Ok(platoon) = platoons.get(fighter.platoon) else { continue };
        let stats = fighter.class.stats();
        let slot = slot_tile(platoon.anchor, fighter.slot, *faction);
        let cover_here = map.cover_at(*pos);
        let exposure = match cover_here {
            CoverLevel::None => 1.0,
            CoverLevel::Light => 0.45,
            CoverLevel::Heavy => 0.1,
        };

        let enemy_distances: Vec<f32> = perception
            .seen_enemies
            .iter()
            .filter_map(|&e| positions.get(e).ok())
            .map(|p| pos.distance(*p))
            .collect();
        let in_range = enemy_distances.iter().filter(|&&d| d <= stats.range).count();
        let threat = (enemy_distances.len() as f32 * 0.3).min(1.0);

        // Lower IQ = noisier, less consistent choices.
        let mut noise = || rng.0.random_range(0.0..0.2) * (1.0 - 0.7 * traits.iq());

        let old_goal = cond.goal;
        taken.remove(pos);
        if let Some(g) = old_goal {
            taken.remove(&g);
        }

        let (decision, goal) = if cond.morale < BROKEN {
            // Morale broke: fall back toward the King, looking for cover.
            let king = kings.iter().find(|(f, _)| **f == *faction).map(|(_, p)| *p);
            let home = king.unwrap_or(*pos);
            let dir = Vec2::new((home.x - pos.x) as f32, (home.y - pos.y) as f32).normalize_or_zero() * 10.0;
            let fallback = pos.offset(dir.x.round() as i32, dir.y.round() as i32);
            (Decision::Retreating, best_tile_near(&map, fallback, 3, *pos, &taken, 1.0, 0.0, 0.0))
        } else if platoon.frozen() {
            (Decision::Frozen, None)
        } else if cond.suppression >= PINNED {
            if cover_here != CoverLevel::None {
                (Decision::Holding, None)
            } else {
                (Decision::TakingCover, best_tile_near(&map, *pos, 3, *pos, &taken, 3.0, 0.0, 0.8))
            }
        } else {
            let tac = traits.tac();
            let at_slot = pos.distance(slot) <= 1.5;

            let s_cover = (cond.suppression / 100.0 + (1.0 - health.frac()) * 0.6 + (1.0 - cond.morale / 100.0) * 0.5
                + threat * 0.5)
                * (0.6 + 0.8 * tac)
                * exposure
                + noise();

            let s_advance = if at_slot {
                0.0
            } else {
                match platoon.mission {
                    Mission::Assault => 0.55 + 0.3 * traits.brav() - if in_range > 0 { 0.25 } else { 0.0 },
                    Mission::Reposition => 0.95,
                    Mission::Overwatch => {
                        if pos.distance(slot) > 2.5 {
                            0.7
                        } else {
                            0.05
                        }
                    }
                }
            } + noise();

            let s_fight = if in_range > 0 {
                let base = 0.5 + 0.3 * traits.acc() + if cover_here != CoverLevel::None { 0.15 } else { 0.0 };
                match platoon.mission {
                    Mission::Overwatch => base + 0.35,
                    Mission::Reposition => base * 0.4,
                    Mission::Assault => base,
                }
            } else {
                0.0
            } + noise();

            let best = s_cover.max(s_advance).max(s_fight);
            if best < 0.15 {
                (Decision::Holding, None)
            } else if best == s_cover {
                // Toward cover; assaulting and repositioning fighters prefer
                // cover that's also toward their objective (cover to cover).
                let center = match platoon.mission {
                    Mission::Overwatch => *pos,
                    _ => {
                        let step = Vec2::new((slot.x - pos.x) as f32, (slot.y - pos.y) as f32).clamp_length_max(2.0);
                        pos.offset(step.x.round() as i32, step.y.round() as i32)
                    }
                };
                (Decision::TakingCover, best_tile_near(&map, center, 4, *pos, &taken, 1.5 + 2.0 * tac, 0.0, 0.3))
            } else if best == s_advance {
                let radius = if platoon.mission == Mission::Reposition { 2 } else { 3 };
                let height = if platoon.mission == Mission::Overwatch { 0.6 * tac } else { 0.0 };
                (Decision::Advancing, best_tile_near(&map, slot, radius, *pos, &taken, 1.5 * tac, height, 0.0))
            } else {
                (Decision::Fighting, None)
            }
        };

        // Speed: sprinting when repositioning or diving for cover; slowed
        // when suppressed or badly hurt.
        let mut speed = 1.0;
        if platoon.mission == Mission::Reposition && decision == Decision::Advancing {
            speed *= 1.35;
        }
        if decision == Decision::TakingCover {
            speed *= 1.2;
        }
        if decision == Decision::Retreating {
            speed *= 1.15;
        }
        if cond.suppression >= 50.0 {
            speed *= 0.6;
        }
        if health.frac() < 0.3 {
            speed *= 0.75;
        }
        mover.speed_mult = speed;

        match goal {
            Some(g) if g != *pos => {
                if old_goal != Some(g) || mover.path.is_empty() {
                    set_destination(&map, *pos, &mut mover, g);
                }
            }
            _ => mover.stop(),
        }
        cond.decision = decision;
        cond.goal = goal.filter(|g| g != pos);
        taken.insert(*pos);
        if let Some(g) = cond.goal {
            taken.insert(g);
        }
    }
}
