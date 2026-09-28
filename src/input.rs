//! Player input. Input never changes the simulation directly: it turns
//! clicks into `SimCommand`s on the command queue.
//!
//! Step 1 controls: left-click selects your King, right-click orders it to
//! move. (Missions and platoon signals arrive in steps 2-3.)

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::faction::{Faction, LocalPlayer};
use crate::fog::FogDebug;
use crate::map::{TILE_SIZE, TilePos};
use crate::sim::{CommandQueue, Mover, SimCommand, Unit};

const SELECT_RADIUS: f32 = 14.0;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorWorld>()
            .init_resource::<Selection>()
            .add_systems(
                Update,
                (
                    update_cursor,
                    (select_and_command, draw_selection).run_if(resource_exists::<CommandQueue>),
                    draw_hover,
                    toggle_pause,
                    toggle_fog_debug,
                )
                    .chain(),
            );
    }
}

/// World position under the mouse cursor, if it's over the window.
#[derive(Resource, Default)]
pub struct CursorWorld(pub Option<Vec2>);

impl CursorWorld {
    pub fn tile(&self) -> Option<TilePos> {
        self.0.and_then(TilePos::from_world)
    }
}

#[derive(Resource, Default)]
pub struct Selection(pub Option<Entity>);

fn update_cursor(
    mut cursor: ResMut<CursorWorld>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    let (camera, camera_transform) = *camera;
    cursor.0 = window
        .cursor_position()
        .and_then(|screen| camera.viewport_to_world_2d(camera_transform, screen).ok());
}

fn select_and_command(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<CursorWorld>,
    local: Res<LocalPlayer>,
    mut selection: ResMut<Selection>,
    mut queue: ResMut<CommandQueue>,
    units: Query<(Entity, &Unit, &Faction, &Transform)>,
) {
    let Some(world) = cursor.0 else { return };

    if buttons.just_pressed(MouseButton::Left) {
        selection.0 = units
            .iter()
            .filter(|(_, _, faction, _)| **faction == local.0)
            .map(|(entity, _, _, t)| (entity, t.translation.truncate().distance(world)))
            .filter(|&(_, d)| d <= SELECT_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(entity, _)| entity);
    }

    if buttons.just_pressed(MouseButton::Right) {
        let (Some(selected), Some(target)) = (selection.0, TilePos::from_world(world)) else { return };
        if let Ok((_, unit, _, _)) = units.get(selected) {
            queue.push(SimCommand::Move { issuer: local.0, unit: unit.id, target });
        }
    }
}

fn draw_selection(
    selection: Res<Selection>,
    units: Query<(&Transform, &Mover)>,
    mut gizmos: Gizmos,
) {
    let Some(selected) = selection.0 else { return };
    let Ok((transform, mover)) = units.get(selected) else { return };
    let here = transform.translation.truncate();
    gizmos.circle_2d(Isometry2d::from_translation(here), 11.0, Color::srgb(1.0, 0.95, 0.3));

    // Planned path.
    let points: Vec<Vec2> = std::iter::once(here).chain(mover.path.iter().map(|p| p.to_world())).collect();
    if points.len() > 1 {
        gizmos.linestrip_2d(points, Color::srgba(1.0, 0.95, 0.3, 0.6));
    }
}

fn draw_hover(cursor: Res<CursorWorld>, mut gizmos: Gizmos) {
    if let Some(tile) = cursor.tile() {
        gizmos.rect_2d(
            Isometry2d::from_translation(tile.to_world()),
            Vec2::splat(TILE_SIZE),
            Color::srgba(1.0, 1.0, 1.0, 0.7),
        );
    }
}

/// Space: pause / resume. Single-player only; there is no pause in
/// multiplayer. Orders can still be given while paused.
fn toggle_pause(keys: Res<ButtonInput<KeyCode>>, mut time: ResMut<Time<Virtual>>) {
    if keys.just_pressed(KeyCode::Space) {
        if time.is_paused() {
            time.unpause();
        } else {
            time.pause();
        }
    }
}

/// F: debug toggle to see the whole map through the fog.
fn toggle_fog_debug(keys: Res<ButtonInput<KeyCode>>, mut debug: ResMut<FogDebug>) {
    if keys.just_pressed(KeyCode::KeyF) {
        debug.show_all = !debug.show_all;
    }
}
