//! Procedural map generation.
//!
//! Every map is symmetric under a 180-degree rotation, so both sides get
//! exactly the same terrain. Noise fields are symmetrized directly, the
//! smoothing passes are rotation-symmetric, and every randomly placed
//! object (ramps, cover) is placed on the Humans' half and then copied
//! to its mirrored tile.
//!
//! Pipeline:
//! 1. Elevation from noise, snapped into 3 terraces and smoothed.
//! 2. Bases flattened; no high ground near a spawn.
//! 3. Ramps scattered along ledges.
//! 4. Connectivity fix: extra ramps until every tile is reachable.
//! 5. Forests from noise + cellular automata.
//! 6. Obstacles: ruined buildings, rock outcrops, barrier walls, boulders
//!    and sandbags. Solid ones are only kept if the map stays connected.
//! 7. Validation. If a check fails, reroll with the next seed.

use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::noise::ValueNoise;
use super::*;

/// Humans' King starts here; the Termbots' spawn is the mirrored tile.
pub const HUMAN_SPAWN: TilePos = TilePos::new(63, 7);
const BASE_FLAT_RADIUS: f32 = 10.0;
const BASE_NO_HIGH_GROUND_RADIUS: f32 = 18.0;
const MAX_ATTEMPTS: u64 = 64;

const TILE_COUNT: usize = (MAP_W * MAP_H) as usize;

#[derive(Debug, PartialEq, Eq)]
pub enum GenError {
    NotSymmetric(TilePos),
    Unreachable(TilePos),
    SpawnOverlooked(Faction),
    SpawnBlocked(Faction),
}

/// Generate a map from a seed. If the seed produces a map that fails
/// validation, the next seeds are tried; `GameMap::seed` records the seed
/// that was actually used.
pub fn generate_map(seed: u64) -> GameMap {
    for attempt in 0..MAX_ATTEMPTS {
        let s = seed.wrapping_add(attempt);
        match try_generate(s) {
            Ok(map) => return map,
            Err(err) => bevy::log::warn!("map seed {s} failed validation ({err:?}), rerolling"),
        }
    }
    panic!("could not generate a valid map after {MAX_ATTEMPTS} attempts from seed {seed}");
}

pub fn try_generate(seed: u64) -> Result<GameMap, GenError> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let spawns = [HUMAN_SPAWN, HUMAN_SPAWN.mirrored()];
    let mut map = GameMap {
        seed,
        tiles: vec![Tile::default(); TILE_COUNT],
        sectors: build_sectors(),
        spawns,
    };

    generate_elevation(&mut map, &mut rng);
    place_ramps(&mut map, &mut rng);
    fix_connectivity(&mut map)?;
    place_trees(&mut map, &mut rng);
    place_obstacles(&mut map, &mut rng);
    validate(&map)?;
    Ok(map)
}

fn all_tiles() -> impl Iterator<Item = TilePos> {
    (0..MAP_H).flat_map(|y| (0..MAP_W).map(move |x| TilePos::new(x, y)))
}

fn idx(pos: TilePos) -> usize {
    (pos.y * MAP_W + pos.x) as usize
}

/// Sample `f` at every tile and average each tile with its mirror, which
/// makes the field exactly symmetric and still smooth across the center.
fn symmetric_field(f: impl Fn(i32, i32) -> f32) -> Vec<f32> {
    let raw: Vec<f32> = all_tiles().map(|p| f(p.x, p.y)).collect();
    all_tiles()
        .map(|p| (raw[idx(p)] + raw[idx(p.mirrored())]) / 2.0)
        .collect()
}

fn quantile(values: &[f32], q: f32) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    sorted[((sorted.len() - 1) as f32 * q) as usize]
}

fn neighbors8(pos: TilePos) -> impl Iterator<Item = TilePos> {
    (-1..=1)
        .flat_map(move |dy| (-1..=1).map(move |dx| (dx, dy)))
        .filter(|&(dx, dy)| dx != 0 || dy != 0)
        .map(move |(dx, dy)| pos.offset(dx, dy))
        .filter(|p| p.in_bounds())
}

fn neighbors4(pos: TilePos) -> impl Iterator<Item = TilePos> {
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .map(move |(dx, dy)| pos.offset(dx, dy))
        .filter(|p| p.in_bounds())
}

fn near_spawn(map: &GameMap, pos: TilePos, radius: f32) -> bool {
    map.spawns.iter().any(|&s| s.distance(pos) <= radius)
}

fn generate_elevation(map: &mut GameMap, rng: &mut ChaCha8Rng) {
    let noise = ValueNoise::new(rng.random());
    let field = symmetric_field(|x, y| noise.fbm(x as f32 / 22.0, y as f32 / 22.0, 3));

    // Thresholds by quantile so every map has a similar terrain mix:
    // roughly 52% lowland, 32% hills, 16% high ground.
    let hills = quantile(&field, 0.52);
    let high = quantile(&field, 0.84);
    let mut elev: Vec<u8> = field
        .iter()
        .map(|&v| if v >= high { 2 } else if v >= hills { 1 } else { 0 })
        .collect();

    // Majority smoothing removes single-tile specks. Reads from a copy so
    // the result doesn't depend on scan order (keeps the map symmetric).
    for _ in 0..2 {
        let old = elev.clone();
        for p in all_tiles() {
            let mut counts = [0u8; (MAX_ELEVATION + 1) as usize];
            counts[old[idx(p)] as usize] += 1;
            for n in neighbors8(p) {
                counts[old[idx(n)] as usize] += 1;
            }
            let best = *counts.iter().max().unwrap();
            let current = old[idx(p)];
            if counts[current as usize] < best {
                elev[idx(p)] = counts.iter().position(|&c| c == best).unwrap() as u8;
            }
        }
    }

    // No sheer 2-level cliffs: high ground next to lowland drops to hills,
    // so terraces step down one level at a time.
    let old = elev.clone();
    for p in all_tiles() {
        if old[idx(p)] == 2 && neighbors8(p).any(|n| old[idx(n)] == 0) {
            elev[idx(p)] = 1;
        }
    }

    // Flat bases, and no high ground overlooking a spawn.
    for p in all_tiles() {
        if near_spawn(map, p, BASE_FLAT_RADIUS) {
            elev[idx(p)] = 0;
        } else if near_spawn(map, p, BASE_NO_HIGH_GROUND_RADIUS) {
            elev[idx(p)] = elev[idx(p)].min(1);
        }
    }

    for p in all_tiles() {
        map.tile_mut(p).elevation = elev[idx(p)];
    }
}

/// A tile on the Humans' half of the map (the half that random placement
/// happens on before mirroring).
fn in_generated_half(pos: TilePos) -> bool {
    pos.y < MAP_H / 2
}

fn is_ledge_top(map: &GameMap, pos: TilePos) -> bool {
    let e = map.tile(pos).elevation;
    e > 0 && neighbors4(pos).any(|n| map.tile(n).elevation + 1 == e)
}

fn set_ramp(map: &mut GameMap, pos: TilePos) {
    map.tile_mut(pos).ramp = true;
    map.tile_mut(pos.mirrored()).ramp = true;
}

fn place_ramps(map: &mut GameMap, rng: &mut ChaCha8Rng) {
    const MIN_SPACING: i32 = 6;
    let mut candidates: Vec<TilePos> = all_tiles()
        .filter(|&p| in_generated_half(p) && is_ledge_top(map, p))
        .collect();
    candidates.shuffle(rng);

    let mut ramps: Vec<TilePos> = Vec::new();
    for p in candidates {
        let too_close = ramps.iter().any(|r| (r.x - p.x).abs().max((r.y - p.y).abs()) < MIN_SPACING);
        if too_close || !rng.random_bool(0.5) {
            continue;
        }
        set_ramp(map, p);
        ramps.push(p);
        ramps.push(p.mirrored());
    }
}

/// Flood fill of every tile reachable from `start`.
pub fn reachable_from(map: &GameMap, start: TilePos) -> Vec<bool> {
    let mut seen = vec![false; TILE_COUNT];
    let mut stack = vec![start];
    seen[idx(start)] = true;
    while let Some(p) = stack.pop() {
        for n in map.neighbors(p) {
            if !seen[idx(n)] {
                seen[idx(n)] = true;
                stack.push(n);
            }
        }
    }
    seen
}

/// Add ramps until every tile can be reached from the Humans' spawn.
fn fix_connectivity(map: &mut GameMap) -> Result<(), GenError> {
    const MAX_FIXES: usize = 400;
    for _ in 0..MAX_FIXES {
        let reach = reachable_from(map, map.spawns[0]);
        let Some(unreached) = all_tiles().find(|&p| !reach[idx(p)]) else {
            return Ok(());
        };
        // Find an unreached tile next to a reached one, one level apart,
        // and turn the higher of the two into a ramp.
        let fix = all_tiles().filter(|&p| !reach[idx(p)]).find_map(|p| {
            let e = map.tile(p).elevation;
            neighbors4(p)
                .find(|&n| reach[idx(n)] && map.tile(n).elevation.abs_diff(e) == 1)
                .map(|n| if map.tile(n).elevation > e { n } else { p })
        });
        match fix {
            Some(ramp) => set_ramp(map, ramp),
            None => return Err(GenError::Unreachable(unreached)),
        }
    }
    let reach = reachable_from(map, map.spawns[0]);
    match all_tiles().find(|&p| !reach[idx(p)]) {
        Some(p) => Err(GenError::Unreachable(p)),
        None => Ok(()),
    }
}

fn place_trees(map: &mut GameMap, rng: &mut ChaCha8Rng) {
    let noise = ValueNoise::new(rng.random());
    let field = symmetric_field(|x, y| noise.fbm(x as f32 / 9.0, y as f32 / 9.0, 2));
    let threshold = quantile(&field, 0.72);
    let mut forest: Vec<bool> = field.iter().map(|&v| v >= threshold).collect();

    // Cellular automata: clumps grow solid, stragglers disappear.
    for _ in 0..3 {
        let old = forest.clone();
        for p in all_tiles() {
            let n = neighbors8(p).filter(|&n| old[idx(n)]).count();
            forest[idx(p)] = if old[idx(p)] { n >= 4 } else { n >= 5 };
        }
    }

    for p in all_tiles() {
        let tile = map.tile(p);
        if forest[idx(p)] && !tile.ramp && !near_spawn(map, p, BASE_FLAT_RADIUS - 1.0) {
            map.tile_mut(p).feature = Feature::Trees;
        }
    }
}

/// Is every walkable tile reachable from the Humans' spawn?
fn fully_connected(map: &GameMap) -> bool {
    let reach = reachable_from(map, map.spawns[0]);
    all_tiles().all(|p| reach[idx(p)] || !map.tile(p).passable())
}

/// Can an obstacle go on this tile? Keeps bases, ramps and the tiles next
/// to ramps clear, and only uses the Humans' half (it's mirrored after).
fn obstacle_ok(map: &GameMap, p: TilePos) -> bool {
    p.in_bounds()
        && in_generated_half(p)
        && !near_spawn(map, p, BASE_FLAT_RADIUS + 1.0)
        && !map.tile(p).ramp
        && neighbors8(p).all(|n| !map.tile(n).ramp)
}

/// Apply features to a set of tiles and their mirrors, then keep the
/// change only if the map is still fully connected.
fn try_place(map: &mut GameMap, changes: &[(TilePos, Feature)]) -> bool {
    if changes.is_empty() {
        return false;
    }
    let backup: Vec<(TilePos, Tile)> = changes
        .iter()
        .flat_map(|&(p, _)| [(p, map.tile(p)), (p.mirrored(), map.tile(p.mirrored()))])
        .collect();
    for &(p, feature) in changes {
        map.tile_mut(p).feature = feature;
        map.tile_mut(p.mirrored()).feature = feature;
    }
    let blocks_any = changes.iter().any(|(_, f)| f.is_solid());
    if blocks_any && !fully_connected(map) {
        for (p, tile) in backup.into_iter().rev() {
            *map.tile_mut(p) = tile;
        }
        return false;
    }
    true
}

fn random_tile_in_half(rng: &mut ChaCha8Rng) -> TilePos {
    TilePos::new(rng.random_range(0..MAP_W), rng.random_range(0..MAP_H / 2))
}

/// A random tile near a sector border (where fighting lines form), on the
/// Humans' half.
fn random_tile_near_border(rng: &mut ChaCha8Rng) -> TilePos {
    let jitter = rng.random_range(-4..=4);
    if rng.random_bool(0.5) {
        let x = [SECTOR_COL_EDGES[1], SECTOR_COL_EDGES[2]][rng.random_range(0..2)] + jitter;
        TilePos::new(x, rng.random_range(0..MAP_H / 2))
    } else {
        let y = [SECTOR_ROW_EDGES[1], SECTOR_ROW_EDGES[2] - 5][rng.random_range(0..2)] + jitter;
        TilePos::new(rng.random_range(0..MAP_W), y)
    }
}

/// Large solid obstacles that fighters maneuver around and take cover
/// behind, plus walkable sandbag lines. Counts are per half of the map.
fn place_obstacles(map: &mut GameMap, rng: &mut ChaCha8Rng) {
    place_repeatedly(map, rng, 7, place_ruin);
    place_repeatedly(map, rng, 12, place_rock_outcrop);
    place_repeatedly(map, rng, 12, place_barrier);
    place_repeatedly(map, rng, 16, place_boulder);
    place_repeatedly(map, rng, 24, place_sandbags);
}

fn place_repeatedly(
    map: &mut GameMap,
    rng: &mut ChaCha8Rng,
    target: usize,
    place: fn(&mut GameMap, &mut ChaCha8Rng) -> bool,
) {
    let mut placed = 0;
    for _ in 0..target * 30 {
        if placed >= target {
            break;
        }
        if place(map, rng) {
            placed += 1;
        }
    }
}

/// A ruined building: rectangular walls with two doorways and some
/// collapsed sections, rubble inside. Great cover for a platoon.
fn place_ruin(map: &mut GameMap, rng: &mut ChaCha8Rng) -> bool {
    let w = rng.random_range(6..=10);
    let h = rng.random_range(5..=8);
    let origin = random_tile_in_half(rng);
    let footprint: Vec<TilePos> = (0..h).flat_map(|y| (0..w).map(move |x| origin.offset(x, y))).collect();
    if !footprint.iter().all(|&p| obstacle_ok(map, p)) {
        return false;
    }
    let elevation = map.tile(origin).elevation;
    if footprint.iter().any(|&p| map.tile(p).elevation != elevation) {
        return false;
    }

    // Two doorways (2 tiles wide) on two different sides.
    let mut sides = [0, 1, 2, 3];
    sides.shuffle(rng);
    let mut doors: Vec<TilePos> = Vec::new();
    for &side in &sides[..2] {
        let (len, at): (i32, Box<dyn Fn(i32) -> TilePos>) = match side {
            0 => (w, Box::new(move |i| origin.offset(i, 0))),
            1 => (w, Box::new(move |i| origin.offset(i, h - 1))),
            2 => (h, Box::new(move |i| origin.offset(0, i))),
            _ => (h, Box::new(move |i| origin.offset(w - 1, i))),
        };
        let i = rng.random_range(1..len - 2);
        doors.push(at(i));
        doors.push(at(i + 1));
    }

    let mut changes = Vec::new();
    for &p in &footprint {
        let edge = p.x == origin.x || p.y == origin.y || p.x == origin.x + w - 1 || p.y == origin.y + h - 1;
        let feature = if edge {
            if doors.contains(&p) || rng.random_bool(0.12) { Feature::None } else { Feature::Wall }
        } else if rng.random_bool(0.1) {
            Feature::LightCover // rubble
        } else {
            Feature::None
        };
        changes.push((p, feature));
    }
    try_place(map, &changes)
}

/// A rough blob of rock, 3 to 10 tiles across.
fn place_rock_outcrop(map: &mut GameMap, rng: &mut ChaCha8Rng) -> bool {
    let center = random_tile_in_half(rng);
    let radius: f32 = rng.random_range(1.8..4.8);
    let r = radius.ceil() as i32 + 1;
    let mut changes = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let p = center.offset(dx, dy);
            let jitter: f32 = rng.random_range(-0.7..0.7);
            if center.distance(p) <= radius + jitter && obstacle_ok(map, p) {
                changes.push((p, Feature::Rock));
            }
        }
    }
    changes.len() >= 4 && try_place(map, &changes)
}

/// A single 1-2 tile boulder: scattered cover in open ground.
fn place_boulder(map: &mut GameMap, rng: &mut ChaCha8Rng) -> bool {
    let p = random_tile_in_half(rng);
    let mut changes = vec![(p, Feature::Rock)];
    if rng.random_bool(0.5) {
        changes.push((p.offset(1, 0), Feature::Rock));
    }
    changes.iter().all(|&(t, _)| obstacle_ok(map, t) && map.tile(t).feature == Feature::None)
        && try_place(map, &changes)
}

/// A straight concrete barrier wall, biased toward sector borders.
fn place_barrier(map: &mut GameMap, rng: &mut ChaCha8Rng) -> bool {
    let start = random_tile_near_border(rng);
    let len = rng.random_range(3..=6);
    let horizontal = rng.random_bool(0.5);
    let changes: Vec<(TilePos, Feature)> = (0..len)
        .map(|i| (if horizontal { start.offset(i, 0) } else { start.offset(0, i) }, Feature::Wall))
        .collect();
    let fits = changes.iter().all(|&(p, _)| {
        obstacle_ok(map, p)
            && map.tile(p).feature == Feature::None
            && map.tile(p).elevation == map.tile(start).elevation
    });
    fits && try_place(map, &changes)
}

/// A short walkable line of sandbags (light cover).
fn place_sandbags(map: &mut GameMap, rng: &mut ChaCha8Rng) -> bool {
    let start = if rng.random_bool(0.5) { random_tile_near_border(rng) } else { random_tile_in_half(rng) };
    let len = rng.random_range(2..=4);
    let horizontal = rng.random_bool(0.5);
    let changes: Vec<(TilePos, Feature)> = (0..len)
        .map(|i| (if horizontal { start.offset(i, 0) } else { start.offset(0, i) }, Feature::LightCover))
        .collect();
    let fits = changes.iter().all(|&(p, _)| {
        p.in_bounds()
            && in_generated_half(p)
            && !near_spawn(map, p, 8.0)
            && map.tile(p).feature == Feature::None
            && !map.tile(p).ramp
    });
    fits && try_place(map, &changes)
}

pub fn validate(map: &GameMap) -> Result<(), GenError> {
    for p in all_tiles() {
        if map.tile(p) != map.tile(p.mirrored()) {
            return Err(GenError::NotSymmetric(p));
        }
    }

    let reach = reachable_from(map, map.spawns[0]);
    if let Some(p) = all_tiles().find(|&p| !reach[idx(p)] && map.tile(p).passable()) {
        return Err(GenError::Unreachable(p));
    }

    for faction in Faction::ALL {
        let spawn = map.spawn(faction);
        let tile = map.tile(spawn);
        if tile.feature != Feature::None || tile.elevation != 0 {
            return Err(GenError::SpawnBlocked(faction));
        }
        let overlooked = all_tiles()
            .any(|p| p.distance(spawn) <= 16.0 && map.tile(p).elevation == MAX_ELEVATION);
        if overlooked {
            return Err(GenError::SpawnOverlooked(faction));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_are_valid_for_many_seeds() {
        for seed in 0..40 {
            let map = generate_map(seed);
            assert_eq!(validate(&map), Ok(()), "seed {seed}");
        }
    }

    #[test]
    fn same_seed_same_map() {
        let a = generate_map(1234);
        let b = generate_map(1234);
        assert_eq!(a.seed, b.seed);
        assert!(a.tiles == b.tiles);
    }

    #[test]
    fn different_seeds_differ() {
        let a = generate_map(1);
        let b = generate_map(2);
        assert!(a.tiles != b.tiles);
    }

    #[test]
    fn maps_have_a_terrain_mix() {
        let map = generate_map(7);
        let count = |f: &dyn Fn(&Tile) -> bool| map.tiles.iter().filter(|t| f(t)).count();
        assert!(count(&|t| t.elevation == 2) > 300, "needs high ground");
        assert!(count(&|t| t.ramp) > 10, "needs ramps");
        assert!(count(&|t| t.feature == Feature::Trees) > 500, "needs forests");
        assert!(count(&|t| t.feature == Feature::Rock) > 150, "needs rock outcrops");
        assert!(count(&|t| t.feature == Feature::Wall) > 150, "needs ruins and barriers");
        assert!(count(&|t| t.feature == Feature::LightCover) > 60, "needs sandbags");
    }

    #[test]
    fn sectors_mirror_each_other() {
        for p in all_tiles() {
            assert_eq!(sector_index(p.mirrored()), mirrored_sector(sector_index(p)));
        }
    }
}
