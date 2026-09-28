//! HUD: match info, controls, the hover tooltip for fighters, and the
//! end-of-match banner.
//!
//! Hovering your own fighter shows everything: class, platoon, traits,
//! health, morale, suppression, what it has decided to do, and its
//! statuses. Hovering an enemy only shows what can be seen from outside:
//! class, rough health, and visible statuses. Its traits stay hidden.

use bevy::prelude::*;

use crate::faction::{Faction, LocalPlayer};
use crate::fog::{FogDebug, FogOfWar};
use crate::input::CursorWorld;
use crate::map::{self, GameMap, TilePos};
use crate::sim::classes::Traits;
use crate::sim::combat::MatchOutcome;
use crate::sim::fighter::{Condition, Fighter, Health, statuses};
use crate::sim::platoon::Platoon;
use crate::sim::{Mover, SimTick, Unit, UnitClass};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(Update, (update_hud, update_tooltip, update_banner).run_if(resource_exists::<GameMap>));
    }
}

#[derive(Component)]
struct HudText;

#[derive(Component)]
struct Tooltip;

#[derive(Component)]
struct TooltipText;

#[derive(Component)]
struct Banner;

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(8.0),
            top: Val::Px(8.0),
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)),
        children![(HudText, Text::new(""), TextFont::from_font_size(13.0), TextColor(Color::WHITE))],
    ));
    commands.spawn((
        Tooltip,
        Node { position_type: PositionType::Absolute, padding: UiRect::all(Val::Px(8.0)), ..default() },
        BackgroundColor(Color::srgba(0.05, 0.05, 0.08, 0.9)),
        Visibility::Hidden,
        GlobalZIndex(10),
        children![(TooltipText, Text::new(""), TextFont::from_font_size(13.0), TextColor(Color::WHITE))],
    ));
    commands.spawn((
        Banner,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Percent(40.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Visibility::Hidden,
        children![(Text::new(""), TextFont::from_font_size(48.0), TextColor(Color::WHITE))],
    ));
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    map: Res<GameMap>,
    tick: Res<SimTick>,
    fog: Res<FogOfWar>,
    fog_debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    time: Res<Time<Virtual>>,
    cursor: Res<CursorWorld>,
    units: Query<(&Faction, Option<&Fighter>)>,
    mut text: Single<&mut Text, With<HudText>>,
) {
    let seconds = tick.0 as f64 / crate::sim::SIM_HZ;
    let count = |f: Faction| units.iter().filter(|(uf, fi)| **uf == f && fi.is_some()).count();
    let mut lines = vec![
        format!("TERMBOTS vs HUMANS  -  prototype step 2   (you: {})", local.0.name()),
        format!(
            "Map seed {}   Time {:02}:{:02}{}",
            map.seed,
            (seconds / 60.0) as u32,
            (seconds % 60.0) as u32,
            if time.is_paused() { "   [PAUSED]" } else { "" }
        ),
        format!(
            "Map explored: {:.0}%{}   Fighters: {} Humans, {} Termbots",
            fog.explored_fraction(local.0) * 100.0,
            if fog_debug.show_all { " (fog hidden)" } else { "" },
            count(Faction::Humans),
            count(Faction::Termbots),
        ),
    ];

    if let Some(tile) = cursor.tile() {
        let t = map.tile(tile);
        let sector = &map.sectors[map::sector_index(tile)];
        let mut desc = format!(
            "Tile ({}, {})  {}  {}  {}  ({})",
            tile.x,
            tile.y,
            sector.name,
            map::elevation_name(t.elevation),
            t.feature.name(),
            if t.passable() { map.cover_at(tile).name() } else { "impassable" }
        );
        if t.ramp {
            desc.push_str("  Ramp");
        }
        lines.push(desc);
    }

    lines.push(String::new());
    lines.push("Left-click a fighter: select its platoon".into());
    lines.push("1/2/3 or buttons: mission   Right-click: target".into());
    lines.push("Hover a fighter for details".into());
    lines.push("WASD/arrows/middle-drag: pan   Wheel: zoom".into());
    lines.push("Space: pause   F: toggle fog   N: new map".into());
    text.0 = lines.join("\n");
}

type HoverQuery<'a> = (
    Entity,
    &'a Unit,
    &'a Faction,
    &'a TilePos,
    &'a Transform,
    &'a Health,
    &'a Mover,
    Option<&'a Fighter>,
    Option<&'a Traits>,
    Option<&'a Condition>,
);

#[allow(clippy::too_many_arguments)]
fn update_tooltip(
    map: Res<GameMap>,
    cursor: Res<CursorWorld>,
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    window: Single<&Window>,
    platoons: Query<&Platoon>,
    units: Query<HoverQuery>,
    mut tooltip: Single<(&mut Node, &mut Visibility), With<Tooltip>>,
    mut text: Single<&mut Text, With<TooltipText>>,
) {
    let hovered = cursor.world.filter(|_| !cursor.over_ui).and_then(|w| {
        units
            .iter()
            .filter(|u| *u.2 == local.0 || debug.show_all || fog.is_explored(local.0, *u.3))
            .map(|u| (u.4.translation.truncate().distance(w), u))
            .filter(|(d, _)| *d <= 10.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, u)| u)
    });
    let (node, visibility) = &mut *tooltip;
    let Some((entity, unit, faction, pos, _, health, mover, fighter, traits, cond)) = hovered else {
        **visibility = Visibility::Hidden;
        return;
    };

    let own = *faction == local.0;
    let mut lines = vec![format!("{} {}", faction.name(), unit.class.name())];
    let platoon = fighter.and_then(|f| platoons.get(f.platoon).ok());

    if unit.class == UnitClass::King {
        lines.push(format!("Health {:.0}/{:.0}", health.current.max(0.0), health.max));
    } else if own {
        if let Some(p) = platoon {
            let leader = if p.leader == Some(entity) { "  (Leader)" } else { "" };
            lines.push(format!("{}{}  -  mission: {}", p.name, leader, p.mission.name()));
        }
        lines.push(format!("Health {:.0}/{:.0}", health.current.max(0.0), health.max));
        if let Some(c) = cond {
            lines.push(format!("Morale {:.0}   Suppression {:.0}", c.morale, c.suppression));
            lines.push(format!("Doing: {}", c.decision.name()));
        }
        if let Some(t) = traits {
            lines.push(format!(
                "Accuracy {}  IQ {}  Tactics {}  Bravery {}  Leadership {}",
                t.accuracy, t.iq, t.tactics, t.bravery, t.leadership
            ));
        }
    } else {
        let condition = match health.frac() {
            f if f > 0.6 => "healthy",
            f if f > 0.3 => "wounded",
            _ => "badly wounded",
        };
        lines.push(format!("Looks {condition}. Traits unknown."));
    }

    if let Some(c) = cond {
        let confusion = platoon.map_or(0.0, |p| p.confusion);
        let list: Vec<&str> = statuses(c, health, confusion, !mover.path.is_empty(), map.cover_at(*pos))
            .into_iter()
            .filter(|s| own || s.visible_to_enemy())
            .map(|s| s.name())
            .collect();
        if !list.is_empty() {
            lines.push(list.join(", "));
        }
    }

    text.0 = lines.join("\n");
    **visibility = Visibility::Inherited;
    if let Some(cursor_px) = window.cursor_position() {
        node.left = Val::Px(cursor_px.x + 16.0);
        node.top = Val::Px(cursor_px.y + 16.0);
    }
}

fn update_banner(
    outcome: Option<Res<MatchOutcome>>,
    local: Res<LocalPlayer>,
    mut banner: Single<(&mut Visibility, &Children), With<Banner>>,
    mut texts: Query<&mut Text, Without<HudText>>,
) {
    let (visibility, children) = &mut *banner;
    let Some(winner) = outcome.and_then(|o| o.winner) else {
        **visibility = Visibility::Hidden;
        return;
    };
    **visibility = Visibility::Inherited;
    for child in children.iter() {
        if let Ok(mut t) = texts.get_mut(child) {
            t.0 = if winner == local.0 {
                "VICTORY - the enemy King is down (N: new map)".into()
            } else {
                "DEFEAT - your King is down (N: new map)".into()
            };
        }
    }
}
