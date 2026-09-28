//! Fighter components: health, and the condition that changes during
//! battle (morale, suppression, what it has decided to do).

use bevy::prelude::*;

use super::classes::{FighterClass, Traits};
use crate::map::{CoverLevel, TilePos};

pub const SUPPRESSED: f32 = 35.0;
pub const PINNED: f32 = 75.0;
pub const SHAKEN: f32 = 40.0;
pub const BROKEN: f32 = 20.0;
pub const BRAVE: f32 = 85.0;

#[derive(Component, Debug)]
pub struct Fighter {
    pub class: FighterClass,
    /// The platoon entity this fighter belongs to.
    pub platoon: Entity,
    /// Position in the platoon's formation.
    pub slot: usize,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self { current: max, max }
    }
    pub fn frac(&self) -> f32 {
        (self.current / self.max).clamp(0.0, 1.0)
    }
}

/// What a fighter has decided to do right now. Chosen autonomously by the
/// fighter (see `ai.rs`), within its platoon's mission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Nothing to do: staying put.
    Holding,
    /// Moving toward its place in the mission.
    Advancing,
    /// Standing its ground and shooting.
    Fighting,
    /// Running for the nearest good cover.
    TakingCover,
    /// Morale broke: falling back toward its King.
    Retreating,
    /// Platoon too confused by signals to act on orders.
    Frozen,
    /// Getting out from under an incoming strike.
    Dodging,
    /// Engineer building a gadget.
    Building,
    /// Engineer defusing an enemy trap.
    Defusing,
}

impl Decision {
    pub fn name(self) -> &'static str {
        match self {
            Decision::Holding => "Holding",
            Decision::Advancing => "Advancing",
            Decision::Fighting => "Fighting",
            Decision::TakingCover => "Taking cover",
            Decision::Retreating => "Retreating",
            Decision::Frozen => "Frozen (orders unclear)",
            Decision::Dodging => "Dodging an incoming strike",
            Decision::Building => "Building",
            Decision::Defusing => "Defusing a trap",
        }
    }
}

/// Battle state that changes moment to moment.
#[derive(Component, Debug)]
pub struct Condition {
    /// 0-100. Low morale means fear; below `BROKEN` the fighter flees.
    pub morale: f32,
    /// 0-100. Incoming fire. Lowers accuracy; above `PINNED` the fighter
    /// won't leave cover.
    pub suppression: f32,
    /// Seconds until the weapon is ready.
    pub fire_cooldown: f32,
    pub decision: Decision,
    /// The tile it's heading for, if any.
    pub goal: Option<TilePos>,
    /// Last enemy it shot at.
    pub fire_target: Option<Entity>,
    /// Tick of its next decision. Smarter fighters think more often.
    pub next_think: u64,
    /// Cover it has right now against the threats it knows about
    /// (directional; updated when it re-thinks).
    pub cover: CoverLevel,
    /// Tick it last fired. Firing is visible to anyone watching (tracers),
    /// so the enemy commander may use it.
    pub last_shot_tick: Option<u64>,
    /// An engineer's current job (building or defusing), if any.
    pub work: Option<Work>,
}

/// An engineer job.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Work {
    pub kind: WorkKind,
    /// Where the work happens (build site, or the trap being defused).
    pub tile: TilePos,
    /// 0 to 1.
    pub progress: f32,
    /// Supply paid for a build (refunded if the build is abandoned).
    pub paid: f32,
    /// Seconds spent unable to reach the work site (gives up after a few).
    pub idle: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WorkKind {
    Build(super::gadgets::GadgetKind),
    Defuse(Entity),
    /// Helping the platoon's lead engineer (this entity) with its build.
    Assist(Entity),
}

impl Condition {
    pub fn new(traits: &Traits) -> Self {
        Self {
            morale: morale_baseline(traits),
            suppression: 0.0,
            fire_cooldown: 0.0,
            decision: Decision::Holding,
            goal: None,
            fire_target: None,
            next_think: 0,
            cover: CoverLevel::None,
            last_shot_tick: None,
            work: None,
        }
    }
}

/// Experience from fighting. Enough of it earns a rank, which raises every
/// trait by 5 (up to 100).
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct Veterancy {
    pub xp: f32,
    pub rank: u8,
}

/// XP needed for each rank above Recruit.
pub const RANK_XP: [f32; 2] = [8.0, 20.0];
pub const RANK_TRAIT_BONUS: u8 = 5;

impl Veterancy {
    pub fn rank_name(&self) -> &'static str {
        match self.rank {
            0 => "Recruit",
            1 => "Veteran",
            _ => "Elite",
        }
    }

    /// Add XP; returns true if the fighter ranked up (traits improved).
    pub fn gain(&mut self, xp: f32, traits: &mut Traits) -> bool {
        self.xp += xp;
        let mut ranked = false;
        while (self.rank as usize) < RANK_XP.len() && self.xp >= RANK_XP[self.rank as usize] {
            self.rank += 1;
            ranked = true;
            for t in [
                &mut traits.accuracy,
                &mut traits.iq,
                &mut traits.tactics,
                &mut traits.bravery,
                &mut traits.leadership,
            ] {
                *t = t.saturating_add(RANK_TRAIT_BONUS).min(100);
            }
        }
        ranked
    }
}

/// XP earned this tick, applied by `apply_experience`.
#[derive(Resource, Default, Debug)]
pub struct XpLog(pub Vec<(Entity, f32)>);

pub fn apply_experience(mut log: ResMut<XpLog>, mut fighters: Query<(&mut Veterancy, &mut Traits)>) {
    for (entity, xp) in log.0.drain(..) {
        if let Ok((mut vet, mut traits)) = fighters.get_mut(entity) {
            vet.gain(xp, &mut traits);
        }
    }
}

/// Resting morale: braver fighters sit higher.
pub fn morale_baseline(traits: &Traits) -> f32 {
    55.0 + 35.0 * traits.brav()
}

/// Everything a player can see about a fighter's state, most important
/// first. Used for the hover tooltip and the icons over its head.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Broken,
    Frozen,
    Pinned,
    Confused,
    Suppressed,
    BadlyInjured,
    Injured,
    Shaken,
    Brave,
    Moving,
    InCover(CoverLevel),
    Veteran,
    Elite,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Broken => "Broken (fleeing)",
            Status::Frozen => "Frozen",
            Status::Pinned => "Pinned",
            Status::Confused => "Confused",
            Status::Suppressed => "Suppressed",
            Status::BadlyInjured => "Badly injured",
            Status::Injured => "Injured",
            Status::Shaken => "Shaken",
            Status::Brave => "Brave",
            Status::Moving => "Moving (less accurate)",
            Status::InCover(CoverLevel::Heavy) => "In heavy cover",
            Status::InCover(_) => "In light cover",
            Status::Veteran => "Veteran",
            Status::Elite => "Elite",
        }
    }

    /// Short overhead icon, or None if this status has no icon.
    pub fn icon(self) -> Option<(&'static str, Color)> {
        match self {
            Status::Broken => Some(("RUN", Color::srgb(1.0, 1.0, 1.0))),
            Status::Frozen => Some(("??", Color::srgb(1.0, 0.85, 0.2))),
            Status::Pinned => Some(("!!", Color::srgb(1.0, 0.45, 0.1))),
            Status::Confused => Some(("?", Color::srgb(1.0, 0.85, 0.2))),
            Status::Suppressed => Some(("!", Color::srgb(1.0, 0.6, 0.2))),
            Status::BadlyInjured => Some(("++", Color::srgb(1.0, 0.25, 0.25))),
            Status::Injured => Some(("+", Color::srgb(1.0, 0.4, 0.4))),
            Status::Shaken => Some(("~", Color::srgb(0.7, 0.7, 1.0))),
            Status::Brave => Some(("^", Color::srgb(0.5, 0.9, 1.0))),
            Status::Elite => Some(("**", Color::srgb(1.0, 0.85, 0.2))),
            Status::Veteran => Some(("*", Color::srgb(1.0, 0.85, 0.2))),
            _ => None,
        }
    }

    /// Visible to the enemy too (the rest needs intel on the fighter).
    pub fn visible_to_enemy(self) -> bool {
        matches!(
            self,
            Status::Broken
                | Status::Pinned
                | Status::Injured
                | Status::BadlyInjured
                | Status::Moving
                | Status::InCover(_)
                | Status::Veteran
                | Status::Elite
        )
    }
}

pub fn statuses(
    condition: &Condition,
    health: &Health,
    platoon_confusion: f32,
    moving: bool,
    cover: CoverLevel,
    rank: u8,
) -> Vec<Status> {
    let mut out = Vec::new();
    if condition.morale < BROKEN {
        out.push(Status::Broken);
    }
    if condition.decision == Decision::Frozen {
        out.push(Status::Frozen);
    }
    if condition.suppression >= PINNED {
        out.push(Status::Pinned);
    }
    if platoon_confusion >= super::platoon::CONFUSION_MEDIUM && condition.decision != Decision::Frozen {
        out.push(Status::Confused);
    }
    if (SUPPRESSED..PINNED).contains(&condition.suppression) {
        out.push(Status::Suppressed);
    }
    if health.frac() < 0.3 {
        out.push(Status::BadlyInjured);
    } else if health.frac() < 0.6 {
        out.push(Status::Injured);
    }
    if (BROKEN..SHAKEN).contains(&condition.morale) {
        out.push(Status::Shaken);
    }
    if condition.morale >= BRAVE {
        out.push(Status::Brave);
    }
    if moving {
        out.push(Status::Moving);
    }
    if cover != CoverLevel::None {
        out.push(Status::InCover(cover));
    }
    match rank {
        0 => {}
        1 => out.push(Status::Veteran),
        _ => out.push(Status::Elite),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_raise_traits() {
        let mut traits = Traits { accuracy: 50, iq: 98, tactics: 40, bravery: 60, leadership: 30 };
        let mut vet = Veterancy::default();
        assert!(!vet.gain(5.0, &mut traits));
        assert!(vet.gain(4.0, &mut traits), "8 XP makes a Veteran");
        assert_eq!(vet.rank_name(), "Veteran");
        assert_eq!(traits.accuracy, 55);
        assert_eq!(traits.iq, 100, "capped at 100");
        assert!(vet.gain(20.0, &mut traits));
        assert_eq!(vet.rank_name(), "Elite");
        assert_eq!(traits.tactics, 50);
    }
}
