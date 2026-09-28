//! Top-down camera: WASD / arrow keys or middle-mouse drag to pan, mouse
//! wheel to zoom. Uses real time, so it keeps working while paused.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::map::{MAP_H, MAP_W, TILE_SIZE};

const PAN_SPEED: f32 = 900.0; // world units per second at zoom 1
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 2.0;
const START_ZOOM: f32 = 0.8;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraFocusRequest>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, (focus_camera, pan_and_zoom).chain());
    }
}

/// Set this to jump the camera to a world position (used at match start).
#[derive(Resource, Default)]
pub struct CameraFocusRequest(pub Option<Vec2>);

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection { scale: START_ZOOM, ..OrthographicProjection::default_2d() }),
    ));
}

fn focus_camera(mut request: ResMut<CameraFocusRequest>, mut camera: Single<&mut Transform, With<Camera2d>>) {
    if let Some(target) = request.0.take() {
        // Look a little ahead of the base, toward the enemy.
        camera.translation.x = target.x;
        camera.translation.y = target.y + 200.0;
    }
}

fn pan_and_zoom(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    camera: Single<(&mut Transform, &mut Projection), With<Camera2d>>,
    window: Single<&Window, With<PrimaryWindow>>,
) {
    let (mut transform, mut projection) = camera.into_inner();
    let Projection::Orthographic(ortho) = projection.as_mut() else { return };

    // Zoom
    if scroll.delta.y != 0.0 {
        let steps = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        ortho.scale = (ortho.scale * (1.0 - steps * 0.1)).clamp(MIN_ZOOM, MAX_ZOOM);
    }

    // Keyboard pan
    let mut dir = Vec2::ZERO;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        dir.y += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        dir.y -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        dir.x -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        dir.x += 1.0;
    }
    let delta = dir.normalize_or_zero() * PAN_SPEED * ortho.scale * time.delta_secs();
    transform.translation += delta.extend(0.0);

    // Middle-mouse drag pan (screen y points down, world y points up).
    if buttons.pressed(MouseButton::Middle) {
        transform.translation.x -= motion.delta.x * ortho.scale;
        transform.translation.y += motion.delta.y * ortho.scale;
    }

    // Keep the view on the map: its edges can't go past the map's edges,
    // and if the view is bigger than the map, center the map.
    let map_half = Vec2::new(MAP_W as f32, MAP_H as f32) * TILE_SIZE / 2.0;
    let view_half = window.size() * ortho.scale / 2.0;
    let limit = (map_half - view_half).max(Vec2::ZERO);
    transform.translation.x = transform.translation.x.clamp(-limit.x, limit.x);
    transform.translation.y = transform.translation.y.clamp(-limit.y, limit.y);
}
