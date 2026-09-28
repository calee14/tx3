//! A* pathfinding on the tile grid (8 directions, terrain-weighted).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use crate::map::{GameMap, TilePos};

const ORTHOGONAL: u32 = 10;
const DIAGONAL: u32 = 14;

fn heuristic(a: TilePos, b: TilePos) -> u32 {
    // Octile distance: exact on open ground, never overestimates.
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    ORTHOGONAL * dx.max(dy) + (DIAGONAL - ORTHOGONAL) * dx.min(dy)
}

/// Shortest path from `start` to `goal`, excluding `start` and including
/// `goal`. Returns `None` if the goal can't be reached.
pub fn find_path(map: &GameMap, start: TilePos, goal: TilePos) -> Option<VecDeque<TilePos>> {
    find_path_avoiding(map, start, goal, &HashSet::new())
}

/// Like `find_path`, but never steps on the `avoid` tiles (known traps),
/// unless one of them is the goal itself.
pub fn find_path_avoiding(
    map: &GameMap,
    start: TilePos,
    goal: TilePos,
    avoid: &HashSet<TilePos>,
) -> Option<VecDeque<TilePos>> {
    if !goal.in_bounds() || !start.in_bounds() {
        return None;
    }
    if start == goal {
        return Some(VecDeque::new());
    }

    let mut open = BinaryHeap::new();
    let mut came_from: HashMap<TilePos, TilePos> = HashMap::new();
    let mut cost: HashMap<TilePos, u32> = HashMap::new();
    cost.insert(start, 0);
    // Ties broken by position so results are deterministic.
    open.push(Reverse((heuristic(start, goal), start)));

    while let Some(Reverse((_, current))) = open.pop() {
        if current == goal {
            let mut path = VecDeque::new();
            let mut node = goal;
            while node != start {
                path.push_front(node);
                node = came_from[&node];
            }
            return Some(path);
        }
        let current_cost = cost[&current];
        for next in map.neighbors(current) {
            if next != goal && avoid.contains(&next) {
                continue;
            }
            let step = if next.x != current.x && next.y != current.y { DIAGONAL } else { ORTHOGONAL };
            let new_cost = current_cost + step * map.tile(next).move_cost();
            if cost.get(&next).is_none_or(|&c| new_cost < c) {
                cost.insert(next, new_cost);
                came_from.insert(next, current);
                open.push(Reverse((new_cost + heuristic(next, goal), next)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::generate::generate_map;

    #[test]
    fn paths_go_around_known_traps() {
        let mut map = generate_map(5);
        for t in &mut map.tiles {
            *t = crate::map::Tile::default();
        }
        let (a, b) = (TilePos::new(10, 10), TilePos::new(20, 10));
        let straight = find_path(&map, a, b).unwrap();
        let mine = straight[4];
        let avoid: HashSet<TilePos> = [mine].into();
        let around = find_path_avoiding(&map, a, b, &avoid).unwrap();
        assert!(!around.contains(&mine));
        assert_eq!(*around.back().unwrap(), b);
    }

    #[test]
    fn kings_can_path_to_each_other() {
        let map = generate_map(99);
        let path = find_path(&map, map.spawns[0], map.spawns[1]).expect("path between spawns");
        assert_eq!(*path.back().unwrap(), map.spawns[1]);
        // Every step must be a legal move.
        let mut prev = map.spawns[0];
        for &step in &path {
            assert!(map.can_step(prev, step), "illegal step {prev:?} -> {step:?}");
            prev = step;
        }
    }
}
