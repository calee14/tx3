//! The commander's fog of war, tracked per sector.
//!
//! - A sector is revealed for a side once any of its fighters sees into it.
//! - Revealed sectors stay revealed permanently, including live enemy units.
//! - Units in revealed sectors are shown plainly (no silhouettes).
//! - Hidden traps are NOT revealed by this; they need a fighter to detect
//!   them directly (added with engineers in step 5).

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::faction::{Faction, LocalPlayer};
use crate::game::MatchEntity;
use crate::map::{self, GameMap, MAP_H, MAP_W, SECTOR_COLS, SECTOR_COUNT, TILE_SIZE, TilePos};
use crate::sim::perception::Perception;
use crate::sim::{SimSet, Unit};

pub const Z_FOG: f32 = 5.0;
/// Opacity of unrevealed sectors. Terrain stays faintly visible: maps are
/// mirrored, so players can infer it from their own half anyway.
const FOG_ALPHA: u8 = 248;

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FogDebug>()
            .add_systems(FixedUpdate, reveal_sectors.in_set(SimSet::Intel))
            .add_systems(
                Update,
                (update_fog_overlay, update_enemy_visibility).run_if(resource_exists::<FogOfWar>),
            );
    }
}

/// Per-faction list of revealed sectors.
#[derive(Resource, Debug)]
pub struct FogOfWar {
    revealed: [[bool; SECTOR_COUNT]; 2],
}

impl FogOfWar {
    /// Opening reveal: each side knows its own half of the map, plus the
    /// enemy's center-forward sector, so both see a little of what they face.
    pub fn with_opening_reveal() -> Self {
        let mut fog = Self { revealed: [[false; SECTOR_COUNT]; 2] };
        let home_rows = map::SECTOR_ROWS / 2;
        for id in 0..home_rows * SECTOR_COLS {
            fog.revealed[Faction::Humans.index()][id] = true;
            fog.revealed[Faction::Termbots.index()][map::mirrored_sector(id)] = true;
        }
        // Enemy center sector in the row just across the middle line.
        let enemy_center_forward = home_rows * SECTOR_COLS + SECTOR_COLS / 2;
        fog.revealed[Faction::Humans.index()][enemy_center_forward] = true;
        fog.revealed[Faction::Termbots.index()][map::mirrored_sector(enemy_center_forward)] = true;
        fog
    }

    pub fn is_revealed(&self, faction: Faction, sector: usize) -> bool {
        self.revealed[faction.index()][sector]
    }

    pub fn is_tile_revealed(&self, faction: Faction, pos: TilePos) -> bool {
        self.is_revealed(faction, map::sector_index(pos))
    }

    pub fn revealed_count(&self, faction: Faction) -> usize {
        self.revealed[faction.index()].iter().filter(|&&r| r).count()
    }
}

/// Debug toggle (F key): show the whole map as if every sector were revealed.
#[derive(Resource, Default)]
pub struct FogDebug {
    pub show_all: bool,
}

#[derive(Resource)]
pub struct FogOverlay(Handle<Image>);

/// Fighters' perception reveals sectors for their commander.
fn reveal_sectors(mut fog: ResMut<FogOfWar>, viewers: Query<(&Faction, &Perception), Changed<Perception>>) {
    let mut changed = false;
    // Write without triggering change detection unless a new sector opens.
    let fog_data = fog.bypass_change_detection();
    for (faction, perception) in &viewers {
        for &tile in &perception.visible_tiles {
            let sector = map::sector_index(tile);
            let slot = &mut fog_data.revealed[faction.index()][sector];
            if !*slot {
                *slot = true;
                changed = true;
                info!("{} revealed sector {}", faction.name(), sector_name(sector));
            }
        }
    }
    if changed {
        fog.set_changed();
    }
}

fn sector_name(id: usize) -> String {
    let col = id % SECTOR_COLS;
    let row = id / SECTOR_COLS;
    format!("{}{}", (b'A' + col as u8) as char, row + 1)
}

/// Spawn the dark overlay sprite: one texture pixel per tile.
pub fn spawn_fog_overlay(commands: &mut Commands, images: &mut Assets<Image>) {
    let mut image = Image::new(
        Extent3d { width: MAP_W as u32, height: MAP_H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (MAP_W * MAP_H * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    let handle = images.add(image);
    commands.insert_resource(FogOverlay(handle.clone()));
    commands.spawn((
        MatchEntity,
        Sprite {
            image: handle,
            custom_size: Some(Vec2::new(MAP_W as f32 * TILE_SIZE, MAP_H as f32 * TILE_SIZE)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, Z_FOG),
    ));
}

fn update_fog_overlay(
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    overlay: Option<Res<FogOverlay>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(overlay) = overlay else { return };
    if !(fog.is_changed() || debug.is_changed() || overlay.is_changed()) {
        return;
    }
    let Some(mut image) = images.get_mut(&overlay.0) else { return };
    let Some(data) = image.data.as_mut() else { return };
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            let hidden = !debug.show_all && !fog.is_tile_revealed(local.0, TilePos::new(x, y));
            let i = (((MAP_H - 1 - y) * MAP_W + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&[4, 5, 9, if hidden { FOG_ALPHA } else { 0 }]);
        }
    }
}

/// Enemy units are shown only in sectors the local commander has revealed.
fn update_enemy_visibility(
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    map: Option<Res<GameMap>>,
    mut units: Query<(&Faction, &TilePos, &mut Visibility), With<Unit>>,
) {
    if map.is_none() {
        return;
    }
    for (faction, pos, mut visibility) in &mut units {
        let shown = *faction == local.0 || debug.show_all || fog.is_tile_revealed(local.0, *pos);
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}
