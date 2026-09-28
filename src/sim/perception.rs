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
    visible_set: HashSet<TilePos>,
    /// Where (and on which map version) the visible tiles were computed
    /// from: recomputed on move or when terrain changes.
    computed_at: Option<(TilePos, u32)>,
}

/// Is there a clear line of sight from `from` to `to`?
///
/// Blocked by any tile in between that is higher than the viewer (ledges
/// block sight from below), by rocks and walls (except ones hugged by the
/// viewer or target, which they can peek over), or by looking too deep
/// into a forest. The
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
        // higher ground, or the obstacle is right next to the viewer or the
        // target: you can peek over or around cover you're hugging.
        let hugged = chebyshev(p, from) <= 1 || chebyshev(p, to) <= 1;
        if tile.feature.is_solid() && tile.elevation >= viewer_elevation && !hugged {
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

fn chebyshev(a: TilePos, b: TilePos) -> i32 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
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
    mut viewers: Query<(&Faction, &TilePos, &Vision, &mut Perception)>,
    targets: Query<(Entity, &Faction, &TilePos, &Unit)>,
) {
    // Stable order so fighters' target lists (and so their random rolls)
    // are the same on every machine.
    let mut all_targets: Vec<(Entity, Faction, TilePos, super::UnitId)> =
        targets.iter().map(|(e, f, p, u)| (e, *f, *p, u.id)).collect();
    all_targets.sort_by_key(|t| t.3);

    for (faction, pos, vision, mut perception) in &mut viewers {
        // Sight only needs recomputing on move or when terrain changes.
        if perception.computed_at != Some((*pos, map.version)) {
            let tiles = visible_tiles(&map, *pos, vision.radius);
            let p = perception.as_mut();
            p.visible_set = tiles.iter().copied().collect();
            p.visible_tiles = tiles;
            p.computed_at = Some((*pos, map.version));
        }
        let seen_enemies: Vec<Entity> = all_targets
            .iter()
            .filter(|(_, f, p, _)| f != faction && perception.visible_set.contains(p))
            .map(|(e, _, _, _)| *e)
            .collect();
        // Only touch the component when something changed, so the fog
        // system (which watches for changes) stays idle otherwise.
        if perception.seen_enemies != seen_enemies {
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
    fn fighters_can_peek_over_cover_they_hug() {
        let mut map = flat_map();
        map.tile_mut(TilePos::new(11, 10)).feature = Feature::Wall;
        // Viewer hugging the wall sees past it...
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(18, 10)));
        // ...and a target hugging the wall's far side can be seen too.
        map.tile_mut(TilePos::new(11, 10)).feature = Feature::None;
        map.tile_mut(TilePos::new(17, 10)).feature = Feature::Wall;
        assert!(line_of_sight(&map, TilePos::new(10, 10), TilePos::new(18, 10)));
        // But a unit two tiles behind the wall is hidden.
        assert!(!line_of_sight(&map, TilePos::new(10, 10), TilePos::new(19, 10)));
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
