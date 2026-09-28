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
            _ => None,
        }
    }

    /// Visible to the enemy too (the rest needs intel on the fighter).
    pub fn visible_to_enemy(self) -> bool {
        matches!(
            self,
            Status::Broken | Status::Pinned | Status::Injured | Status::BadlyInjured | Status::Moving | Status::InCover(_)
        )
    }
}

pub fn statuses(
    condition: &Condition,
    health: &Health,
    platoon_confusion: f32,
    moving: bool,
    cover: CoverLevel,
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
    out
}
