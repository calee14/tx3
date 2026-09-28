//! Termbots vs Humans: a real-time 2D strategy commander game.
//!
//! Steps 1-7 of the build order: a procedurally generated, balanced map,
//! fog of war, and the fixed-timestep simulation; platoons of 1-5 fighters
//! with rolled traits, missions, signals and confusion, and autonomous
//! fighters; Supply, a hidden setup phase, reinforcements on foot or by
//! vehicle, territory and the 15-minute timer; engineers, hidden traps,
//! turrets, artillery and air strikes; loadouts, damage and armor types,
//! destructible terrain and veterancy; and the Termbot AI commander.

mod army_panel;
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
mod termbot;
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
            termbot::TermbotPlugin,
            army_panel::ArmyPanelPlugin,
        ))
        .add_systems(
            Update,
            (
                map::render::draw_sector_borders,
                map::render::color_sector_labels,
                map::render::repaint_changed_terrain.run_if(resource_exists::<map::GameMap>),
            ),
        )
        .run();
}
