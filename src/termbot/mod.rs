//! The Termbot commander: an AI that plays the King's role.
//!
//! It plays by the same rules as a human player:
//! - It only acts through commands (signals, deployments, moving its King),
//!   so its platoons get confused by spammed or contradictory signals
//!   exactly like yours.
//! - It only knows about enemy units on ground its side has explored, and
//!   reads their intent from movement and gunfire (see `intel.rs`).
//!
//! Every 2 seconds it looks at the battlefield and:
//! 1. Groups visible enemies and reads what each group is doing.
//! 2. Picks a posture: Scouting (opening), Holding, Pushing (stronger, or
//!    behind on territory late), or Defending (King threatened).
//! 3. Sends its nearest platoons to answer each enemy group with the
//!    counter-mission: dig in (Overwatch) against an advance, catch rushers
//!    in the open (Assault), flank or slip out of a firing line
//!    (Reposition). Now and then it picks a bolder option, so it isn't
//!    perfectly predictable.
//! 4. Gives idle platoons a job: scout, hold their post, retake lost
//!    sectors, or join a push.
//! 5. Moves its King away from danger.
//! 6. Buys reinforcements that counter what it has seen, by vehicle when
//!    far or urgent.

pub mod intel;
pub mod planning;

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use rand::Rng;

use crate::faction::Faction;
use crate::fog::FogOfWar;
use crate::map::{GameMap, SECTOR_COUNT, TilePos};
use crate::sim::classes::FighterClass;
use crate::sim::economy::{
    MatchPhase, Supply, Territory, base_entry, count_classes, in_battle, in_setup, platoon_cost, seconds_left,
};
use crate::sim::fighter::{Condition, Fighter};
use crate::sim::perception::Perception;
use crate::sim::platoon::{Mission, Platoon, PlatoonId};
use crate::sim::vehicle::Vehicle;
use crate::sim::{CommandQueue, SIM_HZ, SimCommand, SimRng, SimSet, SimTick, Unit, UnitClass};
use intel::{Behaviour, EnemyGroup, FIRING_MEMORY_TICKS, Sighting, class_weight, read_enemy};
use planning::{KnownEnemy, Style, choose_style, depth, nearest_ok, plan_setup, reinforcement};

/// The commander re-reads the battlefield this often (2 s).
const THINK_INTERVAL: u64 = 40;
/// Minimum time between signals to one platoon, and before changing its
/// mission, unless it's an emergency (keeps confusion low).
const MIN_SIGNAL_GAP: u64 = 8 * SIM_HZ as u64;
const MIN_MISSION_CHANGE_GAP: u64 = 12 * SIM_HZ as u64;
/// Enemies this close to the King put the commander on the defensive.
const KING_DANGER_RADIUS: f32 = 22.0;

pub struct TermbotPlugin;

impl Plugin for TermbotPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                ai_setup
                    .run_if(resource_exists::<AiCommanders>)
                    .run_if(in_setup)
                    .before(crate::sim::economy::process_setup_commands),
                ai_battle
                    .run_if(resource_exists::<AiCommanders>)
                    .run_if(in_battle)
                    .in_set(SimSet::Commands)
                    .after(crate::sim::economy::accrue_supply)
                    .before(crate::sim::process_commands),
            ),
        );
    }
}

/// The AI commanders in this match (normally just the Termbots).
#[derive(Resource)]
pub struct AiCommanders {
    pub brains: Vec<Brain>,
}

impl AiCommanders {
    pub fn new(factions: &[Faction]) -> Self {
        Self { brains: factions.iter().map(|&f| Brain::new(f)).collect() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Posture {
    Scouting,
    Holding,
    Pushing,
    Defending,
}

impl Posture {
    pub fn name(self) -> &'static str {
        match self {
            Posture::Scouting => "scouting",
            Posture::Holding => "holding",
            Posture::Pushing => "pushing",
            Posture::Defending => "defending the King",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct OrderMemo {
    mission: Mission,
    tick: u64,
}

pub struct Brain {
    pub faction: Faction,
    pub style: Style,
    pub posture: Posture,
    pub placed: bool,
    sightings: HashMap<Entity, Sighting>,
    orders: HashMap<PlatoonId, OrderMemo>,
    /// Each platoon's post to hold when there's nothing else to do.
    homes: HashMap<PlatoonId, TilePos>,
    setup_homes: VecDeque<TilePos>,
    scout: Option<PlatoonId>,
    next_think: u64,
    next_buy: u64,
    king_moved_tick: u64,
    posture_since: u64,
    last_probe_tick: u64,
    /// Current push objective and when it was chosen (kept for a while so
    /// platoons aren't re-signalled every time the estimate wobbles).
    push_target: Option<(TilePos, u64)>,
    /// Our fighters last time we looked, with their strength, to count losses.
    own_units: HashMap<Entity, f32>,
    /// Strength lost on each side that the commander has witnessed.
    pub our_losses: f32,
    pub enemy_losses: f32,
    /// Recent decisions, newest last (shown in the debug view).
    pub log: VecDeque<String>,
}

impl Brain {
    fn new(faction: Faction) -> Self {
        Self {
            faction,
            style: Style::Balanced,
            posture: Posture::Scouting,
            placed: false,
            sightings: HashMap::new(),
            orders: HashMap::new(),
            homes: HashMap::new(),
            setup_homes: VecDeque::new(),
            scout: None,
            next_think: 0,
            next_buy: 30 * SIM_HZ as u64,
            king_moved_tick: 0,
            posture_since: 0,
            last_probe_tick: 0,
            push_target: None,
            own_units: HashMap::new(),
            our_losses: 0.0,
            enemy_losses: 0.0,
            log: VecDeque::new(),
        }
    }

    /// Where the commander believes enemy units are (for tests and debug).
    #[allow(dead_code)]
    pub fn known_enemy_positions(&self) -> Vec<TilePos> {
        self.sightings.values().map(|s| s.pos).collect()
    }

    fn note(&mut self, tick: u64, text: String) {
        let s = tick / SIM_HZ as u64;
        self.log.push_back(format!("{:02}:{:02} {text}", s / 60, s % 60));
        while self.log.len() > 8 {
            self.log.pop_front();
        }
    }
}

fn ai_setup(
    mut ais: ResMut<AiCommanders>,
    phase: Res<MatchPhase>,
    map: Res<GameMap>,
    mut rng: ResMut<SimRng>,
    mut queue: ResMut<CommandQueue>,
) {
    for brain in &mut ais.brains {
        if brain.placed || phase.ready[brain.faction.index()] {
            continue;
        }
        brain.style = choose_style(&mut rng.0);
        let plan = plan_setup(&map, brain.faction, brain.style, &mut rng.0);
        for (classes, anchor) in &plan {
            queue.push(SimCommand::PlacePlatoon { issuer: brain.faction, classes: classes.clone(), anchor: *anchor });
            brain.setup_homes.push_back(*anchor);
        }
        queue.push(SimCommand::Ready { issuer: brain.faction });
        brain.placed = true;
        let text = format!("{} style, {} platoons placed", brain.style.name(), plan.len());
        brain.note(0, text);
    }
}

/// One of our platoons, as the commander sees it.
struct Ours {
    id: PlatoonId,
    name: String,
    mission: Mission,
    target: TilePos,
    pending: bool,
    confusion: f32,
    center: Vec2,
    strength: f32,
    fighters: usize,
    engaged: bool,
}

impl Ours {
    fn center_tile(&self) -> TilePos {
        TilePos::new(self.center.x.round() as i32, self.center.y.round() as i32)
    }
}

fn to_vec(p: TilePos) -> Vec2 {
    Vec2::new(p.x as f32, p.y as f32)
}

fn to_tile(v: Vec2) -> TilePos {
    TilePos::new(v.x.round() as i32, v.y.round() as i32)
}

type UnitView<'a> = (
    Entity,
    &'a Unit,
    &'a Faction,
    &'a TilePos,
    Option<&'a Condition>,
    Option<&'a Perception>,
);

#[allow(clippy::too_many_arguments)]
fn ai_battle(
    tick: Res<SimTick>,
    map: Res<GameMap>,
    fog: Res<FogOfWar>,
    territory: Res<Territory>,
    supply: Res<Supply>,
    mut rng: ResMut<SimRng>,
    mut queue: ResMut<CommandQueue>,
    mut ais: ResMut<AiCommanders>,
    platoons: Query<(&Faction, &Platoon)>,
    units: Query<UnitView>,
    fighters: Query<(&Faction, &Fighter)>,
    vehicles: Query<(&Faction, &Vehicle)>,
) {
    let world = World { tick: tick.0, map: &map, fog: &fog, territory: &territory };
    for brain in &mut ais.brains {
        if tick.0 < brain.next_think {
            continue;
        }
        brain.next_think = tick.0 + THINK_INTERVAL;
        let counts = count_classes(brain.faction, fighters.iter(), vehicles.iter());
        think(brain, &world, supply.of(brain.faction), &counts, &mut rng.0, &mut queue, &platoons, &units);
    }
}

struct World<'a> {
    tick: u64,
    map: &'a GameMap,
    fog: &'a FogOfWar,
    territory: &'a Territory,
}

#[allow(clippy::too_many_arguments)]
fn think(
    brain: &mut Brain,
    w: &World,
    supply: f32,
    counts: &crate::sim::economy::ClassCounts,
    rng: &mut impl Rng,
    queue: &mut CommandQueue,
    platoons: &Query<(&Faction, &Platoon)>,
    units: &Query<UnitView>,
) {
    let me = brain.faction;
    let enemy = me.opponent();
    let tick = w.tick;

    // --- Our side ---
    let Some(our_king) =
        units.iter().find(|u| *u.2 == me && u.1.class == UnitClass::King).map(|u| *u.3)
    else {
        return;
    };
    let mut ours: Vec<Ours> = platoons
        .iter()
        .filter(|(f, _)| **f == me)
        .filter_map(|(_, p)| {
            let members: Vec<UnitView> = p.members.iter().filter_map(|&m| units.get(m).ok()).collect();
            if members.is_empty() {
                return None;
            }
            let n = members.len() as f32;
            Some(Ours {
                id: p.id,
                name: p.name.clone(),
                mission: p.mission,
                target: p.target,
                pending: p.pending.is_some(),
                confusion: p.confusion,
                center: members.iter().map(|m| to_vec(*m.3)).sum::<Vec2>() / n,
                strength: members.iter().map(|m| class_weight(m.1.class)).sum(),
                fighters: members.len(),
                engaged: members.iter().any(|m| m.5.is_some_and(|per| !per.seen_enemies.is_empty())),
            })
        })
        .collect();
    ours.sort_by_key(|p| p.id);

    // Count our own losses since last look.
    let now: HashMap<Entity, f32> = units
        .iter()
        .filter(|u| *u.2 == me && matches!(u.1.class, UnitClass::Fighter(_)))
        .map(|u| (u.0, class_weight(u.1.class)))
        .collect();
    brain.our_losses += brain.own_units.iter().filter(|(e, _)| !now.contains_key(e)).map(|(_, w)| w).sum::<f32>();
    brain.own_units = now;

    // Posts from setup go to the starting platoons, in order.
    for p in &ours {
        if !brain.homes.contains_key(&p.id) {
            let home = brain.setup_homes.pop_front().unwrap_or(p.target);
            brain.homes.insert(p.id, home);
        }
    }

    // --- Look at the enemy (only what our side can see) ---
    let mut visible = HashSet::new();
    for (e, unit, f, pos, cond, _) in units.iter() {
        if *f != enemy || !w.fog.is_explored(me, *pos) {
            continue;
        }
        visible.insert(e);
        let firing = cond.and_then(|c| c.last_shot_tick).is_some_and(|t| tick.saturating_sub(t) <= FIRING_MEMORY_TICKS);
        let s = brain.sightings.entry(e).or_insert(Sighting {
            pos: *pos,
            prev: *pos,
            seen_tick: tick,
            prev_tick: tick,
            class: unit.class,
            firing,
        });
        s.prev = s.pos;
        s.prev_tick = s.seen_tick;
        s.pos = *pos;
        s.seen_tick = tick;
        s.firing = firing;
    }
    // A unit we were watching that no longer exists was killed (a human
    // watching would see it fall); one that walked into unexplored ground
    // just drops out of sight.
    let killed: f32 = brain
        .sightings
        .iter()
        .filter(|(e, s)| !visible.contains(e) && units.get(**e).is_err() && s.class != UnitClass::King)
        .map(|(_, s)| class_weight(s.class))
        .sum();
    brain.enemy_losses += killed;
    brain.sightings.retain(|e, _| visible.contains(e));
    let enemy_king = brain.sightings.values().find(|s| s.class == UnitClass::King).map(|s| s.pos);

    let assets: Vec<TilePos> = ours.iter().map(|p| p.center_tile()).collect();
    let mut groups = read_enemy(&brain.sightings, our_king, &assets);
    let threat = |g: &EnemyGroup| {
        let moving = matches!(g.behaviour, Behaviour::Advancing | Behaviour::Rushing);
        g.strength * (1.0 + 25.0 / (8.0 + g.king_distance)) * if moving { 1.3 } else { 1.0 }
    };
    groups.sort_by(|a, b| threat(b).total_cmp(&threat(a)));
    let king_danger = groups.iter().any(|g| g.king_distance < KING_DANGER_RADIUS && g.strength >= 1.0);

    // --- Posture ---
    // Unseen enemies: assume the enemy is about as strong as we are (same
    // budget, same income), adjusted by the losses we've seen each side take.
    let our_strength: f32 = ours.iter().map(|p| p.strength).sum();
    let enemy_known: f32 = groups.iter().map(|g| g.strength).sum();
    let enemy_estimate = (our_strength + brain.our_losses - brain.enemy_losses).max(enemy_known);
    let edge = our_strength / enemy_estimate.max(1.0);
    let secs = tick / SIM_HZ as u64;
    let held_me = w.territory.held(me);
    let held_enemy = w.territory.held(enemy);
    let push_edge = match brain.style {
        Style::Aggressive => 1.0,
        Style::Balanced => 1.3,
        Style::Defensive => 1.6,
    };
    let late_and_behind = seconds_left(tick) < 240 && held_me <= held_enemy;
    // Even cautious commanders get bolder as the clock runs.
    let bold_by_now = match brain.style {
        Style::Aggressive => secs > 60,
        Style::Balanced => secs > 300,
        Style::Defensive => secs > 480,
    };
    let settled = tick.saturating_sub(brain.posture_since) >= 10 * SIM_HZ as u64;
    let posture = if king_danger {
        Posture::Defending
    } else if secs < 45 {
        Posture::Scouting
    } else if !settled {
        // Don't flip-flop: a posture lasts at least 10 s (unless the King
        // is threatened).
        if brain.posture == Posture::Defending || brain.posture == Posture::Scouting { Posture::Holding } else { brain.posture }
    } else if late_and_behind
        || (brain.posture == Posture::Pushing && edge >= push_edge * 0.85)
        || edge >= push_edge && (brain.style == Style::Aggressive || secs > 90)
        || (bold_by_now && edge >= 1.0)
    {
        Posture::Pushing
    } else {
        Posture::Holding
    };
    if posture != brain.posture {
        brain.posture = posture;
        brain.posture_since = tick;
        brain.note(tick, format!("posture: {} (edge {:.2})", posture.name(), edge));
    }

    // --- Answer each enemy group with the nearest platoons ---
    let mut busy: HashSet<PlatoonId> = HashSet::new();
    for g in &groups {
        let near_king = g.king_distance < KING_DANGER_RADIUS;
        let need = if near_king { 3 } else { (g.strength / 3.5).ceil().clamp(1.0, 3.0) as usize };
        let mut candidates: Vec<&Ours> =
            ours.iter().filter(|p| !busy.contains(&p.id) && p.center.distance(g.center) < 30.0).collect();
        candidates.sort_by(|a, b| a.center.distance(g.center).total_cmp(&b.center.distance(g.center)));
        for p in candidates.into_iter().take(need) {
            busy.insert(p.id);
            if let Some((mission, target, why)) = counter(brain, w, rng, p, g, posture, edge) {
                issue(brain, queue, w, p, mission, target, near_king, why);
            }
        }
    }

    // --- Jobs for idle platoons ---
    // Clearly winning and the enemy King is in sight: go for the King.
    let king_guard: f32 = enemy_king
        .map(|k| groups.iter().filter(|g| g.center.distance(to_vec(k)) < 15.0).map(|g| g.strength).sum())
        .unwrap_or(f32::MAX);
    let hunt_king = posture == Posture::Pushing && enemy_king.is_some() && (edge >= 1.8 || king_guard < 2.0);
    let push_target = match brain.push_target {
        _ if hunt_king => {
            let k = enemy_king.unwrap_or(our_king);
            if brain.push_target.is_none_or(|(t, _)| t.distance(k) > 6.0) {
                brain.note(tick, "going for the enemy King".into());
            }
            brain.push_target = Some((k, tick));
            k
        }
        Some((t, since))
            if tick.saturating_sub(since) < 40 * SIM_HZ as u64
                && !ours.iter().any(|p| p.center.distance(to_vec(t)) < 6.0)
                && !(w.territory.owner[crate::map::sector_index(t)] == Some(me) && held_me < SECTOR_COUNT) =>
        {
            // Keep the objective until it's reached, taken, or stale.
            t
        }
        _ => {
            let t = pick_push_target(w, me, &groups, &ours, enemy_king);
            brain.push_target = Some((t, tick));
            t
        }
    };
    let guard = if ours.len() >= 3 {
        ours.iter()
            .filter(|p| !busy.contains(&p.id))
            .min_by(|a, b| a.center.distance(to_vec(our_king)).total_cmp(&b.center.distance(to_vec(our_king))))
            .map(|p| p.id)
    } else {
        None
    };
    let lost_home = lost_home_sector(w, me);
    let mut retaker_sent = false;
    for p in ours.iter().filter(|p| !busy.contains(&p.id)) {
        let home = brain.homes.get(&p.id).copied().unwrap_or(p.target);
        match posture {
            Posture::Scouting if brain.scout.is_none() && p.fighters <= 3 => {
                if let Some(t) = scout_target(w, me, our_king) {
                    brain.scout = Some(p.id);
                    issue(brain, queue, w, p, Mission::Reposition, t, false, "scouting ahead".into());
                }
            }
            Posture::Pushing if Some(p.id) != guard => {
                if hunt_king && p.center.distance(to_vec(push_target)) > 12.0 {
                    // Sprint to catch a running King, then assault.
                    issue(brain, queue, w, p, Mission::Reposition, push_target, false, "running down the King".into());
                } else {
                    issue(brain, queue, w, p, Mission::Assault, push_target, false, "joining the push".into());
                }
            }
            Posture::Holding
                if tick.saturating_sub(brain.last_probe_tick) > 100 * SIM_HZ as u64
                    && !p.engaged
                    && Some(p.id) != guard
                    && p.fighters <= 4 =>
            {
                // Probe: send a platoon forward to find out what's there.
                let probe = pick_push_target(w, me, &groups, &ours, enemy_king);
                brain.last_probe_tick = tick;
                issue(brain, queue, w, p, Mission::Reposition, probe, false, "probing their line".into());
            }
            Posture::Defending => {
                if p.center.distance(to_vec(our_king)) > 12.0 {
                    let t = nearest_ok(w.map, our_king, |t| w.map.tile(t).passable()).unwrap_or(our_king);
                    issue(brain, queue, w, p, Mission::Reposition, t, true, "rushing back to the King".into());
                }
            }
            _ => {
                if let (Some(sector_center), false) = (lost_home, retaker_sent)
                    && !p.engaged
                    && p.center.distance(to_vec(sector_center)) < 40.0
                    && Some(p.id) != guard
                {
                    retaker_sent = true;
                    issue(brain, queue, w, p, Mission::Assault, sector_center, false, "retaking our sector".into());
                } else if Some(p.id) != brain.scout {
                    issue(brain, queue, w, p, Mission::Overwatch, home, false, "holding its post".into());
                }
            }
        }
    }

    // --- King safety ---
    if king_danger && tick.saturating_sub(brain.king_moved_tick) > 6 * SIM_HZ as u64 {
        let danger = groups.iter().filter(|g| g.king_distance < KING_DANGER_RADIUS).map(|g| g.center).next();
        if let Some(danger) = danger {
            let away = (to_vec(our_king) - danger).normalize_or_zero() * 12.0;
            let spot = to_tile(to_vec(our_king) + away);
            if let Some(t) = nearest_ok(w.map, spot, |t| w.map.in_home_half(me, t)) {
                let king_id = units.iter().find(|u| *u.2 == me && u.1.class == UnitClass::King).map(|u| u.1.id);
                if let Some(unit) = king_id {
                    queue.push(SimCommand::Move { issuer: me, unit, target: t });
                    brain.king_moved_tick = tick;
                    brain.note(tick, "King moves away from the threat".into());
                }
            }
        }
    }

    // --- Reinforcements ---
    buy(brain, w, supply, counts, queue, &groups, &ours, our_king, king_danger, push_target, edge);
}

/// The counter to what an enemy group seems to be doing.
fn counter(
    brain: &Brain,
    w: &World,
    rng: &mut impl Rng,
    p: &Ours,
    g: &EnemyGroup,
    posture: Posture,
    edge: f32,
) -> Option<(Mission, TilePos, String)> {
    // Overall dominance counts too: a winning army doesn't tiptoe around
    // a couple of defenders.
    let overwhelming = edge >= 1.8 && g.strength <= p.strength * 2.0;
    let stronger = p.strength >= g.strength * 1.15 || overwhelming;
    let bold = rng.random::<f32>() < brain.style.unpredictability();
    let n = g.members.len();
    let what = format!("{n} enemies {}", g.behaviour.name());
    let passable = |t: TilePos| w.map.tile(t).passable();
    let spot = |v: Vec2| nearest_ok(w.map, to_tile(v), passable);
    let toward = (g.center - p.center).normalize_or_zero();

    // A platoon already dug in and trading fire holds unless it's losing.
    if p.engaged && p.mission == Mission::Overwatch && matches!(g.behaviour, Behaviour::Firing | Behaviour::Holding) {
        return None;
    }

    let hold = spot(p.center + toward * 2.0)?;
    Some(match g.behaviour {
        Behaviour::Advancing => {
            if stronger && bold {
                (Mission::Assault, g.center_tile(), format!("{what}: meeting them head-on"))
            } else {
                (Mission::Overwatch, hold, format!("{what}: digging in"))
            }
        }
        Behaviour::Rushing => {
            let ahead = spot(g.center + g.velocity * 3.0)?;
            (Mission::Assault, ahead, format!("{what}: catching them in the open"))
        }
        Behaviour::Firing if overwhelming => {
            (Mission::Assault, g.center_tile(), format!("{what}: overrunning them"))
        }
        Behaviour::Firing => {
            if stronger || bold {
                // Flank: come at them from the side, out of their lane.
                let away = (p.center - g.center).normalize_or_zero();
                let perp = Vec2::new(-away.y, away.x);
                let a = g.center + perp * 10.0 + away * 4.0;
                let b = g.center - perp * 10.0 + away * 4.0;
                let side = if a.distance(p.center) <= b.distance(p.center) { a } else { b };
                (Mission::Reposition, spot(side)?, format!("{what}: flanking their firing line"))
            } else {
                let back = spot(p.center - toward * 8.0)?;
                (Mission::Reposition, back, format!("{what}: slipping out of their lane"))
            }
        }
        Behaviour::Holding => {
            if stronger || posture == Posture::Pushing {
                (Mission::Assault, g.center_tile(), format!("{what}: attacking while they sit"))
            } else {
                (Mission::Overwatch, hold, format!("{what}: watching them"))
            }
        }
        Behaviour::Retreating => {
            if stronger {
                (Mission::Assault, g.center_tile(), format!("{what}: pressing them"))
            } else {
                (Mission::Overwatch, hold, format!("{what}: letting them go"))
            }
        }
    })
}

/// Send a signal, unless it would just add confusion: repeats, orders too
/// soon after the last one, or a platoon that's already confused.
#[allow(clippy::too_many_arguments)]
fn issue(
    brain: &mut Brain,
    queue: &mut CommandQueue,
    w: &World,
    p: &Ours,
    mission: Mission,
    target: TilePos,
    emergency: bool,
    why: String,
) {
    let tick = w.tick;
    if p.pending || !target.in_bounds() {
        return;
    }
    let tolerance = if mission == Mission::Assault { 7.0 } else { 4.0 };
    if p.mission == mission && p.target.distance(target) <= tolerance {
        return;
    }
    if let Some(last) = brain.orders.get(&p.id) {
        let since = tick.saturating_sub(last.tick);
        if emergency {
            if since < 3 * SIM_HZ as u64 {
                return;
            }
        } else if since < MIN_SIGNAL_GAP || (last.mission != mission && since < MIN_MISSION_CHANGE_GAP) {
            return;
        }
    }
    if p.confusion >= 30.0 && !emergency {
        return;
    }
    queue.push(SimCommand::Signal { issuer: brain.faction, platoon: p.id, mission, target });
    brain.orders.insert(p.id, OrderMemo { mission, tick });
    brain.note(tick, format!("{}: {} ({why})", p.name, mission.name()));
}

fn sector_center_tile(w: &World, id: usize) -> TilePos {
    let s = &w.map.sectors[id];
    let c = TilePos::new((s.min.x + s.max.x) / 2, (s.min.y + s.max.y) / 2);
    nearest_ok(w.map, c, |t| w.map.tile(t).passable()).unwrap_or(c)
}

/// Where to push: the enemy-held sector that looks weakest and closest.
fn pick_push_target(w: &World, me: Faction, groups: &[EnemyGroup], ours: &[Ours], enemy_king: Option<TilePos>) -> TilePos {
    let our_center = if ours.is_empty() {
        to_vec(w.map.spawn(me))
    } else {
        ours.iter().map(|p| p.center).sum::<Vec2>() / ours.len() as f32
    };
    let best = (0..SECTOR_COUNT)
        .filter(|&s| w.territory.owner[s] != Some(me))
        .map(|s| {
            let c = sector_center_tile(w, s);
            let defenders: f32 =
                groups.iter().filter(|g| crate::map::sector_index(g.center_tile()) == s).map(|g| g.strength).sum();
            (defenders * 2.0 + to_vec(c).distance(our_center) / 20.0, c)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, c)| c);
    best.or(enemy_king).or_else(|| search_target(w, me, our_center)).unwrap_or_else(|| w.map.spawn(me.opponent()))
}

/// The nearest unexplored ground in the enemy's half (where the enemy
/// King might be hiding).
fn search_target(w: &World, me: Faction, from: Vec2) -> Option<TilePos> {
    let mut best: Option<(f32, TilePos)> = None;
    for y in (0..crate::map::MAP_H).step_by(4) {
        for x in (0..crate::map::MAP_W).step_by(4) {
            let p = TilePos::new(x, y);
            if w.map.in_home_half(me, p) || w.fog.is_explored(me, p) || !w.map.tile(p).passable() {
                continue;
            }
            let d = to_vec(p).distance(from);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// One of our own home sectors the enemy has taken, if any.
fn lost_home_sector(w: &World, me: Faction) -> Option<TilePos> {
    w.map
        .sectors
        .iter()
        .find(|s| s.home_of == me && w.territory.owner[s.id] == Some(me.opponent()))
        .map(|s| sector_center_tile(w, s.id))
}

/// The nearest enemy-half sector we haven't explored yet.
fn scout_target(w: &World, me: Faction, our_king: TilePos) -> Option<TilePos> {
    w.map
        .sectors
        .iter()
        .filter(|s| s.home_of != me)
        .map(|s| sector_center_tile(w, s.id))
        .filter(|c| !w.fog.is_explored(me, *c))
        .min_by(|a, b| a.distance(our_king).total_cmp(&b.distance(our_king)))
        .map(|c| {
            // Stop at the edge of what we know, so the target is reachable
            // knowledge-wise; the platoon's own eyes do the rest.
            let dir = (to_vec(c) - to_vec(our_king)).normalize_or_zero();
            let mut t = our_king;
            for i in 1..120 {
                let p = to_tile(to_vec(our_king) + dir * i as f32);
                if !p.in_bounds() {
                    break;
                }
                t = p;
                if !w.fog.is_explored(me, p) {
                    break;
                }
            }
            nearest_ok(w.map, t, |p| w.map.tile(p).passable()).unwrap_or(t)
        })
}

#[allow(clippy::too_many_arguments)]
fn buy(
    brain: &mut Brain,
    w: &World,
    supply: f32,
    counts: &crate::sim::economy::ClassCounts,
    queue: &mut CommandQueue,
    groups: &[EnemyGroup],
    ours: &[Ours],
    our_king: TilePos,
    king_danger: bool,
    push_target: TilePos,
    edge: f32,
) {
    let tick = w.tick;
    let losing_badly = edge < 0.6 && !king_danger;
    if tick < brain.next_buy && !(king_danger && tick + 20 * SIM_HZ as u64 > brain.next_buy) {
        return;
    }
    let me = brain.faction;

    let mut known = KnownEnemy::default();
    for s in brain.sightings.values() {
        match s.class {
            UnitClass::Fighter(FighterClass::Assaulter) => known.assaulters += 1,
            UnitClass::Fighter(FighterClass::Sniper) => known.snipers += 1,
            UnitClass::Fighter(FighterClass::HeavyWeapons) => known.heavies += 1,
            UnitClass::Fighter(FighterClass::Juggernaut) => known.juggernauts += 1,
            _ => {}
        }
    }
    let mut classes = reinforcement(brain.style, &known, supply);

    // Where they're needed most.
    let (spot, why) = if losing_badly {
        // Regroup: don't feed small groups into a stronger army.
        (to_vec(our_king), "to regroup by the King")
    } else if king_danger {
        let threat = groups.first().map(|g| g.center).unwrap_or(to_vec(our_king));
        (to_vec(our_king) + (threat - to_vec(our_king)).normalize_or_zero() * 4.0, "to defend the King")
    } else if let Some(g) = groups.first() {
        // Behind our line, never on top of the enemy.
        let back = (to_vec(our_king) - g.center).normalize_or_zero();
        let reach = g.center.distance(to_vec(our_king));
        (g.center + back * (reach * 0.5).clamp(10.0, 25.0), "to back up the front")
    } else if brain.posture == Posture::Pushing {
        let staging = (to_vec(push_target) + to_vec(our_king)) / 2.0;
        (staging, "to join the push")
    } else {
        // The post with the fewest of our platoons near it.
        let weakest = brain
            .homes
            .values()
            .min_by_key(|h| ours.iter().filter(|p| p.center.distance(to_vec(**h)) < 12.0).count())
            .copied()
            .unwrap_or(our_king);
        (to_vec(weakest), "to hold the line")
    };
    let Some(dest) = nearest_ok(w.map, to_tile(spot), |t| w.fog.is_explored(me, t) && w.map.tile(t).passable()) else {
        return;
    };
    let vehicle = king_danger || (!losing_badly && base_entry(w.map, me).distance(dest) > 45.0);
    let reserve = if king_danger { 0.0 } else { 80.0 };
    // A lone fighter rarely survives: wait for at least two, unless the
    // King needs anyone now.
    let min_size = if king_danger {
        1
    } else if losing_badly {
        4
    } else {
        2
    };
    if losing_badly {
        while classes.len() < 4 {
            classes.push(FighterClass::Assaulter);
        }
    }
    while classes.len() > min_size && platoon_cost(&classes, counts, vehicle) + reserve > supply {
        classes.pop();
    }
    let cost = platoon_cost(&classes, counts, vehicle);
    if cost + reserve > supply {
        brain.next_buy = tick + THINK_INTERVAL;
        return;
    }
    queue.push(SimCommand::Deploy { issuer: me, classes: classes.clone(), vehicle, target: dest });
    brain.next_buy = tick + 10 * SIM_HZ as u64;
    let letters: Vec<&str> = classes.iter().map(|c| c.stats().letter).collect();
    let how = if vehicle { "by vehicle" } else { "on foot" };
    let d = depth(me, dest);
    brain.note(tick, format!("buys {} {how} {why} (depth {d})", letters.join("")));
}
