//! Player input. Input never changes the simulation directly: it turns
//! clicks into `SimCommand`s on the command queue.
//!
//! Giving a signal: left-click any fighter to select its platoon, pick a
//! mission (buttons at the bottom, or keys 1 / 2 / 3), then right-click
//! the target. Right-clicking without picking a mission keeps the
//! platoon's current mission with a new target. Signals take 1 s to
//! reach the platoon, and every signal adds a little confusion, so don't
//! spam.
//!
//! Platoons with an engineer can also be told to build (buttons, or keys
//! 4-7: mine, bomb, turret, artillery), then right-click the site. The
//! build order travels like any other signal and is paid when sent.
//!
//! The King is selected and moved the same way, without missions. Your
//! artillery can be selected too: right-click an explored tile to shell it.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::faction::{Faction, LocalPlayer};
use crate::fog::FogDebug;
use crate::map::{TILE_SIZE, TilePos};
use crate::army_panel::ArmyBuilder;
use crate::fog::FogOfWar;
use crate::map::GameMap;
use crate::sim::economy::MatchPhase;
use crate::sim::SimTick;
use crate::sim::classes::FighterClass;
use crate::sim::fighter::{Condition, Fighter, WorkKind};
use crate::sim::gadgets::{
    AIR_STRIKE_RADIUS, ARTILLERY_RADIUS, ARTILLERY_RANGE, Gadget, GadgetKind, air_strike_impacts,
};
use crate::sim::platoon::{SIGNAL_TRAVEL_TICKS, SignalsInFlight};
use crate::sim::platoon::{Mission, Platoon};
use crate::sim::{CommandQueue, Mover, SimCommand, Unit, UnitClass};

const SELECT_RADIUS: f32 = 12.0;

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CursorWorld>()
            .init_resource::<Selection>()
            .init_resource::<PendingMission>()
            .init_resource::<PendingBuild>()
            .add_systems(Startup, spawn_orders_panel)
            .add_systems(
                Update,
                (
                    update_cursor,
                    (
                        mission_buttons,
                        build_buttons,
                        mission_hotkeys,
                        select_and_command,
                        draw_selection,
                        update_orders_panel,
                        draw_placement_preview,
                        draw_signals_in_flight,
                    )
                        .chain()
                        .run_if(resource_exists::<CommandQueue>)
                        .run_if(resource_exists::<MatchPhase>),
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
    /// True while the cursor is over a UI panel or button (world clicks
    /// ignored).
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
    Artillery(Entity),
}

/// Mission chosen for the next right-click signal.
#[derive(Resource, Default)]
pub struct PendingMission(pub Option<Mission>);

/// Gadget chosen for the next right-click build order.
#[derive(Resource, Default)]
pub struct PendingBuild(pub Option<GadgetKind>);

#[derive(Component)]
struct MissionButton(Mission);

#[derive(Component)]
struct BuildButton(GadgetKind);

#[derive(Component)]
struct MissionRow;

/// The row of build buttons (shown when the platoon has an engineer).
#[derive(Component)]
struct BuildRow;

#[derive(Component)]
struct OrdersText;

#[derive(Component)]
struct OrdersPanel;

fn update_cursor(
    mut cursor: ResMut<CursorWorld>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    buttons: Query<&Interaction>,
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
            margin: UiRect::left(Val::Px(-370.0)),
            width: Val::Px(740.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            padding: UiRect::all(Val::Px(8.0)),
            display: Display::None,
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        Interaction::default(),
        children![
            (OrdersText, Text::new(""), TextFont::from_font_size(14.0), TextColor(Color::WHITE)),
            (
                MissionRow,
                Node { margin: UiRect::top(Val::Px(6.0)), ..default() },
                children![
                    button(Mission::Assault, "1"),
                    button(Mission::Overwatch, "2"),
                    button(Mission::Reposition, "3"),
                ]
            ),
            (
                BuildRow,
                Node { margin: UiRect::top(Val::Px(6.0)), ..default() },
                children![
                    build_button(GadgetKind::Mine, "4"),
                    build_button(GadgetKind::Bomb, "5"),
                    build_button(GadgetKind::Turret, "6"),
                    build_button(GadgetKind::Artillery, "7"),
                ]
            )
        ],
    ));
}

fn build_button(kind: GadgetKind, key: &str) -> impl Bundle {
    let s = kind.stats();
    (
        Button,
        BuildButton(kind),
        Node { padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)), margin: UiRect::horizontal(Val::Px(4.0)), ..default() },
        BackgroundColor(Color::srgb(0.2, 0.2, 0.24)),
        children![(
            Text::new(format!("{key}  Build {} {:.0}", s.name, s.cost)),
            TextFont::from_font_size(13.0),
            TextColor(Color::WHITE)
        )],
    )
}

fn build_buttons(
    mut pending: ResMut<PendingBuild>,
    mut mission: ResMut<PendingMission>,
    mut buttons: Query<(&Interaction, &BuildButton, &mut BackgroundColor)>,
) {
    for (interaction, button, mut bg) in &mut buttons {
        if *interaction == Interaction::Pressed {
            pending.0 = Some(button.0);
            mission.0 = None;
        }
        let chosen = pending.0 == Some(button.0);
        bg.0 = match (chosen, interaction) {
            (true, _) => Color::srgb(0.75, 0.6, 0.15),
            (false, Interaction::Hovered) => Color::srgb(0.32, 0.32, 0.38),
            _ => Color::srgb(0.2, 0.2, 0.24),
        };
    }
}

fn mission_buttons(
    mut pending: ResMut<PendingMission>,
    mut build: ResMut<PendingBuild>,
    mut buttons: Query<(&Interaction, &MissionButton, &mut BackgroundColor)>,
) {
    for (interaction, button, mut bg) in &mut buttons {
        if *interaction == Interaction::Pressed {
            pending.0 = Some(button.0);
            build.0 = None;
        }
        let chosen = pending.0 == Some(button.0);
        bg.0 = match (chosen, interaction) {
            (true, _) => Color::srgb(0.75, 0.6, 0.15),
            (false, Interaction::Hovered) => Color::srgb(0.32, 0.32, 0.38),
            _ => Color::srgb(0.2, 0.2, 0.24),
        };
    }
}

fn mission_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    selection: Res<Selection>,
    mut pending: ResMut<PendingMission>,
    mut build: ResMut<PendingBuild>,
) {
    if !matches!(*selection, Selection::Platoon(_)) {
        return;
    }
    for (key, mission) in [(KeyCode::Digit1, Mission::Assault), (KeyCode::Digit2, Mission::Overwatch), (KeyCode::Digit3, Mission::Reposition)] {
        if keys.just_pressed(key) {
            pending.0 = Some(mission);
            build.0 = None;
        }
    }
    for (key, kind) in [
        (KeyCode::Digit4, GadgetKind::Mine),
        (KeyCode::Digit5, GadgetKind::Bomb),
        (KeyCode::Digit6, GadgetKind::Turret),
        (KeyCode::Digit7, GadgetKind::Artillery),
    ] {
        if keys.just_pressed(key) {
            build.0 = Some(kind);
            pending.0 = None;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn select_and_command(
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<CursorWorld>,
    local: Res<LocalPlayer>,
    phase: Res<MatchPhase>,
    mut builder: ResMut<ArmyBuilder>,
    mut selection: ResMut<Selection>,
    mut pending: ResMut<PendingMission>,
    mut build: ResMut<PendingBuild>,
    mut queue: ResMut<CommandQueue>,
    units: Query<(Entity, &Unit, &Faction, &Transform, Option<&Fighter>)>,
    platoons: Query<&Platoon>,
) {
    if cursor.over_ui {
        return;
    }
    let Some(world) = cursor.world else { return };

    // Placing something from the army panel: a platoon, a gadget (setup)
    // or an air strike (battle).
    if builder.placing() {
        if buttons.just_pressed(MouseButton::Right) {
            builder.cancel();
        } else if buttons.just_pressed(MouseButton::Left)
            && let Some(tile) = TilePos::from_world(world)
        {
            if let Some(kind) = builder.gadget {
                // Stay in gadget mode, so several can be placed in a row.
                queue.push(SimCommand::PlaceGadget { issuer: local.0, kind, at: tile });
            } else if builder.air_strike {
                builder.air_strike = false;
                queue.push(SimCommand::AirStrike { issuer: local.0, target: tile });
            } else {
                let classes = std::mem::take(&mut builder.classes);
                queue.push(if phase.battle {
                    SimCommand::Deploy { issuer: local.0, classes, vehicle: builder.vehicle, target: tile }
                } else {
                    SimCommand::PlacePlatoon { issuer: local.0, classes, anchor: tile }
                });
            }
        }
        return;
    }
    if !phase.battle {
        return;
    }

    if buttons.just_pressed(MouseButton::Left) {
        let clicked = units
            .iter()
            .filter(|(_, u, faction, _, _)| {
                **faction == local.0
                    && !matches!(u.class, UnitClass::Gadget(k) if k != GadgetKind::Artillery)
            })
            .map(|(e, u, _, t, f)| (e, u, f, t.translation.truncate().distance(world)))
            .filter(|&(_, _, _, d)| d <= SELECT_RADIUS)
            .min_by(|a, b| a.3.total_cmp(&b.3));
        let new = match clicked {
            Some((e, unit, _, _)) if unit.class == UnitClass::King => Selection::King(e),
            Some((e, unit, _, _)) if unit.class == UnitClass::Gadget(GadgetKind::Artillery) => Selection::Artillery(e),
            Some((_, _, Some(fighter), _)) => Selection::Platoon(fighter.platoon),
            _ => Selection::None,
        };
        if new != *selection {
            pending.0 = None;
            build.0 = None;
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
                    if let Some(kind) = build.0.take() {
                        queue.push(SimCommand::Build { issuer: local.0, platoon: platoon.id, kind, target });
                    } else {
                        let mission = pending.0.unwrap_or(platoon.pending.map_or(platoon.mission, |p| p.mission));
                        queue.push(SimCommand::Signal { issuer: local.0, platoon: platoon.id, mission, target });
                    }
                    pending.0 = None;
                }
            }
            Selection::Artillery(e) => {
                if let Ok((_, unit, _, _, _)) = units.get(e) {
                    queue.push(SimCommand::ArtilleryFire { issuer: local.0, unit: unit.id, target });
                }
            }
            Selection::None => {}
        }
    }
}

fn draw_selection(
    mut selection: ResMut<Selection>,
    units: Query<(&Transform, &Mover)>,
    guns: Query<&Gadget>,
    platoons: Query<&Platoon>,
    mut gizmos: Gizmos,
) {
    let yellow = Color::srgb(1.0, 0.95, 0.3);
    match *selection {
        Selection::None => {}
        Selection::Artillery(e) => {
            let (Ok((transform, _)), Ok(gun)) = (units.get(e), guns.get(e)) else {
                *selection = Selection::None; // destroyed
                return;
            };
            let here = transform.translation.truncate();
            gizmos.circle_2d(Isometry2d::from_translation(here), 12.0, yellow);
            gizmos.circle_2d(Isometry2d::from_translation(here), ARTILLERY_RANGE * TILE_SIZE, yellow.with_alpha(0.25));
            if let Some(aim) = gun.aim {
                gizmos.circle_2d(Isometry2d::from_translation(aim.to_world()), ARTILLERY_RADIUS * TILE_SIZE, yellow.with_alpha(0.7));
            }
        }
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

#[allow(clippy::too_many_arguments)]
fn update_orders_panel(
    phase: Res<MatchPhase>,
    flight: Res<SignalsInFlight>,
    selection: Res<Selection>,
    pending: Res<PendingMission>,
    build: Res<PendingBuild>,
    platoons: Query<&Platoon>,
    fighters: Query<(&Fighter, &Condition)>,
    guns: Query<&Gadget>,
    mut panel: Single<&mut Node, (With<OrdersPanel>, Without<BuildRow>, Without<MissionRow>)>,
    mut build_row: Single<&mut Node, (With<BuildRow>, Without<OrdersPanel>, Without<MissionRow>)>,
    mut mission_row: Single<&mut Node, (With<MissionRow>, Without<OrdersPanel>, Without<BuildRow>)>,
    mut text: Single<&mut Text, With<OrdersText>>,
) {
    if !phase.battle {
        panel.display = Display::None;
        return;
    }
    if let Selection::Artillery(e) = *selection {
        let Ok(gun) = guns.get(e) else { return };
        panel.display = Display::Flex;
        build_row.display = Display::None;
        mission_row.display = Display::None;
        let state = if gun.cooldown > 0.0 { format!("reloading ({:.0} s)", gun.cooldown) } else { "ready".into() };
        text.0 = format!(
            "Artillery  |  {state}\nRight-click an explored tile within range to shell it next.\n\
             Left alone, it shells the biggest enemy group your side can see."
        );
        return;
    }
    let Selection::Platoon(e) = *selection else {
        panel.display = Display::None;
        return;
    };
    let Ok(platoon) = platoons.get(e) else { return };
    panel.display = Display::Flex;
    mission_row.display = Display::Flex;
    let engineers: Vec<&Condition> = platoon
        .members
        .iter()
        .filter_map(|m| fighters.get(*m).ok())
        .filter(|(f, _)| f.class == FighterClass::Engineer)
        .map(|(_, c)| c)
        .collect();
    build_row.display = if engineers.is_empty() { Display::None } else { Display::Flex };
    let mut line = format!(
        "{}  |  {} fighters  |  Mission: {}  |  Confusion: {:.0} ({})",
        platoon.name,
        platoon.members.len(),
        platoon.mission.name(),
        platoon.confusion,
        platoon.confusion_label()
    );
    if let Some(s) = flight.heading_to(e) {
        match s.build {
            Some((kind, _)) => line.push_str(&format!("\nBuild order on its way: {}", kind.stats().name)),
            None => line.push_str(&format!("\nSignal on its way: {}", s.mission.name())),
        }
    } else if let Some(p) = platoon.pending {
        line.push_str(&format!("\nOrder received, not acted on yet: {}", p.mission.name()));
    }
    for c in &engineers {
        if let Some(work) = c.work {
            let what = match work.kind {
                WorkKind::Build(kind) => format!("building a {}", kind.stats().name),
                WorkKind::Defuse(_) => "defusing a trap".to_string(),
                WorkKind::Assist(_) => "helping build".to_string(),
            };
            line.push_str(&format!("\nEngineer {what} ({:.0}%)", work.progress * 100.0));
        }
    }
    line.push_str(&match (pending.0, build.0) {
        (_, Some(k)) => format!("\nRight-click the site to build: {} ({:.0})", k.stats().name, k.stats().cost),
        (Some(m), _) => format!("\nRight-click a target to signal: {}", m.name()),
        _ => "\nPick a mission, then right-click a target".to_string(),
    });
    text.0 = line;
}

/// Your signals travelling from the King to their platoons: a pulse moving
/// along a faint line (the enemy's signals are invisible).
fn draw_signals_in_flight(
    flight: Res<SignalsInFlight>,
    tick: Res<SimTick>,
    fixed: Res<Time<Fixed>>,
    local: Res<LocalPlayer>,
    platoons: Query<&Platoon>,
    transforms: Query<&Transform>,
    mut gizmos: Gizmos,
) {
    let travel = SIGNAL_TRAVEL_TICKS.max(1) as f32;
    for s in flight.0.iter().filter(|s| s.issuer == local.0) {
        let from = s.from.to_world();
        let to = platoons
            .get(s.platoon)
            .ok()
            .and_then(|p| p.leader.or(p.members.first().copied()))
            .and_then(|m| transforms.get(m).ok())
            .map_or(s.target.to_world(), |t| t.translation.truncate());
        let t = ((tick.0.saturating_sub(s.sent_tick)) as f32 + fixed.overstep_fraction()) / travel;
        let pulse = from.lerp(to, t.clamp(0.0, 1.0));
        let color = Color::srgb(0.6, 0.95, 1.0);
        gizmos.line_2d(from, to, color.with_alpha(0.25));
        gizmos.circle_2d(Isometry2d::from_translation(pulse), 4.0, color);
        gizmos.circle_2d(Isometry2d::from_translation(pulse), 2.0, color);
    }
}

/// While a platoon is being built: show where it would go, green if the
/// click would be accepted (your half in setup, explored ground in battle).
#[allow(clippy::too_many_arguments)]
fn draw_placement_preview(
    builder: Res<ArmyBuilder>,
    cursor: Res<CursorWorld>,
    phase: Res<MatchPhase>,
    local: Res<LocalPlayer>,
    map: Res<GameMap>,
    fog: Res<FogOfWar>,
    build: Res<PendingBuild>,
    kings: Query<(&Unit, &Faction, &TilePos)>,
    mut gizmos: Gizmos,
) {
    if cursor.over_ui {
        return;
    }
    let Some(tile) = cursor.tile() else { return };
    let good = Color::srgb(0.3, 1.0, 0.4);
    let bad = Color::srgb(1.0, 0.3, 0.3);
    // Engineer build site, or a setup gadget.
    let gadget = build.0.filter(|_| phase.battle).or(builder.gadget.filter(|_| !phase.battle));
    if let Some(kind) = gadget {
        let ok = map.tile(tile).passable()
            && if phase.battle { fog.is_explored(local.0, tile) } else { map.in_home_half(local.0, tile) };
        let size = if kind.is_trap() { 8.0 } else { 14.0 };
        gizmos.rect_2d(Isometry2d::from_translation(tile.to_world()), Vec2::splat(size), if ok { good } else { bad });
        return;
    }
    if builder.air_strike {
        let ok = fog.is_explored(local.0, tile);
        let from = kings
            .iter()
            .find(|(u, f, _)| u.class == UnitClass::King && **f == local.0)
            .map_or(tile, |(_, _, p)| *p);
        for (p, _) in air_strike_impacts(from, tile, 0) {
            gizmos.circle_2d(Isometry2d::from_translation(p.to_world()), AIR_STRIKE_RADIUS * TILE_SIZE, if ok { good } else { bad });
        }
        return;
    }
    if builder.is_empty() {
        return;
    }
    let ok = if phase.battle {
        fog.is_explored(local.0, tile) && map.tile(tile).passable()
    } else {
        map.in_home_half(local.0, tile)
    };
    let color = if ok { Color::srgb(0.3, 1.0, 0.4) } else { Color::srgb(1.0, 0.3, 0.3) };
    for slot in 0..builder.classes.len() {
        let p = crate::sim::platoon::slot_tile(tile, slot, local.0);
        gizmos.circle_2d(Isometry2d::from_translation(p.to_world()), 5.0, color);
    }
    if phase.battle && builder.vehicle {
        gizmos.rect_2d(Isometry2d::from_translation(tile.to_world()), Vec2::new(18.0, 11.0), color);
    }
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
