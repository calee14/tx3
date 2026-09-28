//! Unit perception: what each fighter can see with its own eyes.
//!
//! This is independent of the commander's fog. Fighters always act on what
//! they perceive; the commander only learns about sectors their fighters
//! have seen into (see `fog.rs`).

use std::collections::HashSet;

use bevy::prelude::*;

use crate::faction::Faction;
use crate::map::{Feature, GameMap, TilePos};

use super::Unit;

/// Extra vision range per elevation level the viewer stands on.
const HIGH_GROUND_BONUS: f32 = 2.0;
/// Sight goes this many tree tiles deep into a forest before it's blocked.
const FOREST_SIGHT_DEPTH: u32 = 2;

#[derive(Component, Debug)]
pub struct Vision {
    /// Base sight radius in tiles.
    pub radius: f32,
}

#[derive(Component, Debug, Default)]
pub struct Perception {
    /// Tiles this unit can currently see.
    pub visible_tiles: Vec<TilePos>,
    /// Enemy units this unit can currently see.
    pub seen_enemies: Vec<Entity>,
}

/// Is there a clear line of sight from `from` to `to`?
///
/// Blocked by any tile in between that is higher than the viewer (ledges
/// block sight from below), or by looking too deep into a forest. The
/// target tile itself is always visible if the line reaches it, so a unit
/// can see the edge of a plateau or the treeline.
pub fn line_of_sight(map: &GameMap, from: TilePos, to: TilePos) -> bool {
    let viewer_elevation = map.tile(from).elevation;
    let mut trees = 0;
    let mut blocked = false;
    for_each_line_tile(from, to, |p| {
        if p == from || p == to || blocked {
            return;
        }
        let tile = map.tile(p);
        if tile.elevation > viewer_elevation {
            blocked = true;
        }
        // Rocks and walls block sight, unless you look down on them from
        // higher ground.
        if tile.feature.is_solid() && tile.elevation >= viewer_elevation {
            blocked = true;
        }
        if tile.feature == Feature::Trees {
            trees += 1;
            if trees >= FOREST_SIGHT_DEPTH {
                blocked = true;
            }
        }
    });
    !blocked
}

/// All tiles visible from `from` for a unit with the given base radius.
pub fn visible_tiles(map: &GameMap, from: TilePos, radius: f32) -> Vec<TilePos> {
    let radius = radius + HIGH_GROUND_BONUS * map.tile(from).elevation as f32;
    let r = radius.ceil() as i32;
    let mut out = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let p = from.offset(dx, dy);
            if p.in_bounds() && from.distance(p) <= radius && line_of_sight(map, from, p) {
                out.push(p);
            }
        }
    }
    out
}

/// Bresenham's line: calls `f` for every tile from `a` to `b` inclusive.
fn for_each_line_tile(a: TilePos, b: TilePos, mut f: impl FnMut(TilePos)) {
    let (mut x, mut y) = (a.x, a.y);
    let dx = (b.x - a.x).abs();
    let dy = -(b.y - a.y).abs();
    let sx = if a.x < b.x { 1 } else { -1 };
    let sy = if a.y < b.y { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        f(TilePos::new(x, y));
        if x == b.x && y == b.y {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

pub fn update_perception(
    map: Res<GameMap>,
    mut viewers: Query<(Entity, &Faction, &TilePos, &Vision, &mut Perception)>,
    targets: Query<(Entity, &Faction, &TilePos), With<Unit>>,
) {
    for (_, faction, pos, vision, mut perception) in &mut viewers {
        let tiles = visible_tiles(&map, *pos, vision.radius);
        let visible: HashSet<TilePos> = tiles.iter().copied().collect();
        let seen_enemies = targets
            .iter()
            .filter(|(_, f, p)| **f != *faction && visible.contains(p))
            .map(|(e, _, _)| e)
            .collect();
        // Only touch the component when something changed, so the fog
        // system (which watches for changes) stays idle otherwise.
        if perception.visible_tiles != tiles || perception.seen_enemies != seen_enemies {
            perception.visible_tiles = tiles;
            perception.seen_enemies = seen_enemies;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::Tile;
    use crate::map::generate::generate_map;

    fn flat_map() -> GameMap {
        let mut map = generate_map(5);
        for t in &mut map.tiles {
            *t = Tile::default();
        }
        map
    }

    #[test]
    fn open_ground_is_visible() {
        let map = flat_map();
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(18, 14)));
    }

    #[test]
    fn higher_ground_blocks_sight_from_below() {
        let mut map = flat_map();
        map.tile_mut(TilePos::new(12, 10)).elevation = 1;
        // The ledge itself is visible, what's behind it isn't.
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(12, 10)));
        assert!(!line_of_sight(&map, TilePos::new(10, 10), TilePos::new(15, 10)));
    }

    #[test]
    fn high_ground_sees_over_lower_ground() {
        let mut map = flat_map();
        map.tile_mut(TilePos::new(10, 10)).elevation = 1;
        map.tile_mut(TilePos::new(12, 10)).elevation = 1;
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(15, 10)));
    }

    #[test]
    fn obstacles_block_sight_except_from_above() {
        let mut map = flat_map();
        map.tile_mut(TilePos::new(12, 10)).feature = Feature::Rock;
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(12, 10)));
        assert!(!line_of_sight(&map, TilePos::new(10, 10), TilePos::new(15, 10)));
        map.tile_mut(TilePos::new(10, 10)).elevation = 1;
        map.tile_mut(TilePos::new(11, 10)).elevation = 1;
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(15, 10)));
    }

    #[test]
    fn forests_block_sight_after_two_tiles() {
        let mut map = flat_map();
        for x in 11..=13 {
            map.tile_mut(TilePos::new(x, 10)).feature = Feature::Trees;
        }
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(12, 10)));
        assert!(!line_of_sight(&map, TilePos::new(10, 10), TilePos::new(14, 10)));
    }
}
