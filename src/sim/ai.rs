//! Fighter autonomy.
//!
//! Every fighter makes its own decisions, a few times per second, within
//! its platoon's mission. It scores its options (advance, fight, take
//! cover, hold) from what it sees and how it's doing, tilted by its traits:
//!
//! - Tactics: how much it values cover, how well it picks spots, and
//!   whether it keeps every threat in mind or only the nearest one.
//! - IQ: how often it re-thinks and how erratic its choices are.
//! - Bravery: willingness to push forward under fire.
//! - Accuracy: willingness to stand and shoot.
//!
//! Overrides come first: broken fighters flee, pinned fighters stay down,
//! and a frozen platoon stops following orders. Shooting is separate (see
//! `combat.rs`): fighters shoot at what they see whenever they can,
//! including on the move.
//!
//! ## Choosing cover
//!
//! Cover is directional, so a fighter picks spots that put an obstacle
//! between itself and the enemies it knows about: the ones it sees, plus
//! the ones its platoon-mates see. Closer threats count more. Before any
//! contact, it assumes the enemy is ahead (toward its objective or the
//! enemy's side). A spot scores by how well it protects against each
//! threat, with a penalty for any threat it would be fully exposed to, so a
//! surrounded fighter settles for the spot that protects best overall.
//!
//! ## Strikes, traps and jobs
//!
//! Fighters that see an incoming strike's warning marker (or know about
//! their own side's) may notice it and run clear: smarter, more tactical
//! fighters notice sooner. Known enemy traps are walked around. Engineers
//! with a job (building, defusing) go and do it unless pinned or broken.
//!
//! Two ways of taking cover:
//! - Engage: covered, but still able to shoot back. Used when advancing,
//!   fighting, and moving cover to cover.
//! - Hide: out of sight is best. Used when suppressed, hurt, shaken or
//!   fleeing.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use rand::Rng;

use super::classes::Traits;
use super::fighter::{BROKEN, Condition, Decision, Fighter, Health, PINNED, SHAKEN, SUPPRESSED, WorkKind};
use super::gadgets::{Gadget, KnownTraps, Strike, danger_zones, escape_tile, work_spot};
use super::perception::{Perception, line_of_sight};
use super::platoon::{Mission, Platoon, slot_tile};
use super::{Mover, SimRng, SimTick, Unit, set_destination_avoiding};
use crate::faction::Faction;
use crate::map::{CoverLevel, GameMap, TilePos};

fn cover_value(level: CoverLevel) -> f32 {
    match level {
        CoverLevel::None => 0.0,
        CoverLevel::Light => 1.0,
        CoverLevel::Heavy => 2.0,
    }
}

/// Protection value of being completely out of an enemy's sight.
const HIDDEN_VALUE: f32 = 2.5;
/// Most threats a fighter weighs at once (the most dangerous ones).
const MAX_THREATS: usize = 6;

/// An enemy (or, before contact, the direction one is expected from).
#[derive(Clone, Copy, Debug)]
pub struct Threat {
    pub pos: TilePos,
    /// Closer threats matter more.
    pub weight: f32,
    /// False for the assumed direction of the enemy before contact.
    pub real: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CoverMode {
    /// Protected, but able to shoot back at threats within `range`.
    Engage { range: f32 },
    /// Out of sight is best.
    Hide,
}

/// How well a spot protects against a set of threats. Roughly 0 (fully
/// exposed) to 2.5 (hidden from everything).
pub fn protection(map: &GameMap, tile: TilePos, threats: &[Threat], mode: CoverMode) -> f32 {
    if threats.is_empty() {
        return cover_value(map.cover_at(tile));
    }
    let mut sum = 0.0;
    let mut weights = 0.0;
    let mut exposed = 0.0;
    let mut can_shoot = false;
    for t in threats {
        let seen = !t.real || line_of_sight(map, t.pos, tile);
        let value = if !seen {
            match mode {
                CoverMode::Hide => HIDDEN_VALUE,
                // Safe, but can't shoot from here.
                CoverMode::Engage { .. } => 1.2,
            }
        } else {
            let c = map.cover_against(tile, t.pos);
            if c == CoverLevel::None {
                exposed += t.weight;
            }
            if let CoverMode::Engage { range } = mode
                && t.real
                && tile.distance(t.pos) <= range
            {
                can_shoot = true;
            }
            cover_value(c)
        };
        sum += value * t.weight;
        weights += t.weight;
    }
    let mut score = sum / weights - 0.5 * exposed / weights;
    if let CoverMode::Engage { .. } = mode
        && threats.iter().any(|t| t.real)
    {
        score += if can_shoot { 0.6 } else { -0.4 };
    }
    score
}

/// Cover level a fighter actually has right now: the worst it has against
/// any real threat it knows about (out of sight counts as heavy), or the
/// best available if it knows of none.
pub fn current_cover(map: &GameMap, pos: TilePos, threats: &[Threat]) -> CoverLevel {
    let real: Vec<&Threat> = threats.iter().filter(|t| t.real).collect();
    if real.is_empty() {
        return map.cover_at(pos);
    }
    real.iter()
        .map(|t| if line_of_sight(map, t.pos, pos) { map.cover_against(pos, t.pos) } else { CoverLevel::Heavy })
        .min()
        .unwrap_or(CoverLevel::None)
}

struct SpotWeights {
    protection: f32,
    height: f32,
    near_center: f32,
    near_self: f32,
}

/// Best free tile within `radius` of `center`, and its score.
#[allow(clippy::too_many_arguments)]
fn best_spot(
    map: &GameMap,
    center: TilePos,
    radius: i32,
    from: TilePos,
    taken: &HashSet<TilePos>,
    avoid: &HashSet<TilePos>,
    threats: &[Threat],
    mode: CoverMode,
    w: &SpotWeights,
) -> Option<(TilePos, f32)> {
    let mut best: Option<(f32, TilePos)> = None;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let t = center.offset(dx, dy);
            if !t.in_bounds() || !map.tile(t).passable() || (taken.contains(&t) && t != from) || avoid.contains(&t) {
                continue;
            }
            let d = center.distance(t);
            if d > radius as f32 {
                continue;
            }
            let score = spot_score(map, t, center, from, threats, mode, w);
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, t));
            }
        }
    }
    best.map(|(s, t)| (t, s))
}

fn spot_score(
    map: &GameMap,
    t: TilePos,
    center: TilePos,
    from: TilePos,
    threats: &[Threat],
    mode: CoverMode,
    w: &SpotWeights,
) -> f32 {
    protection(map, t, threats, mode) * w.protection + map.tile(t).elevation as f32 * w.height
        - center.distance(t) * w.near_center
        - from.distance(t) * w.near_self
}

/// Where the enemy is expected from, before any contact: ahead toward the
/// objective, or toward the enemy's side of the map.
fn expected_threat(map: &GameMap, faction: Faction, pos: TilePos, platoon: &Platoon) -> TilePos {
    let enemy_home = map.spawn(match faction {
        Faction::Humans => Faction::Termbots,
        Faction::Termbots => Faction::Humans,
    });
    let from = match platoon.mission {
        Mission::Overwatch => platoon.anchor,
        _ => pos,
    };
    let mut dir = match platoon.mission {
        Mission::Overwatch => Vec2::ZERO,
        _ => Vec2::new((platoon.target.x - from.x) as f32, (platoon.target.y - from.y) as f32),
    };
    if dir.length() < 3.0 {
        dir = Vec2::new((enemy_home.x - from.x) as f32, (enemy_home.y - from.y) as f32);
    }
    let d = dir.normalize_or_zero() * 12.0;
    from.offset(d.x.round() as i32, d.y.round() as i32)
}

#[allow(clippy::too_many_arguments)]
pub fn decide(
    tick: Res<SimTick>,
    map: Res<GameMap>,
    mut rng: ResMut<SimRng>,
    known: Res<KnownTraps>,
    platoons: Query<&Platoon>,
    strikes: Query<&Strike>,
    gadgets: Query<&TilePos, With<Gadget>>,
    kings: Query<(&Unit, &Faction, &TilePos), Without<Fighter>>,
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
    let mut taken: HashSet<TilePos> = gadgets.iter().copied().collect();
    let zones = danger_zones(&strikes);
    // Enemies each platoon knows about (anyone in it can see them).
    let mut platoon_sightings: HashMap<Entity, Vec<Entity>> = HashMap::new();
    for (_, _, _, pos, fighter, _, _, perception, cond, _) in &fighters {
        taken.insert(*pos);
        if let Some(goal) = cond.goal {
            taken.insert(goal);
        }
        let seen = platoon_sightings.entry(fighter.platoon).or_default();
        for &e in &perception.seen_enemies {
            if !seen.contains(&e) {
                seen.push(e);
            }
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
        let tac = traits.tac();

        // Known threats: seen by this fighter or its platoon, nearest first.
        let mut threats: Vec<Threat> = platoon_sightings
            .get(&fighter.platoon)
            .into_iter()
            .flatten()
            .filter_map(|&e| positions.get(e).ok())
            .map(|p| Threat { pos: *p, weight: 1.0 / (1.0 + pos.distance(*p) / 6.0), real: true })
            .collect();
        threats.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.pos.cmp(&b.pos)));
        // Less tactical fighters only keep the nearest threat in mind.
        let keep = if tac < 0.4 { 1 } else { MAX_THREATS };
        threats.truncate(keep);
        let in_contact = !threats.is_empty();
        if !in_contact {
            threats.push(Threat { pos: expected_threat(&map, *faction, *pos, platoon), weight: 0.6, real: false });
        }

        let enemy_distances: Vec<f32> = perception
            .seen_enemies
            .iter()
            .filter_map(|&e| positions.get(e).ok())
            .map(|p| pos.distance(*p))
            .collect();
        let in_range = enemy_distances.iter().filter(|&&d| d <= stats.range).count();
        let threat_level = (enemy_distances.len() as f32 * 0.3).min(1.0);

        cond.cover = current_cover(&map, *pos, &threats);
        let hide_here = protection(&map, *pos, &threats, CoverMode::Hide);
        let exposure = (1.0 - hide_here / HIDDEN_VALUE).clamp(0.05, 1.0);

        let engage = CoverMode::Engage { range: stats.range };
        let defensive = cond.suppression >= SUPPRESSED || health.frac() < 0.5 || cond.morale < SHAKEN;
        let cover_mode = if defensive { CoverMode::Hide } else { engage };
        let prot_weight = 0.6 + 1.4 * tac;

        // Incoming strikes this fighter knows about and is standing under.
        let under: Vec<(TilePos, f32)> = zones
            .iter()
            .filter(|(t, r, f)| pos.distance(*t) <= r + 0.8 && (*f == *faction || perception.sees(*t)))
            .map(|(t, r, _)| (*t, *r))
            .collect();
        let notices = !under.is_empty()
            && (cond.decision == Decision::Dodging
                || rng.0.random::<f32>() < 0.2 + 0.25 * (traits.iq() + traits.tac()));

        // Lower IQ = noisier, less consistent choices.
        let mut noise = || rng.0.random_range(0.0..0.2) * (1.0 - 0.7 * traits.iq());

        let old_goal = cond.goal;
        taken.remove(pos);
        if let Some(g) = old_goal {
            taken.remove(&g);
        }
        let avoid = known.of(*faction);
        let spot = |center: TilePos, radius: i32, mode: CoverMode, w: SpotWeights| {
            best_spot(&map, center, radius, *pos, &taken, avoid, &threats, mode, &w).map(|(t, _)| t)
        };

        let (decision, goal) = if cond.morale < BROKEN {
            // Morale broke: fall back toward the King, hiding on the way.
            let king = kings
                .iter()
                .find(|(u, f, _)| u.class == super::UnitClass::King && **f == *faction)
                .map(|(_, _, p)| *p);
            let home = king.unwrap_or(*pos);
            let dir = Vec2::new((home.x - pos.x) as f32, (home.y - pos.y) as f32).normalize_or_zero() * 10.0;
            let fallback = pos.offset(dir.x.round() as i32, dir.y.round() as i32);
            let w = SpotWeights { protection: 1.0, height: 0.0, near_center: 0.35, near_self: 0.0 };
            (Decision::Retreating, spot(fallback, 3, CoverMode::Hide, w))
        } else if notices {
            // Get out from under it, whatever else is going on.
            let all: Vec<(TilePos, f32)> = zones.iter().map(|(t, r, _)| (*t, *r)).collect();
            (Decision::Dodging, escape_tile(&map, *pos, &all, &taken))
        } else if platoon.frozen() {
            (Decision::Frozen, None)
        } else if cond.suppression >= PINNED {
            if cond.cover != CoverLevel::None {
                (Decision::Holding, None)
            } else {
                // Crawl to the nearest spot that shields from the fire.
                let w = SpotWeights { protection: 3.0, height: 0.0, near_center: 0.35, near_self: 0.8 };
                (Decision::TakingCover, spot(*pos, 3, CoverMode::Hide, w))
            }
        } else if let Some(work) = cond.work {
            match work.kind {
                WorkKind::Build(_) => (Decision::Building, Some(work.tile)),
                WorkKind::Defuse(_) if pos.distance(work.tile) <= 1.5 => (Decision::Defusing, None),
                WorkKind::Defuse(_) => (Decision::Defusing, work_spot(&map, work.tile, *pos, &taken)),
            }
        } else {
            let at_slot = pos.distance(slot) <= 1.5;

            let s_cover = (cond.suppression / 100.0 + (1.0 - health.frac()) * 0.6 + (1.0 - cond.morale / 100.0) * 0.5
                + threat_level * 0.5)
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
                let base = 0.5 + 0.3 * traits.acc() + if cond.cover != CoverLevel::None { 0.15 } else { 0.0 };
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
                let w = SpotWeights { protection: prot_weight + 0.8, height: 0.0, near_center: 0.35, near_self: 0.3 };
                (Decision::TakingCover, spot(center, 4, cover_mode, w))
            } else if best == s_advance {
                let radius = if platoon.mission == Mission::Reposition { 2 } else { 3 };
                let height = if platoon.mission == Mission::Overwatch { 0.6 * tac } else { 0.0 };
                let w = SpotWeights { protection: prot_weight, height, near_center: 0.35, near_self: 0.0 };
                (Decision::Advancing, spot(slot, radius, engage, w))
            } else {
                // Fight from here, unless a clearly better firing position
                // is a step or two away.
                let w = SpotWeights { protection: prot_weight, height: 0.3 * tac, near_center: 0.0, near_self: 0.5 };
                let here = spot_score(&map, *pos, *pos, *pos, &threats, engage, &w);
                let better = best_spot(&map, *pos, 2, *pos, &taken, avoid, &threats, engage, &w)
                    .filter(|&(t, s)| t != *pos && s > here + 0.5)
                    .map(|(t, _)| t);
                (Decision::Fighting, better)
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
        if decision == Decision::Dodging {
            speed *= 1.3;
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
                    set_destination_avoiding(&map, *pos, &mut mover, g, avoid);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{Feature, Tile};

    fn flat_map() -> GameMap {
        let mut map = crate::map::generate::generate_map(5);
        for t in &mut map.tiles {
            *t = Tile::default();
        }
        map
    }

    fn threat(x: i32, y: i32) -> Threat {
        Threat { pos: TilePos::new(x, y), weight: 1.0, real: true }
    }

    const W: SpotWeights = SpotWeights { protection: 2.0, height: 0.0, near_center: 0.35, near_self: 0.0 };

    /// A wall segment running east-west at y = 21, from x = 18 to 22.
    fn map_with_wall() -> GameMap {
        let mut map = flat_map();
        for x in 18..=22 {
            map.tile_mut(TilePos::new(x, 21)).feature = Feature::Wall;
        }
        map
    }

    #[test]
    fn picks_the_side_of_the_wall_away_from_the_enemy() {
        let map = map_with_wall();
        let from = TilePos::new(20, 20);
        // Enemy to the north: stay on the south side, against the wall.
        let (spot, _) =
            best_spot(&map, from, 3, from, &HashSet::new(), &HashSet::new(), &[threat(20, 32)], CoverMode::Engage { range: 12.0 }, &W).unwrap();
        assert_eq!(spot.y, 20, "south of the wall, got {spot:?}");
        // Enemy to the south: go around to the north side.
        let (spot, _) =
            best_spot(&map, from, 3, from, &HashSet::new(), &HashSet::new(), &[threat(20, 8)], CoverMode::Engage { range: 12.0 }, &W).unwrap();
        assert_eq!(spot.y, 22, "north of the wall, got {spot:?}");
    }

    #[test]
    fn protection_counts_every_threat() {
        let map = map_with_wall();
        let south_side = TilePos::new(20, 20);
        let north = [threat(20, 32)];
        let both = [threat(20, 32), threat(20, 8)];
        let hide = CoverMode::Hide;
        assert!(protection(&map, south_side, &north, hide) > protection(&map, south_side, &both, hide));
    }

    #[test]
    fn surrounded_fighter_protects_against_the_closer_threat() {
        let map = map_with_wall();
        let from = TilePos::new(20, 21 - 1);
        // A close threat to the north, a distant one to the south.
        let threats = [
            Threat { pos: TilePos::new(20, 26), weight: 1.0 / (1.0 + 6.0 / 6.0), real: true },
            Threat { pos: TilePos::new(20, 5), weight: 1.0 / (1.0 + 15.0 / 6.0), real: true },
        ];
        let (spot, _) = best_spot(&map, from, 3, from, &HashSet::new(), &HashSet::new(), &threats, CoverMode::Hide, &W).unwrap();
        assert!(spot.y <= 20, "shelters from the nearer northern threat, got {spot:?}");
    }

    #[test]
    fn current_cover_is_the_worst_against_known_threats() {
        let map = map_with_wall();
        let me = TilePos::new(20, 20);
        assert_eq!(current_cover(&map, me, &[threat(20, 32)]), CoverLevel::Heavy);
        assert_eq!(current_cover(&map, me, &[threat(20, 32), threat(32, 20)]), CoverLevel::None);
    }
}
