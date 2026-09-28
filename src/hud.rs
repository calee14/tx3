//! Debug HUD: match info, what's under the cursor, and the controls.

use bevy::prelude::*;

use crate::faction::LocalPlayer;
use crate::fog::{FogDebug, FogOfWar};
use crate::input::{CursorWorld, Selection};
use crate::map::{self, GameMap};
use crate::sim::perception::Perception;
use crate::sim::{SimTick, Unit};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud)
            .add_systems(Update, update_hud.run_if(resource_exists::<GameMap>));
    }
}

#[derive(Component)]
struct HudText;

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
        children![(HudText, Text::new(""), TextFont::from_font_size(14.0), TextColor(Color::WHITE))],
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
    selection: Res<Selection>,
    units: Query<(&Unit, &map::TilePos, &Perception)>,
    mut text: Single<&mut Text, With<HudText>>,
) {
    let seconds = tick.0 as f64 / crate::sim::SIM_HZ;
    let mut lines = vec![
        format!("TERMBOTS vs HUMANS  -  prototype step 1   (you: {})", local.0.name()),
        format!(
            "Map seed {}   Time {:02}:{:02}{}",
            map.seed,
            (seconds / 60.0) as u32,
            (seconds % 60.0) as u32,
            if time.is_paused() { "   [PAUSED]" } else { "" }
        ),
        format!(
            "Map explored: {:.0}%{}",
            fog.explored_fraction(local.0) * 100.0,
            if fog_debug.show_all { "   (fog hidden: debug)" } else { "" }
        ),
    ];

    if let Some(tile) = cursor.tile() {
        let t = map.tile(tile);
        let sector = &map.sectors[map::sector_index(tile)];
        let mut desc = format!(
            "Tile ({}, {})  Sector {}  {}  {}  ({})",
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

    if let Some((unit, pos, perception)) = selection.0.and_then(|e| units.get(e).ok()) {
        lines.push(format!(
            "Selected: {} at ({}, {})  sees {} tiles, {} enemies",
            unit.class.name(),
            pos.x,
            pos.y,
            perception.visible_tiles.len(),
            perception.seen_enemies.len()
        ));
    }

    lines.push(String::new());
    lines.push("Left-click: select King   Right-click: move".into());
    lines.push("WASD / arrows / middle-drag: pan   Wheel: zoom".into());
    lines.push("Space: pause   F: toggle fog   N: new map".into());

    text.0 = lines.join("\n");
}
