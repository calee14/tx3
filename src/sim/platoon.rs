//! Platoons: mixed groups of fighters led by a leader, carrying out one
//! mission at a time. The King commands platoons with signals.

use std::collections::{HashSet, VecDeque};

use bevy::prelude::*;
use rand::Rng;

use super::classes::{FighterClass, Traits};
use super::fighter::{Condition, Fighter, Health};
use super::perception::{Perception, Vision};
use super::{DT, Mover, SIM_HZ, SimInterp, SimTick, Unit, UnitClass, UnitIndex};
use crate::faction::Faction;
use crate::game::MatchEntity;
use crate::map::{GameMap, TilePos};
use crate::pathfinding::find_path;

/// Confusion at or above this delays new orders.
pub const CONFUSION_MEDIUM: f32 = 35.0;
/// Confusion at or above this delays new orders even longer.
pub const CONFUSION_HIGH: f32 = 70.0;
/// Confusion at or above this freezes the platoon: it stops following its
/// mission (fighters still defend themselves).
pub const CONFUSION_FREEZE: f32 = 85.0;
/// How far an assaulting platoon advances per bound, in tiles.
const ASSAULT_BOUND: usize = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mission {
    /// Push toward the target in bounds, cover to cover, attacking what
    /// it finds. Beats Reposition (catches it in the open).
    Assault,
    /// Hold a position in cover and fire first at anything that moves in.
    /// Beats Assault (reaction fire).
    Overwatch,
    /// Move fast to the target, dodging, favoring speed over shooting.
    /// Beats Overwatch (hard to hit while sprinting between cover).
    Reposition,
}

impl Mission {
    pub fn name(self) -> &'static str {
        match self {
            Mission::Assault => "Assault",
            Mission::Overwatch => "Overwatch",
            Mission::Reposition => "Reposition",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlatoonId(pub u32);

#[derive(Clone, Copy, Debug)]
pub struct PendingSignal {
    pub mission: Mission,
    pub target: TilePos,
    pub apply_tick: u64,
}

#[derive(Component, Debug)]
pub struct Platoon {
    pub id: PlatoonId,
    pub name: String,
    pub mission: Mission,
    /// Where the mission is headed (or held, for Overwatch).
    pub target: TilePos,
    /// Where the platoon is forming up right now. For Assault this moves
    /// toward the target one bound at a time.
    pub anchor: TilePos,
    pub members: Vec<Entity>,
    pub leader: Option<Entity>,
    /// 0-100. Rises with every signal, faster with contradictory orders,
    /// and decays over time (faster with smart fighters and a good leader).
    pub confusion: f32,
    pub last_signal_tick: Option<u64>,
    /// A received order not acted on yet (delayed by confusion).
    pub pending: Option<PendingSignal>,
}

impl Platoon {
    pub fn frozen(&self) -> bool {
        self.confusion >= CONFUSION_FREEZE
    }

    pub fn confusion_label(&self) -> &'static str {
        if self.frozen() {
            "frozen"
        } else if self.confusion >= CONFUSION_HIGH {
            "high"
        } else if self.confusion >= CONFUSION_MEDIUM {
            "medium"
        } else {
            "low"
        }
    }
}

/// Formation offsets (in tiles) around the anchor, by slot.
pub const FORMATION: [(i32, i32); 6] = [(0, 0), (-2, -1), (2, -1), (-1, -3), (1, -3), (0, -5)];

pub fn slot_tile(anchor: TilePos, slot: usize, faction: Faction) -> TilePos {
    let (dx, dy) = FORMATION[slot % FORMATION.len()];
    // Termbots face the other way: mirror the formation.
    match faction {
        Faction::Humans => anchor.offset(dx, dy),
        Faction::Termbots => anchor.offset(-dx, -dy),
    }
}

/// Handle a signal from the King. Every signal adds confusion; a signal
/// that contradicts a recent one adds much more. Confused platoons act on
/// orders late, and very confused ones freeze.
pub fn receive_signal(platoon: &mut Platoon, mission: Mission, target: TilePos, tick: u64, leadership: f32) {
    let before = platoon.confusion;
    let mut added = 18.0;
    if let Some(last) = platoon.last_signal_tick {
        let since = (tick - last) as f32 / SIM_HZ as f32;
        if since < 3.0 {
            added += 10.0; // spam
        }
        if since < 8.0 && mission != platoon.pending.map_or(platoon.mission, |p| p.mission) {
            added += 22.0; // contradicts a recent order
        }
    }
    // A strong leader keeps the platoon steady.
    added *= 1.25 - 0.5 * leadership;
    platoon.confusion = (platoon.confusion + added).min(100.0);
    platoon.last_signal_tick = Some(tick);

    let delay_secs = if before < CONFUSION_MEDIUM {
        0.0
    } else if before < CONFUSION_HIGH {
        1.5
    } else {
        3.5
    };
    platoon.pending = Some(PendingSignal {
        mission,
        target,
        apply_tick: tick + (delay_secs * SIM_HZ as f32) as u64,
    });
}

pub fn update_platoons(
    mut commands: Commands,
    tick: Res<SimTick>,
    map: Res<GameMap>,
    mut platoons: Query<(Entity, &mut Platoon)>,
    members: Query<(&Unit, &TilePos, &Traits), With<Fighter>>,
) {
    for (entity, mut platoon) in &mut platoons {
        platoon.members.retain(|&m| members.get(m).is_ok());
        if platoon.members.is_empty() {
            commands.entity(entity).despawn();
            continue;
        }

        // Leader: keep the current one while alive, else the member with
        // the best leadership steps up.
        if platoon.leader.is_none_or(|l| !platoon.members.contains(&l)) {
            platoon.leader = platoon
                .members
                .iter()
                .filter_map(|&m| members.get(m).ok().map(|(u, _, t)| (m, u.id, t.leadership)))
                .max_by_key(|&(_, id, lead)| (lead, std::cmp::Reverse(id)))
                .map(|(m, _, _)| m);
        }

        // Confusion decays over time.
        let n = platoon.members.len() as f32;
        let avg_iq = platoon.members.iter().filter_map(|&m| members.get(m).ok()).map(|(_, _, t)| t.iq()).sum::<f32>() / n;
        let lead = platoon.leader.and_then(|l| members.get(l).ok()).map_or(0.0, |(_, _, t)| t.lead());
        let decay = 4.0 * (0.5 + avg_iq) * (1.0 + 0.5 * lead);
        platoon.confusion = (platoon.confusion - decay * DT).max(0.0);

        // Carry out a pending order once its delay has passed.
        if let Some(pending) = platoon.pending
            && tick.0 >= pending.apply_tick
        {
            platoon.pending = None;
            platoon.mission = pending.mission;
            platoon.target = pending.target;
            platoon.anchor = match pending.mission {
                // Assault bounds forward from where the platoon is now.
                Mission::Assault => centroid_tile(&map, &platoon.members, &members).unwrap_or(platoon.anchor),
                _ => pending.target,
            };
        }

        // Assault: once most of the platoon reaches the anchor, bound forward.
        if platoon.mission == Mission::Assault && platoon.anchor != platoon.target {
            let arrived = platoon
                .members
                .iter()
                .filter_map(|&m| members.get(m).ok())
                .filter(|(_, p, _)| p.distance(platoon.anchor) <= 4.0)
                .count();
            if arrived * 2 >= platoon.members.len() {
                if let Some(path) = find_path(&map, platoon.anchor, platoon.target) {
                    platoon.anchor = path.get(ASSAULT_BOUND - 1).or(path.back()).copied().unwrap_or(platoon.target);
                } else {
                    platoon.anchor = platoon.target;
                }
            }
        }
    }
}

/// The walkable tile closest to the platoon's average position.
fn centroid_tile(
    map: &GameMap,
    members: &[Entity],
    query: &Query<(&Unit, &TilePos, &Traits), With<Fighter>>,
) -> Option<TilePos> {
    let positions: Vec<TilePos> = members.iter().filter_map(|&m| query.get(m).ok()).map(|(_, p, _)| *p).collect();
    if positions.is_empty() {
        return None;
    }
    let n = positions.len() as f32;
    let cx = positions.iter().map(|p| p.x as f32).sum::<f32>() / n;
    let cy = positions.iter().map(|p| p.y as f32).sum::<f32>() / n;
    let center = TilePos::new(cx.round() as i32, cy.round() as i32);
    nearest_free_tile(map, &HashSet::new(), center)
}

/// Breadth-first search for the closest walkable tile not in `taken`.
pub fn nearest_free_tile(map: &GameMap, taken: &HashSet<TilePos>, around: TilePos) -> Option<TilePos> {
    let mut seen = HashSet::new();
    let mut queue = VecDeque::new();
    // Start from any walkable tile near `around` (it may be inside a rock).
    for r in 0..8 {
        for dy in -r..=r {
            for dx in -r..=r {
                let p = around.offset(dx, dy);
                if p.in_bounds() && map.tile(p).passable() && seen.insert(p) {
                    queue.push_back(p);
                }
            }
        }
        if !queue.is_empty() {
            break;
        }
    }
    while let Some(p) = queue.pop_front() {
        if !taken.contains(&p) {
            return Some(p);
        }
        for n in map.neighbors(p) {
            if seen.insert(n) {
                queue.push_back(n);
            }
        }
    }
    None
}

pub struct PlatoonTemplate {
    pub name: &'static str,
    pub members: &'static [FighterClass],
}

/// Each side's starting platoons: mixed classes so every platoon can do a
/// bit of everything, with a different emphasis.
pub const STARTING_PLATOONS: [PlatoonTemplate; 3] = [
    PlatoonTemplate {
        name: "Rifle",
        members: &[FighterClass::Assaulter, FighterClass::Assaulter, FighterClass::Assaulter, FighterClass::HeavyWeapons],
    },
    PlatoonTemplate {
        name: "Recon",
        members: &[FighterClass::Sniper, FighterClass::Sniper, FighterClass::Assaulter],
    },
    PlatoonTemplate {
        name: "Breach",
        members: &[FighterClass::Juggernaut, FighterClass::Assaulter, FighterClass::Assaulter, FighterClass::HeavyWeapons],
    },
];

/// Humans' starting platoon positions; Termbots get the mirrored tiles.
const HUMAN_PLATOON_ANCHORS: [TilePos; 3] = [TilePos::new(52, 17), TilePos::new(63, 20), TilePos::new(74, 17)];
const KING_HEALTH: f32 = 150.0;

/// Spawn one side's King and starting platoons (simulation components
/// only; visuals are attached separately so the sim can run headless).
pub fn spawn_forces(commands: &mut Commands, index: &mut UnitIndex, rng: &mut impl Rng, map: &GameMap, faction: Faction) {
    let mut taken: HashSet<TilePos> = HashSet::new();

    let king_pos = map.spawn(faction);
    taken.insert(king_pos);
    let king_id = index.allocate();
    let king = commands
        .spawn((
            MatchEntity,
            Unit { id: king_id, class: UnitClass::King },
            faction,
            king_pos,
            Mover::new(2.5),
            Vision { radius: 9.0 },
            Perception::default(),
            SimInterp::at(king_pos.to_world()),
            Transform::from_translation(king_pos.to_world().extend(0.0)),
            Health::new(KING_HEALTH),
        ))
        .id();
    index.entities.insert(king_id, king);

    for (i, template) in STARTING_PLATOONS.iter().enumerate() {
        let anchor_human = HUMAN_PLATOON_ANCHORS[i];
        let anchor = match faction {
            Faction::Humans => anchor_human,
            Faction::Termbots => anchor_human.mirrored(),
        };
        let anchor = nearest_free_tile(map, &HashSet::new(), anchor).unwrap_or(anchor);
        let platoon_id = index.allocate_platoon();
        let platoon_entity = commands.spawn((MatchEntity, faction)).id();

        let mut members = Vec::new();
        for (slot, &class) in template.members.iter().enumerate() {
            let stats = class.stats();
            let traits = Traits::roll(&stats.traits, rng);
            let pos = nearest_free_tile(map, &taken, slot_tile(anchor, slot, faction)).unwrap_or(anchor);
            taken.insert(pos);
            let id = index.allocate();
            let fighter = commands
                .spawn((
                    MatchEntity,
                    Unit { id, class: UnitClass::Fighter(class) },
                    faction,
                    pos,
                    Mover::new(stats.speed),
                    Vision { radius: stats.vision },
                    Perception::default(),
                    SimInterp::at(pos.to_world()),
                    Transform::from_translation(pos.to_world().extend(0.0)),
                    Fighter { class, platoon: platoon_entity, slot },
                    traits,
                    Health::new(stats.max_health),
                    Condition::new(&traits),
                ))
                .id();
            index.entities.insert(id, fighter);
            members.push(fighter);
        }

        commands.entity(platoon_entity).insert(Platoon {
            id: platoon_id,
            name: format!("{} {}", template.name, i + 1),
            mission: Mission::Overwatch,
            target: anchor,
            anchor,
            members,
            leader: None,
            confusion: 0.0,
            last_signal_tick: None,
            pending: None,
        });
        index.platoons.insert(platoon_id, platoon_entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_platoon() -> Platoon {
        Platoon {
            id: PlatoonId(0),
            name: "Test".into(),
            mission: Mission::Overwatch,
            target: TilePos::new(10, 10),
            anchor: TilePos::new(10, 10),
            members: vec![],
            leader: None,
            confusion: 0.0,
            last_signal_tick: None,
            pending: None,
        }
    }

    #[test]
    fn first_signal_applies_immediately() {
        let mut p = test_platoon();
        receive_signal(&mut p, Mission::Assault, TilePos::new(20, 20), 100, 0.5);
        assert_eq!(p.pending.unwrap().apply_tick, 100);
        assert!(p.confusion > 0.0 && p.confusion < CONFUSION_MEDIUM);
    }

    #[test]
    fn contradictory_spam_confuses_and_freezes() {
        let mut p = test_platoon();
        let missions = [Mission::Assault, Mission::Reposition, Mission::Assault, Mission::Overwatch];
        for (i, &m) in missions.iter().enumerate() {
            receive_signal(&mut p, m, TilePos::new(20, 20), 100 + i as u64 * 10, 0.3);
        }
        assert!(p.frozen(), "confusion {}", p.confusion);
        assert!(p.pending.unwrap().apply_tick > 130, "late orders are delayed");
    }

    #[test]
    fn good_leaders_reduce_confusion() {
        let mut weak = test_platoon();
        let mut strong = test_platoon();
        receive_signal(&mut weak, Mission::Assault, TilePos::new(20, 20), 0, 0.1);
        receive_signal(&mut strong, Mission::Assault, TilePos::new(20, 20), 0, 0.9);
        assert!(strong.confusion < weak.confusion);
    }
}
