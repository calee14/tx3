//! The AI commander's army planning: its personality, the hidden army it
//! places during setup, and which reinforcements it buys.

use std::collections::HashSet;

use rand::Rng;

use crate::faction::Faction;
use crate::map::{CoverLevel, GameMap, MAP_H, MAP_W, TilePos};
use crate::sim::classes::FighterClass::{self, *};
use crate::sim::economy::{ClassCounts, STARTING_SUPPLY, platoon_cost};
use crate::sim::platoon::nearest_free_tile_where;

/// The enemy's opening view reaches about this many rows into our half;
/// setup placements deeper than this start hidden.
pub const ENEMY_PEEK_DEPTH: i32 = 7;

/// The commander's personality, picked at random each match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Juggernaut-heavy army placed forward; pushes early.
    Aggressive,
    /// Mixed army; pushes when it has the edge.
    Balanced,
    /// Snipers and heavy weapons placed deep; waits for mistakes.
    Defensive,
}

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Style::Aggressive => "aggressive",
            Style::Balanced => "balanced",
            Style::Defensive => "defensive",
        }
    }

    /// Chance of picking a bolder, less predictable counter.
    pub fn unpredictability(self) -> f32 {
        match self {
            Style::Aggressive => 0.25,
            Style::Balanced => 0.15,
            Style::Defensive => 0.1,
        }
    }

    /// How far behind the middle line it likes to set up (rows).
    fn preferred_depth(self) -> f32 {
        match self {
            Style::Aggressive => 10.0,
            Style::Balanced => 14.0,
            Style::Defensive => 19.0,
        }
    }

    fn army(self) -> Vec<Vec<FighterClass>> {
        match self {
            Style::Aggressive => {
                vec![vec![Juggernaut, Assaulter, Assaulter, Assaulter], vec![Juggernaut, Assaulter, Assaulter], vec![
                    Assaulter,
                    Assaulter,
                    HeavyWeapons,
                ]]
            }
            Style::Balanced => vec![
                vec![Assaulter, Assaulter, Assaulter, HeavyWeapons],
                vec![Sniper, Sniper, Assaulter],
                vec![Juggernaut, Assaulter, Assaulter, HeavyWeapons],
            ],
            Style::Defensive => vec![
                vec![Sniper, HeavyWeapons, Assaulter],
                vec![Sniper, HeavyWeapons, Assaulter],
                vec![Assaulter, Assaulter, Assaulter, HeavyWeapons],
                vec![Assaulter],
            ],
        }
    }
}

pub fn choose_style(rng: &mut impl Rng) -> Style {
    match rng.random_range(0..3) {
        0 => Style::Aggressive,
        1 => Style::Balanced,
        _ => Style::Defensive,
    }
}

/// How many rows into its own half a tile is (0 at the middle line).
pub fn depth(faction: Faction, pos: TilePos) -> i32 {
    match faction {
        Faction::Humans => MAP_H / 2 - 1 - pos.y,
        Faction::Termbots => pos.y - MAP_H / 2,
    }
}

fn anchor_score(map: &GameMap, pos: TilePos, preferred_depth: f32, faction: Faction) -> f32 {
    let mut cover = 0.0;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let p = pos.offset(dx, dy);
            if p.in_bounds() && map.tile(p).passable() {
                cover += match map.cover_at(p) {
                    CoverLevel::Heavy => 0.5,
                    CoverLevel::Light => 0.2,
                    CoverLevel::None => 0.0,
                };
            }
        }
    }
    cover + map.tile(pos).elevation as f32 * 1.2 - (depth(faction, pos) as f32 - preferred_depth).abs() * 0.12
}

/// The hidden army: which platoons to buy with the starting Supply and
/// where to put them. Spots are chosen for cover and high ground, spread
/// across the front, and deep enough to start out of the enemy's view.
pub fn plan_setup(map: &GameMap, faction: Faction, style: Style, rng: &mut impl Rng) -> Vec<(Vec<FighterClass>, TilePos)> {
    let mut budget = STARTING_SUPPLY;
    let mut counts = ClassCounts::default();
    let mut plan = Vec::new();
    let king = map.spawn(faction);
    let mut chosen: Vec<TilePos> = Vec::new();

    for mut classes in style.army() {
        while !classes.is_empty() && platoon_cost(&classes, &counts, false) > budget {
            classes.pop();
        }
        if classes.is_empty() {
            continue;
        }
        let cost = platoon_cost(&classes, &counts, false);

        // Best spot on a coarse grid, with a little randomness so the
        // army isn't in the same places every match.
        let mut best: Option<(f32, TilePos)> = None;
        for y in (0..MAP_H).step_by(3) {
            for x in (2..MAP_W - 2).step_by(3) {
                let p = TilePos::new(x, y);
                let d = depth(faction, p);
                if !map.tile(p).passable() || d <= ENEMY_PEEK_DEPTH || d > 40 {
                    continue;
                }
                if p.distance(king) < 8.0 || chosen.iter().any(|c| c.distance(p) < 14.0) {
                    continue;
                }
                let score = anchor_score(map, p, style.preferred_depth(), faction) + rng.random_range(0.0..0.8);
                if best.is_none_or(|(s, _)| score > s) {
                    best = Some((score, p));
                }
            }
        }
        let Some((_, anchor)) = best else { break };
        chosen.push(anchor);
        budget -= cost;
        for &c in &classes {
            counts.add(c);
        }
        plan.push((classes, anchor));
    }
    plan
}

/// Enemy fighters seen right now, by class.
#[derive(Clone, Copy, Debug, Default)]
pub struct KnownEnemy {
    pub assaulters: u32,
    pub snipers: u32,
    pub heavies: u32,
    pub juggernauts: u32,
}

/// What to buy next: counter what the enemy has shown, else follow style.
/// Bigger platoons when Supply is plentiful.
pub fn reinforcement(style: Style, known: &KnownEnemy, supply: f32) -> Vec<FighterClass> {
    let mut classes = if known.snipers >= 2 {
        // Close the distance fast against snipers.
        vec![Juggernaut, Assaulter, Assaulter]
    } else if known.heavies + known.juggernauts >= 3 {
        // Pick heavy targets apart from range.
        vec![Sniper, HeavyWeapons, Assaulter]
    } else if known.assaulters >= 4 {
        // Suppress massed infantry.
        vec![HeavyWeapons, Assaulter, Assaulter]
    } else {
        match style {
            Style::Aggressive => vec![Juggernaut, Assaulter, Assaulter],
            Style::Balanced => vec![Assaulter, Assaulter, HeavyWeapons],
            Style::Defensive => vec![Sniper, Assaulter, HeavyWeapons],
        }
    };
    if supply > 700.0 {
        classes.push(Assaulter);
    }
    if supply > 1100.0 {
        classes.push(Assaulter);
    }
    classes
}

/// The nearest walkable tile to `around` that passes `ok` (e.g. explored).
pub fn nearest_ok(map: &GameMap, around: TilePos, ok: impl Fn(TilePos) -> bool) -> Option<TilePos> {
    let clamped = TilePos::new(around.x.clamp(0, MAP_W - 1), around.y.clamp(0, MAP_H - 1));
    nearest_free_tile_where(map, &HashSet::new(), clamped, ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::generate::generate_map;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn setup_stays_in_budget_hidden_and_spread_out() {
        for seed in [1u64, 42, 99] {
            let map = generate_map(seed);
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            for style in [Style::Aggressive, Style::Balanced, Style::Defensive] {
                let plan = plan_setup(&map, Faction::Termbots, style, &mut rng);
                assert!(plan.len() >= 3, "{style:?} placed {}", plan.len());
                let mut counts = ClassCounts::default();
                let mut spent = 0.0;
                for (classes, anchor) in &plan {
                    assert!(map.in_home_half(Faction::Termbots, *anchor));
                    assert!(depth(Faction::Termbots, *anchor) > ENEMY_PEEK_DEPTH, "starts hidden");
                    assert!((1..=5).contains(&classes.len()));
                    spent += platoon_cost(classes, &counts, false);
                    for &c in classes {
                        counts.add(c);
                    }
                }
                assert!(spent <= STARTING_SUPPLY);
                for (i, a) in plan.iter().enumerate() {
                    for b in &plan[i + 1..] {
                        assert!(a.1.distance(b.1) >= 14.0);
                    }
                }
            }
        }
    }

    #[test]
    fn reinforcements_counter_what_the_enemy_shows() {
        let snipers = KnownEnemy { snipers: 2, ..Default::default() };
        assert!(reinforcement(Style::Defensive, &snipers, 300.0).contains(&Juggernaut));
        let armor = KnownEnemy { juggernauts: 2, heavies: 1, ..Default::default() };
        assert!(reinforcement(Style::Aggressive, &armor, 300.0).contains(&Sniper));
        assert_eq!(reinforcement(Style::Balanced, &KnownEnemy::default(), 800.0).len(), 4);
    }
}
