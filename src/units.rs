//! Unit presentation (placeholder art). The sim spawns units with game
//! data only; this attaches sprites and draws overhead info.
//!
//! - Colored square with a class letter (K, A, S, H, J).
//! - Status icons above fighters: RUN broken, ?? frozen, !! pinned,
//!   ? confused, ! suppressed, ++/+ injured, ~ shaken, ^ brave.
//! - Health bar, and a gold marker over platoon leaders.
//! - Shot tracers.

use bevy::prelude::*;

use crate::faction::{Faction, LocalPlayer};
use crate::fog::{FogDebug, FogOfWar};
use crate::map::{GameMap, TilePos};
use crate::sim::classes::FighterClass;
use crate::sim::combat::ShotLog;
use crate::sim::fighter::{Condition, Fighter, Health, statuses};
use crate::sim::platoon::Platoon;
use crate::sim::vehicle::ParkedVehicle;
use crate::sim::{Mover, Unit, UnitClass};

pub const Z_UNITS: f32 = 10.0;
pub const UNIT_SPRITE_SIZE: f32 = 12.0;
const TRACER_SECONDS: f32 = 0.15;

pub struct UnitVisualsPlugin;

impl Plugin for UnitVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tracers>().add_systems(
            Update,
            (
                attach_unit_visuals,
                attach_parked_vehicle_visuals,
                update_status_icons,
                (draw_unit_overlays, collect_and_draw_tracers).run_if(resource_exists::<FogOfWar>),
            ),
        );
    }
}

#[derive(Component)]
struct StatusIcons;

/// Parked vehicles sit just under the fog layer, like terrain.
const Z_PARKED: f32 = 4.0;

fn attach_parked_vehicle_visuals(
    mut commands: Commands,
    parked: Query<(Entity, &Faction, &TilePos, &ParkedVehicle), Added<ParkedVehicle>>,
) {
    for (entity, faction, pos, parked) in &parked {
        let color = if parked.wrecked {
            Color::srgb(0.18, 0.17, 0.16)
        } else {
            faction.color().darker(0.15)
        };
        commands.entity(entity).insert((
            Sprite::from_color(color, Vec2::new(16.0, 11.0)),
            Transform::from_translation(pos.to_world().extend(Z_PARKED)),
            Visibility::default(),
            children![(
                Sprite::from_color(Color::srgb(0.1, 0.1, 0.1), Vec2::new(16.0, 2.0)),
                Transform::from_xyz(0.0, -3.0, 0.1),
            )],
        ));
    }
}

fn attach_unit_visuals(mut commands: Commands, units: Query<(Entity, &Unit, &Faction), Added<Unit>>) {
    for (entity, unit, faction) in &units {
        let (size, color) = match unit.class {
            UnitClass::King => (Vec2::splat(UNIT_SPRITE_SIZE + 3.0), faction.color()),
            UnitClass::Vehicle => (Vec2::new(18.0, 11.0), faction.color().darker(0.2)),
            UnitClass::Fighter(_) => (Vec2::splat(UNIT_SPRITE_SIZE), faction.color()),
        };
        commands.entity(entity).insert((Sprite::from_color(color, size), Visibility::default()));
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Text2d::new(unit.class.letter()),
                TextFont::from_font_size(10.0),
                TextColor(Color::BLACK),
                Transform::from_xyz(0.0, 0.0, 1.0),
            ));
            parent.spawn((
                StatusIcons,
                Text2d::new(""),
                TextFont::from_font_size(10.0),
                TextColor(Color::WHITE),
                Transform::from_xyz(0.0, 17.0, 2.0),
            ));
        });
        // Put units above the map and fog layers.
        commands.entity(entity).insert(Transform::from_xyz(0.0, 0.0, Z_UNITS));
    }
}

/// Is this position visible to the local commander?
fn shown(fog: &FogOfWar, debug: &FogDebug, local: LocalPlayer, faction: Faction, pos: TilePos) -> bool {
    faction == local.0 || debug.show_all || fog.is_explored(local.0, pos)
}

fn update_status_icons(
    map: Option<Res<GameMap>>,
    local: Res<LocalPlayer>,
    platoons: Query<&Platoon>,
    fighters: Query<(&Faction, &TilePos, &Fighter, &Condition, &Health, &Mover, &Children)>,
    mut icons: Query<(&mut Text2d, &mut TextColor), With<StatusIcons>>,
) {
    let Some(map) = map else { return };
    for (faction, pos, fighter, cond, health, mover, children) in &fighters {
        let confusion = platoons.get(fighter.platoon).map_or(0.0, |p| p.confusion);
        let all = statuses(cond, health, confusion, !mover.path.is_empty(), map.cover_at(*pos));
        // Enemies only show what can be seen from outside.
        let visible: Vec<_> =
            all.into_iter().filter(|s| *faction == local.0 || s.visible_to_enemy()).filter_map(|s| s.icon()).collect();
        let text: String = visible.iter().take(3).map(|(t, _)| *t).collect::<Vec<_>>().join(" ");
        let color = visible.first().map_or(Color::WHITE, |(_, c)| *c);
        for child in children.iter() {
            if let Ok((mut t, mut c)) = icons.get_mut(child) {
                if t.0 != text {
                    t.0 = text.clone();
                }
                c.0 = color;
            }
        }
    }
}

fn draw_unit_overlays(
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    platoons: Query<&Platoon>,
    units: Query<(Entity, &Faction, &TilePos, &Transform, &Health, Option<&Fighter>)>,
    mut gizmos: Gizmos,
) {
    for (entity, faction, pos, transform, health, fighter) in &units {
        if !shown(&fog, &debug, *local, *faction, *pos) {
            continue;
        }
        let p = transform.translation.truncate();
        // Health bar.
        let left = p + Vec2::new(-7.0, 10.0);
        gizmos.line_2d(left, left + Vec2::new(14.0, 0.0), Color::srgba(0.0, 0.0, 0.0, 0.8));
        let color = if health.frac() > 0.6 {
            Color::srgb(0.3, 0.9, 0.3)
        } else if health.frac() > 0.3 {
            Color::srgb(0.95, 0.8, 0.2)
        } else {
            Color::srgb(0.95, 0.25, 0.2)
        };
        gizmos.line_2d(left, left + Vec2::new(14.0 * health.frac(), 0.0), color);

        // Leader marker: a small gold diamond on the left of the bar.
        if let Some(f) = fighter
            && platoons.get(f.platoon).is_ok_and(|pl| pl.leader == Some(entity))
        {
            let c = p + Vec2::new(-10.0, 10.0);
            let gold = Color::srgb(1.0, 0.85, 0.2);
            gizmos.linestrip_2d(
                [c + Vec2::Y * 3.0, c + Vec2::X * 3.0, c - Vec2::Y * 3.0, c - Vec2::X * 3.0, c + Vec2::Y * 3.0],
                gold,
            );
        }
    }
}

#[derive(Resource, Default)]
struct Tracers(Vec<(Vec2, Vec2, bool, bool, f32)>);

fn collect_and_draw_tracers(
    time: Res<Time<Real>>,
    virt: Res<Time<Virtual>>,
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    mut log: ResMut<ShotLog>,
    mut tracers: ResMut<Tracers>,
    mut gizmos: Gizmos,
) {
    for shot in log.0.drain(..) {
        let visible = debug.show_all || fog.is_explored(local.0, shot.from) || fog.is_explored(local.0, shot.to);
        if !visible {
            continue;
        }
        let from = shot.from.to_world();
        let mut to = shot.to.to_world();
        if !shot.hit {
            // Misses fly past (fixed offset; purely visual).
            let off = ((shot.from.x * 31 + shot.to.y * 17) % 7 - 3) as f32 * 3.0;
            to += Vec2::new(off, -off * 0.7);
        }
        let heavy = shot.shooter_class == FighterClass::Sniper;
        tracers.0.push((from, to, shot.hit, heavy, if heavy { TRACER_SECONDS * 3.0 } else { TRACER_SECONDS }));
    }
    if !virt.is_paused() {
        let dt = time.delta_secs();
        for t in &mut tracers.0 {
            t.4 -= dt;
        }
        tracers.0.retain(|t| t.4 > 0.0);
    }
    for &(from, to, hit, sniper, life) in &tracers.0 {
        let a = (life / if sniper { TRACER_SECONDS * 3.0 } else { TRACER_SECONDS }).clamp(0.0, 1.0);
        // Sniper shots: long-lived cyan streaks, so you can spot the shooter.
        let color = match (sniper, hit) {
            (true, _) => Color::srgba(0.5, 1.0, 1.0, a),
            (false, true) => Color::srgba(1.0, 0.9, 0.5, a),
            (false, false) => Color::srgba(1.0, 1.0, 1.0, a * 0.5),
        };
        gizmos.line_2d(from, to, color);
    }
}
