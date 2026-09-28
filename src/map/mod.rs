//! The battlefield: a tile grid with elevation, ramps, trees and cover,
//! divided into 12 sectors.
//!
//! Everything in this module is plain game data with no rendering, so it
//! can be unit-tested and later run on a headless server.

pub mod generate;
pub mod noise;
pub mod render;

use bevy::prelude::*;

use crate::faction::Faction;

pub const MAP_W: i32 = 128;
pub const MAP_H: i32 = 128;

/// Size of one tile in world units (pixels at zoom 1).
pub const TILE_SIZE: f32 = 16.0;

/// Sector grid: 3 columns x 4 rows = 12 sectors.
/// Column edges are chosen so the layout is symmetric under the map's
/// 180-degree mirror (columns 43 / 42 / 43 tiles wide).
pub const SECTOR_COL_EDGES: [i32; 4] = [0, 43, 85, MAP_W];
pub const SECTOR_ROW_EDGES: [i32; 5] = [0, 32, 64, 96, MAP_H];
pub const SECTOR_COLS: usize = 3;
pub const SECTOR_ROWS: usize = 4;
pub const SECTOR_COUNT: usize = SECTOR_COLS * SECTOR_ROWS;

/// Maximum elevation level (0 = lowland, 1 = hills, 2 = high ground).
pub const MAX_ELEVATION: u8 = 2;

/// A tile coordinate. (0, 0) is the bottom-left corner of the map.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TilePos {
    pub x: i32,
    pub y: i32,
}

impl TilePos {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// The matching tile on the other side of the map (180-degree
    /// rotation). Used for spawns and sector layout, which stay symmetric
    /// even though the terrain is random.
    pub const fn mirrored(self) -> Self {
        Self::new(MAP_W - 1 - self.x, MAP_H - 1 - self.y)
    }

    pub fn in_bounds(self) -> bool {
        self.x >= 0 && self.y >= 0 && self.x < MAP_W && self.y < MAP_H
    }

    /// Straight-line distance in tiles.
    pub fn distance(self, other: TilePos) -> f32 {
        let dx = (self.x - other.x) as f32;
        let dy = (self.y - other.y) as f32;
        (dx * dx + dy * dy).sqrt()
    }

    /// World-space center of this tile. The map is centered on the origin.
    pub fn to_world(self) -> Vec2 {
        Vec2::new(
            (self.x as f32 - MAP_W as f32 / 2.0 + 0.5) * TILE_SIZE,
            (self.y as f32 - MAP_H as f32 / 2.0 + 0.5) * TILE_SIZE,
        )
    }

    /// The tile under a world-space point, if it is on the map.
    pub fn from_world(world: Vec2) -> Option<TilePos> {
        let x = (world.x / TILE_SIZE + MAP_W as f32 / 2.0).floor() as i32;
        let y = (world.y / TILE_SIZE + MAP_H as f32 / 2.0).floor() as i32;
        let pos = TilePos::new(x, y);
        pos.in_bounds().then_some(pos)
    }

    pub fn offset(self, dx: i32, dy: i32) -> TilePos {
        TilePos::new(self.x + dx, self.y + dy)
    }
}

/// What sits on top of a tile's ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Feature {
    #[default]
    None,
    /// Concealment and light cover; slows movement.
    Trees,
    /// Sandbags, craters, debris. Walkable; light cover for whoever
    /// stands in it. Destroyed by Explosive.
    LightCover,
    /// Rock outcrop. Solid: blocks movement and sight. Units next to it
    /// get heavy cover.
    Rock,
    /// Ruined building wall or concrete barrier. Solid like rock; Tanks and
    /// Explosive will be able to break it (later).
    Wall,
    /// A parked (or wrecked) troop vehicle. Appears during battle when a
    /// vehicle deployment reaches its spot. Solid like rock.
    Vehicle,
}

impl Feature {
    pub fn name(self) -> &'static str {
        match self {
            Feature::None => "Open ground",
            Feature::Trees => "Trees",
            Feature::LightCover => "Sandbags",
            Feature::Rock => "Rock",
            Feature::Wall => "Wall",
            Feature::Vehicle => "Parked vehicle",
        }
    }

    /// Solid obstacles can't be walked through and block line of sight.
    pub fn is_solid(self) -> bool {
        matches!(self, Feature::Rock | Feature::Wall | Feature::Vehicle)
    }
}

/// The 8 neighboring directions: E, W, N, S, NE, SE, NW, SW.
pub const DIRS8: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
/// An obstacle protects against shots whose direction is within this cosine
/// of the direction to the obstacle (0.8 = about 37 degrees either side).
/// A straight neighbor covers straight-on shots, a diagonal one covers
/// diagonal shots, and a corner (both) covers everything in between.
const COVER_FACING: f32 = 0.8;

/// How protected a unit standing on a tile is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoverLevel {
    None,
    Light,
    Heavy,
}

impl CoverLevel {
    pub fn name(self) -> &'static str {
        match self {
            CoverLevel::None => "No cover",
            CoverLevel::Light => "Light cover",
            CoverLevel::Heavy => "Heavy cover",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Tile {
    /// 0 = lowland, 1 = hills, 2 = high ground. Edges between levels are
    /// ledges: impassable except at ramps, and they block sight from below.
    pub elevation: u8,
    /// A ramp sits on the higher level and connects to the level below.
    pub ramp: bool,
    pub feature: Feature,
}

impl Tile {
    /// Movement cost multiplier for entering this tile (1 = open ground).
    /// Not meaningful for solid tiles, which can't be entered at all.
    pub fn move_cost(self) -> u32 {
        match self.feature {
            Feature::None => 1,
            Feature::Trees | Feature::LightCover => 2,
            Feature::Rock | Feature::Wall | Feature::Vehicle => u32::MAX / 64,
        }
    }

    pub fn passable(self) -> bool {
        !self.feature.is_solid()
    }
}

pub fn elevation_name(elevation: u8) -> &'static str {
    match elevation {
        0 => "Lowland",
        1 => "Hills",
        _ => "High ground",
    }
}

/// One territory zone. Territory control over sectors decides the match
/// when time runs out.
#[derive(Clone, Debug)]
#[allow(dead_code)] // `id` and `home_of` are used by territory control (step 4)
pub struct Sector {
    pub id: usize,
    pub name: String,
    /// Inclusive min corner, exclusive max corner, in tiles.
    pub min: TilePos,
    pub max: TilePos,
    /// Which side's half of the map this sector is in.
    pub home_of: Faction,
}

impl Sector {
    pub fn center_world(&self) -> Vec2 {
        let min = Vec2::new(self.min.x as f32, self.min.y as f32);
        let max = Vec2::new(self.max.x as f32, self.max.y as f32);
        let center_tiles = (min + max) / 2.0;
        Vec2::new(
            (center_tiles.x - MAP_W as f32 / 2.0) * TILE_SIZE,
            (center_tiles.y - MAP_H as f32 / 2.0) * TILE_SIZE,
        )
    }
}

/// Sector index for a tile. Row 0 is the bottom (Humans' home row).
pub fn sector_index(pos: TilePos) -> usize {
    let col = SECTOR_COL_EDGES[1..].iter().position(|&edge| pos.x < edge).unwrap_or(SECTOR_COLS - 1);
    let row = SECTOR_ROW_EDGES[1..].iter().position(|&edge| pos.y < edge).unwrap_or(SECTOR_ROWS - 1);
    row * SECTOR_COLS + col
}

/// The sector on the other side of the map that mirrors this one.
#[allow(dead_code)] // used by tests now, territory control later
pub fn mirrored_sector(id: usize) -> usize {
    SECTOR_COUNT - 1 - id
}

pub fn build_sectors() -> Vec<Sector> {
    let mut sectors = Vec::with_capacity(SECTOR_COUNT);
    for row in 0..SECTOR_ROWS {
        for col in 0..SECTOR_COLS {
            let id = row * SECTOR_COLS + col;
            let letter = (b'A' + col as u8) as char;
            sectors.push(Sector {
                id,
                name: format!("{letter}{}", row + 1),
                min: TilePos::new(SECTOR_COL_EDGES[col], SECTOR_ROW_EDGES[row]),
                max: TilePos::new(SECTOR_COL_EDGES[col + 1], SECTOR_ROW_EDGES[row + 1]),
                home_of: if row < SECTOR_ROWS / 2 { Faction::Humans } else { Faction::Termbots },
            });
        }
    }
    sectors
}

/// The generated battlefield. Shared by the simulation and rendering.
#[derive(Resource, Clone)]
pub struct GameMap {
    /// Seed that produced this exact map (may differ from the requested
    /// seed if the generator had to reroll).
    pub seed: u64,
    pub tiles: Vec<Tile>,
    pub sectors: Vec<Sector>,
    /// Starting King position per faction, indexed by `Faction::index()`.
    pub spawns: [TilePos; 2],
    /// Bumped whenever terrain changes during a match (e.g. a vehicle
    /// parks), so cached line-of-sight gets recomputed.
    pub version: u32,
}

impl GameMap {
    fn idx(pos: TilePos) -> usize {
        (pos.y * MAP_W + pos.x) as usize
    }

    pub fn tile(&self, pos: TilePos) -> Tile {
        self.tiles[Self::idx(pos)]
    }

    pub fn tile_mut(&mut self, pos: TilePos) -> &mut Tile {
        &mut self.tiles[Self::idx(pos)]
    }

    /// Is this tile on the given side's half of the map?
    pub fn in_home_half(&self, faction: Faction, pos: TilePos) -> bool {
        match faction {
            Faction::Humans => pos.y < MAP_H / 2,
            Faction::Termbots => pos.y >= MAP_H / 2,
        }
    }

    pub fn spawn(&self, faction: Faction) -> TilePos {
        self.spawns[faction.index()]
    }

    /// Can a unit step directly from `a` to the neighboring tile `b`?
    ///
    /// - Never into a solid obstacle (rock, wall).
    /// - Same elevation: always.
    /// - One level apart (orthogonal steps only): only if the higher tile
    ///   is a ramp.
    /// - Diagonal steps: only across flat ground, and never cutting a
    ///   corner past a ledge or an obstacle.
    pub fn can_step(&self, a: TilePos, b: TilePos) -> bool {
        if !b.in_bounds() || !a.in_bounds() {
            return false;
        }
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        if dx.abs() > 1 || dy.abs() > 1 || (dx == 0 && dy == 0) {
            return false;
        }
        let ta = self.tile(a);
        let tb = self.tile(b);
        if !ta.passable() || !tb.passable() {
            return false;
        }
        if dx != 0 && dy != 0 {
            // Diagonal: all four tiles involved must be on the same level,
            // and the two corner tiles must be open.
            let c1 = self.tile(TilePos::new(a.x + dx, a.y));
            let c2 = self.tile(TilePos::new(a.x, a.y + dy));
            return ta.elevation == tb.elevation
                && c1.elevation == ta.elevation
                && c2.elevation == ta.elevation
                && c1.passable()
                && c2.passable();
        }
        match ta.elevation.abs_diff(tb.elevation) {
            0 => true,
            1 => {
                if ta.elevation > tb.elevation {
                    ta.ramp
                } else {
                    tb.ramp
                }
            }
            _ => false,
        }
    }

    /// Protection a unit on `target` has against fire coming from `from`.
    ///
    /// Cover is directional: a rock, wall or parked vehicle only protects
    /// if it's on the neighboring tile facing the shooter (within ~37
    /// degrees), and a sandbag line in front gives light cover the same way.
    /// Standing in sandbags or trees gives light cover from every side.
    pub fn cover_against(&self, target: TilePos, from: TilePos) -> CoverLevel {
        let toward = Vec2::new((from.x - target.x) as f32, (from.y - target.y) as f32);
        let mut best = match self.tile(target).feature {
            Feature::LightCover | Feature::Trees => CoverLevel::Light,
            _ => CoverLevel::None,
        };
        if toward.length_squared() < 0.25 {
            return best;
        }
        let toward = toward.normalize();
        for (dx, dy) in DIRS8 {
            let n = target.offset(dx, dy);
            if !n.in_bounds() || Vec2::new(dx as f32, dy as f32).normalize().dot(toward) < COVER_FACING {
                continue;
            }
            match self.tile(n).feature {
                f if f.is_solid() => return CoverLevel::Heavy,
                Feature::LightCover => best = best.max(CoverLevel::Light),
                _ => {}
            }
        }
        best
    }

    /// Compass directions (N, NE, ...) in which `pos` has a solid obstacle
    /// right next to it, i.e. directions it has heavy cover from.
    pub fn cover_facings(&self, pos: TilePos) -> Vec<&'static str> {
        const NAMES: [&str; 8] = ["E", "W", "N", "S", "NE", "SE", "NW", "SW"];
        DIRS8
            .iter()
            .zip(NAMES)
            .filter(|((dx, dy), _)| {
                let n = pos.offset(*dx, *dy);
                n.in_bounds() && self.tile(n).feature.is_solid()
            })
            .map(|(_, name)| name)
            .collect()
    }

    /// The best protection `pos` offers from any direction (used when no
    /// threat direction is known).
    pub fn cover_at(&self, pos: TilePos) -> CoverLevel {
        let beside_obstacle = DIRS8
            .into_iter()
            .map(|(dx, dy)| pos.offset(dx, dy))
            .any(|n| n.in_bounds() && self.tile(n).feature.is_solid());
        if beside_obstacle {
            return CoverLevel::Heavy;
        }
        match self.tile(pos).feature {
            Feature::LightCover | Feature::Trees => CoverLevel::Light,
            _ => CoverLevel::None,
        }
    }

    /// All tiles a unit can step to from `pos`.
    pub fn neighbors(&self, pos: TilePos) -> impl Iterator<Item = TilePos> + '_ {
        const DIRS: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
        DIRS.iter()
            .map(move |&(dx, dy)| pos.offset(dx, dy))
            .filter(move |&n| self.can_step(pos, n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_map() -> GameMap {
        let mut map = generate::generate_map(5);
        for t in &mut map.tiles {
            *t = Tile::default();
        }
        map
    }

    #[test]
    fn obstacles_only_cover_the_side_they_are_on() {
        let mut map = flat_map();
        let me = TilePos::new(20, 20);
        map.tile_mut(me.offset(0, 1)).feature = Feature::Rock; // rock to the north
        assert_eq!(map.cover_against(me, TilePos::new(20, 30)), CoverLevel::Heavy); // shot from N
        assert_eq!(map.cover_against(me, TilePos::new(23, 30)), CoverLevel::Heavy); // from NNE
        assert_eq!(map.cover_against(me, TilePos::new(26, 26)), CoverLevel::None); // from NE: exposed
        assert_eq!(map.cover_against(me, TilePos::new(30, 20)), CoverLevel::None); // from E
        assert_eq!(map.cover_against(me, TilePos::new(20, 10)), CoverLevel::None); // from S
        assert_eq!(map.cover_at(me), CoverLevel::Heavy);
        assert_eq!(map.cover_facings(me), vec!["N"]);
    }

    #[test]
    fn sandbags_in_front_give_light_cover_and_trees_cover_all_round() {
        let mut map = flat_map();
        let me = TilePos::new(20, 20);
        map.tile_mut(me.offset(1, 0)).feature = Feature::LightCover;
        assert_eq!(map.cover_against(me, TilePos::new(30, 21)), CoverLevel::Light);
        assert_eq!(map.cover_against(me, TilePos::new(10, 20)), CoverLevel::None);
        map.tile_mut(me).feature = Feature::Trees;
        assert_eq!(map.cover_against(me, TilePos::new(10, 20)), CoverLevel::Light);
    }
}
