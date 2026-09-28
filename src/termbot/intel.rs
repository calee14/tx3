//! What the AI commander knows about the enemy, and how it reads them.
//!
//! It plays by the same rules as a human: it only sees enemy units on
//! ground its side has explored, it never sees their missions, traits or
//! orders, and it reads intent from what's visible: where units move, how
//! fast, and whether they're shooting.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::map::TilePos;
use crate::sim::UnitClass;
use crate::sim::classes::FighterClass;

/// How long a shot keeps a unit marked as "firing" (3 s).
pub const FIRING_MEMORY_TICKS: u64 = 60;
/// Units within this many tiles of each other count as one group.
pub const GROUP_RADIUS: f32 = 7.0;

/// One enemy unit as last seen.
#[derive(Clone, Copy, Debug)]
pub struct Sighting {
    pub pos: TilePos,
    /// Position at the previous look (for movement).
    pub prev: TilePos,
    pub seen_tick: u64,
    pub prev_tick: u64,
    pub class: UnitClass,
    pub firing: bool,
}

impl Sighting {
    /// Tiles per second, from the last two looks.
    pub fn velocity(&self) -> Vec2 {
        let dt = (self.seen_tick.saturating_sub(self.prev_tick)) as f32 / crate::sim::SIM_HZ as f32;
        if dt < 0.5 {
            return Vec2::ZERO;
        }
        Vec2::new((self.pos.x - self.prev.x) as f32, (self.pos.y - self.prev.y) as f32) / dt
    }
}

/// What a group of enemies looks like it's doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behaviour {
    /// Standing still, not shooting.
    Holding,
    /// Standing still and shooting: probably dug in on Overwatch.
    Firing,
    /// Coming toward us at a steady pace: probably an Assault.
    Advancing,
    /// Moving fast and not shooting: probably a Reposition or a flank.
    Rushing,
    /// Moving away from us.
    Retreating,
}

impl Behaviour {
    pub fn name(self) -> &'static str {
        match self {
            Behaviour::Holding => "holding",
            Behaviour::Firing => "dug in and firing",
            Behaviour::Advancing => "advancing",
            Behaviour::Rushing => "rushing",
            Behaviour::Retreating => "pulling back",
        }
    }
}

/// Speeds (tiles/s) used to read movement.
const STILL: f32 = 0.35;
const FAST: f32 = 2.2;

/// Read a group's intent from its average velocity, the direction toward
/// our forces, and how many of it are shooting.
pub fn classify(velocity: Vec2, toward_us: Vec2, firing_frac: f32) -> Behaviour {
    let speed = velocity.length();
    if speed < STILL {
        return if firing_frac >= 0.25 { Behaviour::Firing } else { Behaviour::Holding };
    }
    if speed >= FAST && firing_frac < 0.3 {
        return Behaviour::Rushing;
    }
    if velocity.normalize().dot(toward_us.normalize_or_zero()) < -0.3 {
        Behaviour::Retreating
    } else {
        Behaviour::Advancing
    }
}

/// How dangerous a unit looks, by class (visible to anyone).
pub fn class_weight(class: UnitClass) -> f32 {
    match class {
        UnitClass::King => 0.0,
        UnitClass::Vehicle => 1.2,
        UnitClass::Fighter(FighterClass::Assaulter) => 1.0,
        UnitClass::Fighter(FighterClass::Sniper) => 1.3,
        UnitClass::Fighter(FighterClass::HeavyWeapons) => 1.3,
        UnitClass::Fighter(FighterClass::Juggernaut) => 1.8,
    }
}

/// A cluster of enemy units that seem to be acting together.
#[derive(Clone, Debug)]
pub struct EnemyGroup {
    pub members: Vec<Entity>,
    /// Center, in tile coordinates.
    pub center: Vec2,
    pub strength: f32,
    pub velocity: Vec2,
    pub behaviour: Behaviour,
    /// Distance to our King, in tiles.
    pub king_distance: f32,
}

impl EnemyGroup {
    pub fn center_tile(&self) -> TilePos {
        TilePos::new(self.center.x.round() as i32, self.center.y.round() as i32)
    }
}

/// Group sightings by proximity (units chained within `GROUP_RADIUS`).
pub fn cluster(sightings: &[(Entity, Sighting)]) -> Vec<Vec<usize>> {
    let n = sightings.len();
    let mut group = vec![usize::MAX; n];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if group[start] != usize::MAX {
            continue;
        }
        let id = groups.len();
        let mut members = vec![start];
        group[start] = id;
        let mut i = 0;
        while i < members.len() {
            let a = sightings[members[i]].1.pos;
            for j in 0..n {
                if group[j] == usize::MAX && a.distance(sightings[j].1.pos) <= GROUP_RADIUS {
                    group[j] = id;
                    members.push(j);
                }
            }
            i += 1;
        }
        groups.push(members);
    }
    groups
}

/// Build enemy groups from current sightings. `our_assets` are positions
/// the enemy might be heading for (our King, our platoons).
pub fn read_enemy(sightings: &HashMap<Entity, Sighting>, our_king: TilePos, our_assets: &[TilePos]) -> Vec<EnemyGroup> {
    let mut list: Vec<(Entity, Sighting)> = sightings
        .iter()
        .filter(|(_, s)| s.class != UnitClass::King)
        .map(|(e, s)| (*e, *s))
        .collect();
    list.sort_by_key(|(e, _)| *e);

    cluster(&list)
        .into_iter()
        .map(|idx| {
            let n = idx.len() as f32;
            let members: Vec<Entity> = idx.iter().map(|&i| list[i].0).collect();
            let center = idx
                .iter()
                .map(|&i| Vec2::new(list[i].1.pos.x as f32, list[i].1.pos.y as f32))
                .sum::<Vec2>()
                / n;
            let velocity = idx.iter().map(|&i| list[i].1.velocity()).sum::<Vec2>() / n;
            let firing_frac = idx.iter().filter(|&&i| list[i].1.firing).count() as f32 / n;
            let strength = idx.iter().map(|&i| class_weight(list[i].1.class)).sum();
            let center_tile = TilePos::new(center.x.round() as i32, center.y.round() as i32);
            // What are they closest to: our King or one of our platoons?
            let target = our_assets
                .iter()
                .copied()
                .chain(std::iter::once(our_king))
                .min_by(|a, b| a.distance(center_tile).total_cmp(&b.distance(center_tile)))
                .unwrap_or(our_king);
            let toward_us = Vec2::new(target.x as f32, target.y as f32) - center;
            EnemyGroup {
                members,
                center,
                strength,
                velocity,
                behaviour: classify(velocity, toward_us, firing_frac),
                king_distance: our_king.distance(center_tile),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOWARD: Vec2 = Vec2::new(0.0, -1.0);

    #[test]
    fn reads_behaviour_from_movement_and_fire() {
        assert_eq!(classify(Vec2::ZERO, TOWARD, 0.0), Behaviour::Holding);
        assert_eq!(classify(Vec2::ZERO, TOWARD, 0.5), Behaviour::Firing);
        assert_eq!(classify(Vec2::new(0.0, -1.2), TOWARD, 0.4), Behaviour::Advancing);
        assert_eq!(classify(Vec2::new(2.8, 0.5), TOWARD, 0.0), Behaviour::Rushing);
        assert_eq!(classify(Vec2::new(0.0, 1.2), TOWARD, 0.0), Behaviour::Retreating);
        // Fast but shooting: still an advance, not a rush.
        assert_eq!(classify(Vec2::new(0.0, -2.5), TOWARD, 0.5), Behaviour::Advancing);
    }

    #[test]
    fn clusters_nearby_units_together() {
        let s = |x, y| Sighting {
            pos: TilePos::new(x, y),
            prev: TilePos::new(x, y),
            seen_tick: 0,
            prev_tick: 0,
            class: UnitClass::Fighter(FighterClass::Assaulter),
            firing: false,
        };
        let e = |i| Entity::from_raw_u32(i).unwrap();
        let list = vec![(e(1), s(10, 10)), (e(2), s(14, 10)), (e(3), s(18, 12)), (e(4), s(50, 50))];
        let groups = cluster(&list);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), 3, "chained within the group radius");
    }
}
