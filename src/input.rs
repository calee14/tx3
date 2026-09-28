//! Player input. Input never changes the simulation directly: it turns
//! clicks into `SimCommand`s on the command queue.
//!
//! Giving a signal: left-click any fighter to select its platoon, pick a
//! mission (buttons at the bottom, or keys 1 / 2 / 3), then right-click
//! the target. Right-clicking without picking a mission keeps the
//! platoon's current mission with a new target. Every signal adds a
//! little confusion to the platoon, so don't spam.
//!
//! The King is selected and moved the same way, without missions.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::faction::{Faction, LocalPlayer};
use crate::fog::FogDebug;
use crate::map::{TILE_SIZE, TilePos};
use crate::sim::fighter::Fighter;
use crate::sim::platoon::{Mission, Platoon};
use crate::sim::{CommandQueue, Mover, SimCommand, Unit, UnitClass};

const SELECT_RADIUS: f32 = 12.0;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorWorld>()
            .init_resource::<Selection>()
            .init_resource::<PendingMission>()
            .add_systems(Startup, spawn_orders_panel)
            .add_systems(
                Update,
                (
                    update_cursor,
                    (mission_buttons, mission_hotkeys, select_and_command, draw_selection, update_orders_panel)
                        .chain()
                        .run_if(resource_exists::<CommandQueue>),
                    draw_hover,
                    toggle_pause,
                    toggle_fog_debug,
                )
                    .chain(),
            );
    }
}

/// World position under the mouse cursor, if it's over the map view.
#[derive(Resource, Default)]
pub struct CursorWorld {
    pub world: Option<Vec2>,
    /// True while the cursor is over a UI button (world clicks ignored).
    pub over_ui: bool,
}

impl CursorWorld {
    pub fn tile(&self) -> Option<TilePos> {
        self.world.and_then(TilePos::from_world)
    }
}

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Selection {
    #[default]
    None,
    King(Entity),
    Platoon(Entity),
}

/// Mission chosen for the next right-click signal.
#[derive(Resource, Default)]
pub struct PendingMission(pub Option<Mission>);

#[derive(Component)]
struct MissionButton(Mission);

#[derive(Component)]
struct OrdersText;

#[derive(Component)]
struct OrdersPanel;

fn update_cursor(
    mut cursor: ResMut<CursorWorld>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    buttons: Query<&Interaction, With<MissionButton>>,
) {
    let (camera, camera_transform) = *camera;
    cursor.world = window
        .cursor_position()
        .and_then(|screen| camera.viewport_to_world_2d(camera_transform, screen).ok());
    cursor.over_ui = buttons.iter().any(|i| *i != Interaction::None);
}

fn spawn_orders_panel(mut commands: Commands) {
    let button = |mission: Mission, key: &str| {
        (
            Button,
            MissionButton(mission),
            Node { padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)), margin: UiRect::horizontal(Val::Px(4.0)), ..default() },
            BackgroundColor(Color::srgb(0.2, 0.2, 0.24)),
            children![(Text::new(format!("{key}  {}", mission.name())), TextFont::from_font_size(14.0), TextColor(Color::WHITE))],
        )
    };
    commands.spawn((
        OrdersPanel,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(10.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-330.0)),
            width: Val::Px(660.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        Visibility::Hidden,
        children![
            (OrdersText, Text::new(""), TextFont::from_font_size(14.0), TextColor(Color::WHITE)),
            (
                Node { margin: UiRect::top(Val::Px(6.0)), ..default() },
                children![
                    button(Mission::Assault, "1"),
                    button(Mission::Overwatch, "2"),
                    button(Mission::Reposition, "3"),
                ]
            )
        ],
    ));
}

fn mission_buttons(
    mut pending: ResMut<PendingMission>,
    mut buttons: Query<(&Interaction, &MissionButton, &mut BackgroundColor)>,
) {
    for (interaction, button, mut bg) in &mut buttons {
        if *interaction == Interaction::Pressed {
            pending.0 = Some(button.0);
        }
        let chosen = pending.0 == Some(button.0);
        bg.0 = match (chosen, interaction) {
            (true, _) => Color::srgb(0.75, 0.6, 0.15),
            (false, Interaction::Hovered) => Color::srgb(0.32, 0.32, 0.38),
            _ => Color::srgb(0.2, 0.2, 0.24),
        };
    }
}

fn mission_hotkeys(keys: Res<ButtonInput<KeyCode>>, selection: Res<Selection>, mut pending: ResMut<PendingMission>) {
    if !matches!(*selection, Selection::Platoon(_)) {
        return;
    }
    for (key, mission) in [(KeyCode::Digit1, Mission::Assault), (KeyCode::Digit2, Mission::Overwatch), (KeyCode::Digit3, Mission::Reposition)] {
        if keys.just_pressed(key) {
            pending.0 = Some(mission);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn select_and_command(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<CursorWorld>,
    local: Res<LocalPlayer>,
    mut selection: ResMut<Selection>,
    mut pending: ResMut<PendingMission>,
    mut queue: ResMut<CommandQueue>,
    units: Query<(Entity, &Unit, &Faction, &Transform, Option<&Fighter>)>,
    platoons: Query<&Platoon>,
) {
    if cursor.over_ui {
        return;
    }
    let Some(world) = cursor.world else { return };

    if buttons.just_pressed(MouseButton::Left) {
        let clicked = units
            .iter()
            .filter(|(_, _, faction, _, _)| **faction == local.0)
            .map(|(e, u, _, t, f)| (e, u, f, t.translation.truncate().distance(world)))
            .filter(|&(_, _, _, d)| d <= SELECT_RADIUS)
            .min_by(|a, b| a.3.total_cmp(&b.3));
        let new = match clicked {
            Some((e, unit, _, _)) if unit.class == UnitClass::King => Selection::King(e),
            Some((_, _, Some(fighter), _)) => Selection::Platoon(fighter.platoon),
            _ => Selection::None,
        };
        if new != *selection {
            pending.0 = None;
        }
        *selection = new;
    }

    if buttons.just_pressed(MouseButton::Right) {
        let Some(target) = TilePos::from_world(world) else { return };
        match *selection {
            Selection::King(e) => {
                if let Ok((_, unit, _, _, _)) = units.get(e) {
                    queue.push(SimCommand::Move { issuer: local.0, unit: unit.id, target });
                }
            }
            Selection::Platoon(e) => {
                if let Ok(platoon) = platoons.get(e) {
                    let mission = pending.0.unwrap_or(platoon.pending.map_or(platoon.mission, |p| p.mission));
                    queue.push(SimCommand::Signal { issuer: local.0, platoon: platoon.id, mission, target });
                    pending.0 = None;
                }
            }
            Selection::None => {}
        }
    }
}

fn draw_selection(
    mut selection: ResMut<Selection>,
    units: Query<(&Transform, &Mover)>,
    platoons: Query<&Platoon>,
    mut gizmos: Gizmos,
) {
    let yellow = Color::srgb(1.0, 0.95, 0.3);
    match *selection {
        Selection::None => {}
        Selection::King(e) => {
            let Ok((transform, mover)) = units.get(e) else {
                *selection = Selection::None;
                return;
            };
            let here = transform.translation.truncate();
            gizmos.circle_2d(Isometry2d::from_translation(here), 11.0, yellow);
            let points: Vec<Vec2> = std::iter::once(here).chain(mover.path.iter().map(|p| p.to_world())).collect();
            if points.len() > 1 {
                gizmos.linestrip_2d(points, yellow.with_alpha(0.6));
            }
        }
        Selection::Platoon(e) => {
            let Ok(platoon) = platoons.get(e) else {
                *selection = Selection::None; // platoon wiped out
                return;
            };
            for &m in &platoon.members {
                if let Ok((transform, _)) = units.get(m) {
                    gizmos.circle_2d(Isometry2d::from_translation(transform.translation.truncate()), 10.0, yellow);
                }
            }
            // Where it's forming up now, and the mission target.
            let anchor = platoon.anchor.to_world();
            let target = platoon.target.to_world();
            gizmos.rect_2d(Isometry2d::from_translation(anchor), Vec2::splat(TILE_SIZE * 0.6), yellow.with_alpha(0.7));
            if platoon.anchor != platoon.target {
                gizmos.line_2d(anchor, target, yellow.with_alpha(0.4));
            }
            let mission_color = match platoon.mission {
                Mission::Assault => Color::srgb(1.0, 0.4, 0.3),
                Mission::Overwatch => Color::srgb(0.4, 0.8, 1.0),
                Mission::Reposition => Color::srgb(0.5, 1.0, 0.5),
            };
            gizmos.circle_2d(Isometry2d::from_translation(target), TILE_SIZE * 0.8, mission_color);
            if let Some(p) = platoon.pending {
                gizmos.circle_2d(Isometry2d::from_translation(p.target.to_world()), TILE_SIZE * 0.5, yellow.with_alpha(0.5));
            }
        }
    }
}

fn update_orders_panel(
    selection: Res<Selection>,
    pending: Res<PendingMission>,
    platoons: Query<&Platoon>,
    mut panel: Single<&mut Visibility, With<OrdersPanel>>,
    mut text: Single<&mut Text, With<OrdersText>>,
) {
    let Selection::Platoon(e) = *selection else {
        **panel = Visibility::Hidden;
        return;
    };
    let Ok(platoon) = platoons.get(e) else { return };
    **panel = Visibility::Inherited;
    let mut line = format!(
        "{}  |  {} fighters  |  Mission: {}  |  Confusion: {:.0} ({})",
        platoon.name,
        platoon.members.len(),
        platoon.mission.name(),
        platoon.confusion,
        platoon.confusion_label()
    );
    if let Some(p) = platoon.pending {
        line.push_str(&format!("\nOrder received, not acted on yet: {}", p.mission.name()));
    }
    line.push_str(&match pending.0 {
        Some(m) => format!("\nRight-click a target to signal: {}", m.name()),
        None => "\nPick a mission, then right-click a target".to_string(),
    });
    text.0 = line;
}

fn draw_hover(cursor: Res<CursorWorld>, mut gizmos: Gizmos) {
    if cursor.over_ui {
        return;
    }
    if let Some(tile) = cursor.tile() {
        gizmos.rect_2d(
            Isometry2d::from_translation(tile.to_world()),
            Vec2::splat(TILE_SIZE),
            Color::srgba(1.0, 1.0, 1.0, 0.5),
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
