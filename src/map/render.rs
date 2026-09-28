//! Placeholder map rendering: the whole map is painted into one texture
//! (8 pixels per tile) and shown as a single sprite. Cheap to draw and
//! easy to replace with real tile art later.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use super::*;
use crate::game::MatchEntity;

const PX: i32 = 8; // texture pixels per tile

pub const Z_MAP: f32 = 0.0;
pub const Z_LABELS: f32 = 30.0;

type Rgb = [f32; 3];

fn ground_color(elevation: u8) -> Rgb {
    match elevation {
        0 => [0.33, 0.43, 0.27],
        1 => [0.47, 0.52, 0.33],
        _ => [0.62, 0.61, 0.44],
    }
}

fn scale(c: Rgb, k: f32) -> Rgb {
    [c[0] * k, c[1] * k, c[2] * k]
}

/// Paint one tile into an 8x8 block. `lx`, `ly` are pixel coordinates
/// inside the tile with (0, 0) at the bottom-left.
fn tile_pixel(map: &GameMap, pos: TilePos, lx: i32, ly: i32) -> Rgb {
    let tile = map.tile(pos);
    let mut color = ground_color(tile.elevation);

    // Subtle checker so individual tiles are readable when zoomed in.
    if (pos.x + pos.y) % 2 == 0 {
        color = scale(color, 0.96);
    }

    // Ramps: bright diagonal stripes.
    if tile.ramp && (lx + ly) % 4 < 2 {
        color = [0.86, 0.80, 0.58];
    }

    match tile.feature {
        Feature::None => {}
        Feature::Trees => {
            // A round canopy, with a little variation per tile.
            let jitter = ((pos.x * 7 + pos.y * 13) % 3) as f32 * 0.03;
            let d = Vec2::new(lx as f32 - 3.5, ly as f32 - 3.5).length();
            if d < 3.6 {
                color = [0.12 + jitter, 0.29 + jitter, 0.14];
                if d < 1.6 {
                    color = [0.17 + jitter, 0.37 + jitter, 0.18];
                }
            }
        }
        Feature::LightCover => {
            if (1..=6).contains(&lx) && (2..=5).contains(&ly) {
                color = if ly == 2 || lx == 1 { [0.55, 0.47, 0.30] } else { [0.78, 0.68, 0.46] };
            }
        }
        Feature::Rock | Feature::Wall | Feature::Vehicle => {
            let rock = tile.feature == Feature::Rock;
            // Solid fill with a dark outline wherever the obstacle ends, so
            // multi-tile outcrops and ruins read as one shape.
            let same = |dx: i32, dy: i32| {
                let n = pos.offset(dx, dy);
                n.in_bounds() && map.tile(n).feature == tile.feature
            };
            let outline = (lx == 0 && !same(-1, 0))
                || (lx == PX - 1 && !same(1, 0))
                || (ly == 0 && !same(0, -1))
                || (ly == PX - 1 && !same(0, 1));
            color = if outline {
                [0.14, 0.13, 0.13]
            } else if rock {
                // Speckled stone, lighter on top-left facets.
                let n = ((pos.x * 31 + pos.y * 17 + lx * 7 + ly * 13) % 9) as f32 / 9.0;
                let light = if lx + (PX - 1 - ly) < 6 { 0.08 } else { 0.0 };
                [0.47 + n * 0.08 + light, 0.44 + n * 0.07 + light, 0.40 + n * 0.06 + light]
            } else {
                // Concrete blocks.
                let seam = ly % 4 == 0 || (lx + if (ly / 4) % 2 == 0 { 0 } else { 4 }) % 8 == 0;
                if seam { [0.36, 0.35, 0.37] } else { [0.58, 0.57, 0.60] }
            };
        }
    }

    // Ledges: shade the edge of a tile that drops to a lower neighbor, and
    // draw a thick shadow line when the drop has no ramp.
    let checks = [(lx == PX - 1, 1, 0), (lx == 0, -1, 0), (ly == PX - 1, 0, 1), (ly == 0, 0, -1)];
    for (on_edge, dx, dy) in checks {
        let n = pos.offset(dx, dy);
        if on_edge && n.in_bounds() && map.tile(n).elevation < tile.elevation {
            color = if tile.ramp { scale(color, 0.8) } else { [0.12, 0.12, 0.10] };
        }
    }
    color
}

/// Paint one tile's pixels into the RGBA texture data.
fn paint_tile(map: &GameMap, pos: TilePos, data: &mut [u8]) {
    let w = (MAP_W * PX) as u32;
    for ly in 0..PX {
        for lx in 0..PX {
            let c = tile_pixel(map, pos, lx, ly);
            // Texture rows go top-down; the map's y goes bottom-up.
            let px = (pos.x * PX + lx) as u32;
            let py = ((MAP_H - 1 - pos.y) * PX + (PX - 1 - ly)) as u32;
            let i = ((py * w + px) * 4) as usize;
            for (k, channel) in c.iter().enumerate() {
                data[i + k] = (channel.clamp(0.0, 1.0) * 255.0) as u8;
            }
            data[i + 3] = 255;
        }
    }
}

pub fn build_map_image(map: &GameMap) -> Image {
    let (w, h) = ((MAP_W * PX) as u32, (MAP_H * PX) as u32);
    let mut data = vec![0u8; (w * h * 4) as usize];
    for y in 0..MAP_H {
        for x in 0..MAP_W {
            paint_tile(map, TilePos::new(x, y), &mut data);
        }
    }
    let mut image = Image::new(
        Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Spawn the map sprite and sector labels for a new match.
pub fn spawn_map_visuals(commands: &mut Commands, images: &mut Assets<Image>, map: &GameMap) {
    let handle = images.add(build_map_image(map));
    commands.insert_resource(MapImage { handle: handle.clone(), painted: map.terrain_log.len() });
    commands.spawn((
        MatchEntity,
        Sprite {
            image: handle,
            custom_size: Some(Vec2::new(MAP_W as f32 * TILE_SIZE, MAP_H as f32 * TILE_SIZE)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, Z_MAP),
    ));

    for sector in &map.sectors {
        commands.spawn((
            MatchEntity,
            SectorLabel(sector.id),
            Text2d::new(sector.name.clone()),
            TextFont::from_font_size(40.0),
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.35)),
            Transform::from_translation(sector.center_world().extend(Z_LABELS)),
        ));
    }
}

/// The map texture, and how much of the terrain log it has painted.
#[derive(Resource)]
pub struct MapImage {
    handle: Handle<Image>,
    painted: usize,
}

/// Repaint tiles whose terrain changed during the match (a parked
/// vehicle, a blown-up wall), plus their neighbors (outlines).
pub fn repaint_changed_terrain(map: Res<GameMap>, image: Option<ResMut<MapImage>>, mut images: ResMut<Assets<Image>>) {
    let Some(mut image) = image else { return };
    if map.terrain_log.len() <= image.painted {
        return;
    }
    let start = image.painted;
    image.painted = map.terrain_log.len();
    let Some(mut texture) = images.get_mut(&image.handle) else { return };
    let Some(data) = texture.data.as_mut() else { return };
    for &pos in &map.terrain_log[start..] {
        for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
            let p = pos.offset(dx, dy);
            if p.in_bounds() {
                paint_tile(&map, p, data);
            }
        }
    }
}

/// Marks a sector's name label, so it can be tinted by who controls it.
#[derive(Component)]
pub struct SectorLabel(pub usize);

/// Sector names take the color of the side that controls them.
pub fn color_sector_labels(
    territory: Option<Res<crate::sim::economy::Territory>>,
    mut labels: Query<(&SectorLabel, &mut TextColor)>,
) {
    let Some(territory) = territory else { return };
    for (label, mut color) in &mut labels {
        let c = match territory.owner[label.0] {
            Some(faction) => faction.color().with_alpha(0.55),
            None => Color::srgba(1.0, 1.0, 1.0, 0.35),
        };
        if color.0 != c {
            color.0 = c;
        }
    }
}

/// Sector borders, drawn every frame with gizmos.
pub fn draw_sector_borders(mut gizmos: Gizmos) {
    let half = Vec2::new(MAP_W as f32, MAP_H as f32) * TILE_SIZE / 2.0;
    let color = Color::srgba(1.0, 1.0, 1.0, 0.3);
    for &x in &SECTOR_COL_EDGES {
        let wx = x as f32 * TILE_SIZE - half.x;
        gizmos.line_2d(Vec2::new(wx, -half.y), Vec2::new(wx, half.y), color);
    }
    for &y in &SECTOR_ROW_EDGES {
        let wy = y as f32 * TILE_SIZE - half.y;
        // The middle line splits the two halves of the map: draw it brighter.
        let c = if y == MAP_H / 2 { Color::srgba(1.0, 1.0, 1.0, 0.6) } else { color };
        gizmos.line_2d(Vec2::new(-half.x, wy), Vec2::new(half.x, wy), c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::generate::generate_map;

    /// Writes map previews as PPM images for eyeballing the generator:
    /// `MAP_PREVIEW_DIR=out cargo test dump_map_previews -- --ignored`
    #[test]
    #[ignore]
    fn dump_map_previews() {
        let dir = std::env::var("MAP_PREVIEW_DIR").unwrap_or_else(|_| ".".into());
        for seed in [1u64, 42, 2026] {
            let map = generate_map(seed);
            let image = build_map_image(&map);
            let (w, h) = (image.width(), image.height());
            let data = image.data.as_ref().unwrap();
            let mut out = format!("P6 {w} {h} 255\n").into_bytes();
            for px in data.chunks(4) {
                out.extend_from_slice(&px[..3]);
            }
            std::fs::write(format!("{dir}/map_seed_{}.ppm", map.seed), out).unwrap();
        }
    }
}
