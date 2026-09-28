//! Termbots vs Humans: a real-time 2D strategy commander game.
//!
//! Steps 1-3 of the build order: a procedurally generated, mirrored map,
//! top-down camera, fog of war, and the fixed-timestep simulation; mixed
//! platoons of fighters with rolled traits, missions, signals and
//! confusion, and autonomous fighters that move, shoot and take cover on
//! their own.

mod camera;
mod commander;
mod faction;
mod fog;
mod game;
mod hud;
mod input;
mod map;
mod pathfinding;
mod sim;
mod units;

use bevy::prelude::*;

use faction::{Faction, LocalPlayer};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Termbots vs Humans".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.05, 0.07)))
        .insert_resource(LocalPlayer(Faction::Humans))
        .add_plugins((
            sim::SimPlugin,
            fog::FogPlugin,
            camera::CameraPlugin,
            input::InputPlugin,
            hud::HudPlugin,
            game::GamePlugin,
            units::UnitVisualsPlugin,
            commander::CommanderPlugin,
        ))
        .add_systems(Update, map::render::draw_sector_borders)
        .run();
}
