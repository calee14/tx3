//! Platoons: mixed groups of fighters led by a leader, carrying out one
//! mission at a time. The King commands platoons with signals.

use std::collections::{HashSet, VecDeque};

use bevy::prelude::*;
use rand::Rng;

use super::classes::{Equipment, FighterClass, Loadout, Traits};
use super::economy::Supply;
use super::fighter::{Condition, Fighter, Health, Veterancy, Work, WorkKind};
use super::gadgets::GadgetKind;
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
/// How long a signal takes to travel from the King to a platoon. Until it
/// arrives the platoon carries on with its current orders, so commanding
/// is about planning and positioning, not reflexes.
pub const SIGNAL_TRAVEL_SECONDS: f32 = 1.0;
pub const SIGNAL_TRAVEL_TICKS: u64 = (SIGNAL_TRAVEL_SECONDS * SIM_HZ as f32) as u64;

/// A signal on its way from the King to a platoon.
#[derive(Clone, Debug)]
pub struct SignalInFlight {
    pub issuer: Faction,
    pub platoon: Entity,
    pub mission: Mission,
    pub target: TilePos,
    /// Where the King was when it sent the signal (for the visual pulse).
    pub from: TilePos,
    pub sent_tick: u64,
    pub arrive_tick: u64,
    /// A build order for the platoon's engineer instead of a mission
    /// (with the Supply already paid, refunded if nobody can build it).
    pub build: Option<(GadgetKind, f32)>,
}

/// Signals sent but not yet received, oldest first.
#[derive(Resource, Default, Debug)]
pub struct SignalsInFlight(pub Vec<SignalInFlight>);

impl SignalsInFlight {
    pub fn heading_to(&self, platoon: Entity) -> Option<&SignalInFlight> {
        self.0.iter().rev().find(|s| s.platoon == platoon)
    }
}

/// Deliver signals that have reached their platoon. Confusion counts from
/// the moment a signal arrives.
pub fn deliver_signals(
    tick: Res<SimTick>,
    mut flight: ResMut<SignalsInFlight>,
    mut supply: ResMut<Supply>,
    mut platoons: Query<&mut Platoon>,
    traits: Query<&Traits>,
    mut engineers: Query<(&Fighter, &TilePos, &mut Condition)>,
) {
    if flight.0.is_empty() {
        return;
    }
    let (arrived, travelling): (Vec<SignalInFlight>, Vec<SignalInFlight>) =
        std::mem::take(&mut flight.0).into_iter().partition(|s| s.arrive_tick <= tick.0);
    flight.0 = travelling;
    for s in arrived {
        let Ok(mut platoon) = platoons.get_mut(s.platoon) else { continue };
        let leadership = platoon.leader.and_then(|l| traits.get(l).ok()).map_or(0.0, |t| t.lead());
        let Some((kind, paid)) = s.build else {
            receive_signal(&mut platoon, s.mission, s.target, tick.0, leadership);
            continue;
        };
        // A build order: a signal like any other (adds some confusion), for
        // the platoon's free engineer nearest the site.
        platoon.confusion = (platoon.confusion + BUILD_SIGNAL_CONFUSION * (1.25 - 0.5 * leadership)).min(100.0);
        platoon.last_signal_tick = Some(tick.0);
        let builder = platoon
            .members
            .iter()
            .filter(|m| engineers.get(**m).is_ok_and(|(f, _, c)| f.class == FighterClass::Engineer && c.work.is_none()))
            .min_by(|a, b| {
                let d = |e: &Entity| engineers.get(*e).map_or(f32::MAX, |(_, p, _)| p.distance(s.target));
                d(a).total_cmp(&d(b)).then(a.cmp(b))
            })
            .copied();
        match builder.and_then(|b| engineers.get_mut(b).ok()) {
            Some((_, _, mut cond)) => {
                cond.work = Some(Work { kind: WorkKind::Build(kind), tile: s.target, progress: 0.0, paid, idle: 0.0 });
            }
            None => supply.0[s.issuer.index()] += paid,
        }
    }
}

/// Confusion added by an engineer build order.
pub const BUILD_SIGNAL_CONFUSION: f32 = 12.0;

/// How far an assaulting platoon advances per bound, in tiles.
const ASSAULT_BOUND: usize = 7;
/// An assaulting platoon bounds forward after this long even if not
/// everyone has caught up (10 s).
const BOUND_TIMEOUT_TICKS: u64 = 200;

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
    /// Supply paid for this platoon (refunded if placement is undone
    /// during setup).
    pub paid: f32,
    /// Tick of the last assault bound (or when the assault began).
    pub last_bound_tick: u64,
}

/// Largest allowed platoon.
pub const MAX_PLATOON_SIZE: usize = 5;

/// Platoon call signs, handed out in order per side.
pub const CALL_SIGNS: [&str; 16] = [
    "Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot", "Golf", "Hotel", "India", "Juliet", "Kilo", "Lima", "Mike",
    "November", "Oscar", "Papa",
];

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
    slots: Query<(&TilePos, &Fighter, &Faction)>,
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
            platoon.last_bound_tick = tick.0;
            platoon.mission = pending.mission;
            platoon.target = pending.target;
            platoon.anchor = match pending.mission {
                // Assault bounds forward from where the platoon is now.
                Mission::Assault => centroid_tile(&map, &platoon.members, &members).unwrap_or(platoon.anchor),
                _ => pending.target,
            };
        }

        // Assault: once most of the platoon has reached its places around the
        // anchor, bound forward. Fighters may settle into cover a few tiles
        // from their exact slot, so "arrived" is measured against each
        // fighter's own slot; and a bound never waits more than 10 s.
        if platoon.mission == Mission::Assault && platoon.anchor != platoon.target {
            let arrived = platoon
                .members
                .iter()
                .filter_map(|&m| slots.get(m).ok())
                .filter(|(p, f, faction)| p.distance(slot_tile(platoon.anchor, f.slot, **faction)) <= 4.5)
                .count();
            let waited_too_long = tick.0.saturating_sub(platoon.last_bound_tick) > BOUND_TIMEOUT_TICKS;
            if arrived * 2 >= platoon.members.len() || waited_too_long {
                platoon.last_bound_tick = tick.0;
                if let Some(path) = find_path(&map, platoon.anchor, platoon.target) {
                    platoon.anchor = path.get(ASSAULT_BOUND - 1).or(path.back()).copied().unwrap_or(platoon.target);
                } else {
                    platoon.anchor = platoon.target;
                }
            }
        }

        // Reposition: once most of the platoon has arrived, it digs in and
        // watches (Overwatch at the same spot). Not a signal, so no confusion.
        if platoon.mission == Mission::Reposition && platoon.pending.is_none() {
            let arrived = platoon
                .members
                .iter()
                .filter_map(|&m| members.get(m).ok())
                .filter(|(_, p, _)| p.distance(platoon.target) <= REPOSITION_ARRIVED)
                .count();
            if arrived * 2 >= platoon.members.len() {
                platoon.mission = Mission::Overwatch;
                platoon.anchor = platoon.target;
            }
        }
    }
}

/// A repositioning fighter this close to the target counts as arrived.
const REPOSITION_ARRIVED: f32 = 3.5;

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
    nearest_free_tile_where(map, taken, around, |_| true)
}

/// Like `nearest_free_tile`, but only accepts tiles passing `allowed`.
pub fn nearest_free_tile_where(
    map: &GameMap,
    taken: &HashSet<TilePos>,
    around: TilePos,
    allowed: impl Fn(TilePos) -> bool,
) -> Option<TilePos> {
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
    let mut steps = 0;
    while let Some(p) = queue.pop_front() {
        steps += 1;
        if steps > 4000 {
            break;
        }
        if !taken.contains(&p) && allowed(p) {
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

const KING_HEALTH: f32 = 150.0;

pub fn spawn_king(commands: &mut Commands, index: &mut UnitIndex, map: &GameMap, faction: Faction) -> Entity {
    let king_pos = map.spawn(faction);
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
    king
}

/// A fighter that has been bought but not spawned yet (e.g. riding in a
/// vehicle). Traits are rolled at purchase.
#[derive(Clone, Copy, Debug)]
pub struct Recruit {
    pub class: FighterClass,
    pub traits: Traits,
    /// Hidden equipment, rolled at purchase.
    pub loadout: Loadout,
}

pub fn roll_recruits(classes: &[FighterClass], rng: &mut impl Rng) -> Vec<Recruit> {
    classes
        .iter()
        .map(|&class| Recruit {
            class,
            traits: Traits::roll(&class.stats().traits, rng),
            loadout: Loadout::roll(class, rng),
        })
        .collect()
}

/// Everything needed to put a new platoon on the field.
pub struct NewPlatoon<'a> {
    pub faction: Faction,
    pub recruits: &'a [Recruit],
    pub anchor: TilePos,
    pub mission: Mission,
    pub target: TilePos,
    pub paid: f32,
}

/// Spawn a platoon's fighters around `anchor` (simulation components only;
/// visuals are attached separately so the sim can run headless). Fighters
/// go on free walkable tiles that pass `allowed`. Returns the platoon
/// entity and its fighters.
pub fn spawn_platoon(
    commands: &mut Commands,
    index: &mut UnitIndex,
    map: &GameMap,
    taken: &mut HashSet<TilePos>,
    new: NewPlatoon,
    allowed: impl Fn(TilePos) -> bool,
) -> (Entity, Vec<Entity>) {
    let faction = new.faction;
    let platoon_id = index.allocate_platoon();
    let number = index.next_platoon_number(faction);
    let platoon_entity = commands.spawn((MatchEntity, faction)).id();

    let mut members = Vec::new();
    for (slot, recruit) in new.recruits.iter().enumerate().take(MAX_PLATOON_SIZE) {
        let stats = recruit.class.stats();
        let pos = nearest_free_tile_where(map, taken, slot_tile(new.anchor, slot, faction), &allowed)
            .or_else(|| nearest_free_tile(map, taken, new.anchor))
            .unwrap_or(new.anchor);
        taken.insert(pos);
        let id = index.allocate();
        let fighter = commands
            .spawn((
                MatchEntity,
                Unit { id, class: UnitClass::Fighter(recruit.class) },
                faction,
                pos,
                Mover::new(stats.speed),
                Vision { radius: stats.vision },
                Perception::default(),
                SimInterp::at(pos.to_world()),
                Transform::from_translation(pos.to_world().extend(0.0)),
                Fighter { class: recruit.class, platoon: platoon_entity, slot },
                recruit.traits,
                Health::new(stats.max_health),
                Condition::new(&recruit.traits),
                Equipment::new(recruit.loadout, faction.index()),
                Veterancy::default(),
            ))
            .id();
        index.entities.insert(id, fighter);
        members.push(fighter);
    }

    let name = CALL_SIGNS.get(number as usize).map_or_else(|| format!("Platoon {}", number + 1), |n| n.to_string());
    commands.entity(platoon_entity).insert(Platoon {
        id: platoon_id,
        name,
        mission: new.mission,
        target: new.target,
        anchor: if new.mission == Mission::Assault { new.anchor } else { new.target },
        members: members.clone(),
        leader: None,
        confusion: 0.0,
        last_signal_tick: None,
        pending: None,
        paid: new.paid,
        last_bound_tick: 0,
    });
    index.platoons.insert(platoon_id, platoon_entity);
    (platoon_entity, members)
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
            paid: 0.0,
            last_bound_tick: 0,
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
