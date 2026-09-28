use bevy::prelude::*;

/// The two sides of every match. Humans start at the bottom of the map,
/// Termbots at the top.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Faction {
    Humans,
    Termbots,
}

impl Faction {
    pub const ALL: [Faction; 2] = [Faction::Humans, Faction::Termbots];

    /// Stable index for per-faction arrays (fog, supply, ...).
    pub fn index(self) -> usize {
        match self {
            Faction::Humans => 0,
            Faction::Termbots => 1,
        }
    }

    pub fn opponent(self) -> Faction {
        match self {
            Faction::Humans => Faction::Termbots,
            Faction::Termbots => Faction::Humans,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Faction::Humans => "Humans",
            Faction::Termbots => "Termbots",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Faction::Humans => Color::srgb(0.25, 0.55, 1.0),
            Faction::Termbots => Color::srgb(1.0, 0.3, 0.25),
        }
    }
}

/// Which faction this machine controls. Single-player: always Humans.
/// In multiplayer each client gets its own value.
#[derive(Resource, Clone, Copy, Debug)]
pub struct LocalPlayer(pub Faction);
