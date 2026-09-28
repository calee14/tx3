//! The commander's fog of war, tracked per tile.
//!
//! - Each side starts with a large circle around its own King revealed,
//!   reaching a little past the middle of the map.
//! - Fighters uncover the map as they move: any tile a fighter sees becomes
//!   explored for its commander, permanently.
//! - Enemy units on explored tiles are shown plainly (no silhouettes).
//! - Hidden traps are NOT revealed by this; they need a fighter to detect
//!   them directly (added with engineers in step 5).
//!
//! Fighters' own perception is separate (see `sim/perception.rs`): the fog
//! never limits what units can see or react to.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::faction::{Faction, LocalPlayer};
use crate::game::MatchEntity;
use crate::map::{GameMap, MAP_H, MAP_W, TILE_SIZE, TilePos};
use crate::sim::perception::Perception;
use crate::sim::{SimSet, Unit};

pub const Z_FOG: f32 = 5.0;
/// Opacity of unexplored ground. Fully dark: maps are random, so the
/// terrain there is unknown until someone scouts it. (Lower this, e.g. to
/// 248, to show a faint outline of the terrain under the fog.)
const FOG_ALPHA: u8 = 255;
/// Radius (in tiles) around each King that starts out explored. Kings
/// start 56 tiles from the middle line, so this covers most of your own
/// half and peeks about 6 tiles into the enemy's: the opening glimpse of
/// what you're facing.
pub const START_REVEAL_RADIUS: f32 = 63.0;

pub struct FogPlugin;

impl Plugin for FogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FogDebug>()
            .add_systems(FixedUpdate, reveal_tiles.in_set(SimSet::Intel))
            .add_systems(
                Update,
                (update_fog_overlay, update_enemy_visibility).run_if(resource_exists::<FogOfWar>),
            );
    }
}

fn tile_index(pos: TilePos) -> usize {
    (pos.y * MAP_W + pos.x) as usize
}

/// Which tiles each side's commander has explored.
#[derive(Resource, Debug)]
pub struct FogOfWar {
    explored: [Vec<bool>; 2],
}

impl FogOfWar {
    /// Opening state: a circle around each side's own King.
    pub fn new(map: &GameMap) -> Self {
        let tiles = (MAP_W * MAP_H) as usize;
        let mut fog = Self { explored: [vec![false; tiles], vec![false; tiles]] };
        for faction in Faction::ALL {
            let spawn = map.spawn(faction);
            for y in 0..MAP_H {
                for x in 0..MAP_W {
                    let p = TilePos::new(x, y);
                    if p.distance(spawn) <= START_REVEAL_RADIUS {
                        fog.explored[faction.index()][tile_index(p)] = true;
                    }
                }
            }
        }
        fog
    }

    pub fn is_explored(&self, faction: Faction, pos: TilePos) -> bool {
        self.explored[faction.index()][tile_index(pos)]
    }

    /// Fraction of the map this side has explored, 0 to 1.
    pub fn explored_fraction(&self, faction: Faction) -> f32 {
        let tiles = &self.explored[faction.index()];
        tiles.iter().filter(|&&e| e).count() as f32 / tiles.len() as f32
    }
}

/// Debug toggle (F key): show the whole map as if it were all explored.
#[derive(Resource, Default)]
pub struct FogDebug {
    pub show_all: bool,
}

#[derive(Resource)]
pub struct FogOverlay(Handle<Image>);

/// Whatever fighters see becomes explored for their commander.
fn reveal_tiles(mut fog: ResMut<FogOfWar>, viewers: Query<(&Faction, &Perception), Changed<Perception>>) {
    let mut changed = false;
    // Write without triggering change detection unless new ground opens up.
    let fog_data = fog.bypass_change_detection();
    for (faction, perception) in &viewers {
        let explored = &mut fog_data.explored[faction.index()];
        for &tile in &perception.visible_tiles {
            let slot = &mut explored[tile_index(tile)];
            if !*slot {
                *slot = true;
                changed = true;
            }
        }
    }
    if changed {
        fog.set_changed();
    }
}

/// Spawn the dark overlay sprite: one texture pixel per tile, sampled with
/// linear filtering so the fog has soft edges.
pub fn spawn_fog_overlay(commands: &mut Commands, images: &mut Assets<Image>) {
    let mut image = Image::new(
        Extent3d { width: MAP_W as u32, height: MAP_H as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (MAP_W * MAP_H * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = ImageSampler::linear();
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
            let hidden = !debug.show_all && !fog.is_explored(local.0, TilePos::new(x, y));
            let i = (((MAP_H - 1 - y) * MAP_W + x) * 4) as usize;
            data[i..i + 4].copy_from_slice(&[4, 5, 9, if hidden { FOG_ALPHA } else { 0 }]);
        }
    }
}

/// Enemy units are shown only on ground the local commander has explored.
fn update_enemy_visibility(
    fog: Res<FogOfWar>,
    debug: Res<FogDebug>,
    local: Res<LocalPlayer>,
    mut units: Query<(&Faction, &TilePos, &mut Visibility), With<Unit>>,
) {
    for (faction, pos, mut visibility) in &mut units {
        let shown = *faction == local.0 || debug.show_all || fog.is_explored(local.0, *pos);
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
}
