//! Unit presentation (placeholder art). The sim spawns units with game
//! data only; this attaches sprites and draws overhead info.
//!
//! - Colored square with a class letter (K, A, S, H, J).
//! - Status icons above fighters: RUN broken, ?? frozen, !! pinned,
//!   ? confused, ! suppressed, ++/+ injured, ~ shaken, ^ brave.
//! - Health bar, and a gold marker over platoon leaders.
//! - Shot tracers.
//! - Gadgets: small dark squares (mines m, bombs b), turrets (T) and
//!   artillery (Q). Enemy gadgets only appear once your side has found them.
//! - Strike warnings (red rings that close in as the impact nears),
//!   explosions, and smoke.

use bevy::prelude::*;

use crate::faction::{Faction, LocalPlayer};
use crate::fog::{FogDebug, FogOfWar};
use crate::map::TilePos;
use crate::sim::classes::FighterClass;
use crate::sim::combat::ShotLog;
use crate::sim::fighter::{Condition, Fighter, Health, Veterancy, statuses};
use crate::sim::gadgets::{BlastLog, Concealed, GadgetKind, Strike, StrikeKind};
use crate::sim::SimTick;
use crate::map::{GameMap, TILE_SIZE};
use crate::sim::platoon::Platoon;
use crate::sim::vehicle::ParkedVehicle;
use crate::sim::{Mover, Unit, UnitClass};

pub const Z_UNITS: f32 = 10.0;
pub const UNIT_SPRITE_SIZE: f32 = 12.0;
const TRACER_SECONDS: f32 = 0.15;

pub struct UnitVisualsPlugin;

impl Plugin for UnitVisualsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tracers>().init_resource::<Blasts>().add_systems(
            Update,
            (
                attach_unit_visuals,
                attach_parked_vehicle_visuals,
                update_status_icons,
                (draw_unit_overlays, collect_and_draw_tracers, draw_strikes, draw_blasts, draw_smoke)
                    .run_if(resource_exists::<FogOfWar>)
                    .run_if(resource_exists::<crate::map::GameMap>),
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
            UnitClass::Gadget(kind) => match kind {
                GadgetKind::Mine => (Vec2::splat(7.0), faction.color().darker(0.35)),
                GadgetKind::Bomb => (Vec2::splat(9.0), faction.color().darker(0.3)),
                GadgetKind::Turret => (Vec2::splat(14.0), faction.color().darker(0.1)),
                GadgetKind::Artillery => (Vec2::new(17.0, 12.0), faction.color().darker(0.1)),
            },
        };
        commands.entity(entity).insert((Sprite::from_color(color, size), Visibility::default()));
        // Dark trap sprites get a light letter.
        let letter_color = match unit.class {
            UnitClass::Gadget(k) if k.is_trap() => Color::srgb(0.9, 0.9, 0.9),
            _ => Color::BLACK,
        };
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                Text2d::new(unit.class.letter()),
                TextFont::from_font_size(10.0),
                TextColor(letter_color),
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

/// Is this unit visible to the local commander? Enemy units show on
/// explored ground; enemy gadgets only once found.
pub fn shown(
    fog: &FogOfWar,
    debug: &FogDebug,
    local: LocalPlayer,
    faction: Faction,
    pos: TilePos,
    concealed: Option<&Concealed>,
) -> bool {
    faction == local.0
        || debug.show_all
        || (fog.is_explored(local.0, pos) && concealed.is_none_or(|c| c.known_to(local.0)))
}

fn update_status_icons(
    local: Res<LocalPlayer>,
    platoons: Query<&Platoon>,
    fighters: Query<(&Faction, &Fighter, &Condition, &Health, &Mover, &Veterancy, &Children)>,
    mut icons: Query<(&mut Text2d, &mut TextColor), With<StatusIcons>>,
) {
    for (faction, fighter, cond, health, mover, vet, children) in &fighters {
        let confusion = platoons.get(fighter.platoon).map_or(0.0, |p| p.confusion);
        let all = statuses(cond, health, confusion, !mover.path.is_empty(), cond.cover, vet.rank);
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
    units: Query<(Entity, &Faction, &TilePos, &Transform, &Health, Option<&Fighter>, Option<&Concealed>)>,
    mut gizmos: Gizmos,
) {
    for (entity, faction, pos, transform, health, fighter, concealed) in &units {
        if !shown(&fog, &debug, *local, *faction, *pos, concealed) {
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
        let heavy = matches!(shot.shooter, UnitClass::Fighter(FighterClass::Sniper) | UnitClass::Gadget(_));
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

/// Incoming strikes: a red ring at each impact point, with an inner ring
/// that closes in as the impact nears. Shown for your own strikes, and for
/// enemy strikes landing on ground you've explored.
fn draw_strikes(
    tick: Res<SimTick>,
    fixed: Res<Time<Fixed>>,
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    strikes: Query<&Strike>,
    mut gizmos: Gizmos,
) {
    let now = tick.0 as f32 + fixed.overstep_fraction();
    for strike in &strikes {
        let color = match strike.kind {
            StrikeKind::Air => Color::srgb(1.0, 0.3, 0.2),
            StrikeKind::Artillery => Color::srgb(1.0, 0.45, 0.15),
            _ => Color::srgb(1.0, 0.75, 0.2),
        };
        let mut points = Vec::new();
        for &(at, due) in &strike.impacts {
            let visible = strike.faction == local.0 || debug.show_all || fog.is_explored(local.0, at);
            if !visible {
                continue;
            }
            let p = at.to_world();
            points.push(p);
            let r = strike.radius * TILE_SIZE;
            let total = (due.saturating_sub(strike.launched_tick)).max(1) as f32;
            let left = ((due as f32 - now) / total).clamp(0.0, 1.0);
            gizmos.circle_2d(Isometry2d::from_translation(p), r, color);
            gizmos.circle_2d(Isometry2d::from_translation(p), r * left, color.with_alpha(0.6));
            gizmos.line_2d(p - Vec2::new(4.0, 4.0), p + Vec2::new(4.0, 4.0), color);
            gizmos.line_2d(p - Vec2::new(4.0, -4.0), p + Vec2::new(4.0, -4.0), color);
        }
        if strike.kind == StrikeKind::Air && points.len() > 1 {
            gizmos.linestrip_2d(points, color.with_alpha(0.35));
        }
    }
}

/// Explosions, as expanding orange rings (purely visual).
#[derive(Resource, Default)]
struct Blasts(Vec<(Vec2, f32, f32)>);

const BLAST_SECONDS: f32 = 0.5;

fn draw_blasts(
    time: Res<Time<Real>>,
    virt: Res<Time<Virtual>>,
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    mut log: ResMut<BlastLog>,
    mut blasts: ResMut<Blasts>,
    mut gizmos: Gizmos,
) {
    for (at, radius) in log.0.drain(..) {
        if debug.show_all || fog.is_explored(local.0, at) {
            blasts.0.push((at.to_world(), radius * TILE_SIZE, BLAST_SECONDS));
        }
    }
    if !virt.is_paused() {
        let dt = time.delta_secs();
        for b in &mut blasts.0 {
            b.2 -= dt;
        }
        blasts.0.retain(|b| b.2 > 0.0);
    }
    for &(p, r, life) in &blasts.0 {
        let t = 1.0 - life / BLAST_SECONDS;
        let a = 1.0 - t;
        gizmos.circle_2d(Isometry2d::from_translation(p), r * (0.3 + 0.7 * t), Color::srgba(1.0, 0.6, 0.15, a));
        gizmos.circle_2d(Isometry2d::from_translation(p), r * 0.5 * (0.3 + 0.7 * t), Color::srgba(1.0, 0.9, 0.4, a));
    }
}

/// Smoke clouds, as grey puffs on each smoky tile.
fn draw_smoke(
    map: Res<GameMap>,
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    mut gizmos: Gizmos,
) {
    for tile in map.smoke.keys() {
        if !(debug.show_all || fog.is_explored(local.0, *tile)) {
            continue;
        }
        let p = tile.to_world();
        for r in [7.0, 5.0, 3.0] {
            gizmos.circle_2d(Isometry2d::from_translation(p), r, Color::srgba(0.75, 0.75, 0.78, 0.55));
        }
    }
}
