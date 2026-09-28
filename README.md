# Termbots vs Humans

A real-time 2D strategy commander game built with Rust and [Bevy](https://bevy.org) 0.19.

This covers **steps 1–3** of the build order in the design doc:
- a procedurally generated mirrored map with fog of war
- mixed platoons of fighters with rolled traits
- missions and signals with confusion
- autonomous fighters that move, shoot, take cover and flee on their own

The Termbots follow a fixed placeholder opening until the real Termbot AI arrives in step 7: two platoons assault your line, and one moves up into overwatch.

## Running it

1. Install Rust with [rustup](https://rustup.rs), or update an existing install with `rustup update`. Bevy 0.19 needs a recent stable Rust (built and tested here with 1.95).
2. Platform setup:
   - **Windows:** rustup will prompt you to install the Visual Studio C++ Build Tools. That's all you need.
   - **macOS:** `xcode-select --install`.
   - **Linux (Debian/Ubuntu):** `sudo apt install g++ pkg-config libudev-dev libwayland-dev libxkbcommon-dev`
3. From this folder, run:

```sh
cargo run              # random map
cargo run -- 42        # a specific map seed
```

The **first build takes a while** (5–25 minutes depending on your CPU) because it compiles all of Bevy. Later builds only recompile the game and take seconds.

For faster rebuilds while developing, use Bevy's dynamic linking:

```sh
cargo run --features dev
```

Run the tests with `cargo test`.

## Controls

| Input | Action |
| --- | --- |
| Left-click a fighter | Select its platoon (or your King) |
| 1 / 2 / 3, or the buttons | Choose a mission: Assault / Overwatch / Reposition |
| Right-click | Signal the selected platoon: the chosen mission (or its current one) at that tile. Moves the King if it's selected |
| Hover a fighter | Details: your fighters show traits, morale, suppression and what they're doing; enemies show only what's visible |
| Hover a tile | The HUD shows its terrain and cover |
| WASD / arrow keys / middle-mouse drag | Pan the camera |
| Mouse wheel | Zoom |
| Space | Pause / resume (single-player only). Orders given while paused apply on resume |
| F | Debug: toggle fog of war off |
| N | New map (next seed) |

Every signal adds confusion to the platoon, and contradictory signals in quick succession add a lot. Confused platoons act on orders late, and very confused ones freeze.

**Overhead icons:** `RUN` broken · `??` frozen · `!!` pinned · `?` confused · `!` suppressed · `++`/`+` injured · `~` shaken · `^` brave. Gold diamond = platoon leader. Cyan streaks are sniper shots.

## What's in it

- **Mirrored procedural maps.** 128×128 tiles, symmetric under a 180° rotation so both sides get identical terrain. The terrain has three elevation levels with ledges and ramps, and forests. Obstacles include ruined buildings with doorways, rock outcrops, concrete barrier walls, boulders and sandbag lines. Rocks and walls are solid: they block movement and sight, and a unit standing next to one gets heavy cover. Sandbags and trees give light cover. Every map is validated: every walkable tile is reachable, and no high ground overlooks a spawn. A seed that fails validation is rerolled.
- **12 sectors** (A1–C4), used for territory control. Rows 1–2 are the Humans' half and rows 3–4 are the Termbots'.
- **Two layers of knowledge.** Each unit has its own vision: radius-based, extended by high ground, blocked by higher ledges, rocks, walls and forest depth. The commander's fog is per tile. Each side starts with a large circle around its King revealed, peeking a few tiles past the middle, and every tile a unit sees stays explored. Enemy units show up only on explored ground.
- **Platoons.** Three per side: Rifle (3 Assaulters, Heavy Weapons), Recon (2 Snipers, Assaulter), Breach (Juggernaut, 2 Assaulters, Heavy Weapons). Fighters roll Accuracy, IQ, Tactics, Bravery and Leadership within their class's range. The best leader leads, and the next one steps up if they fall.
- **Missions.** Assault bounds toward the target cover to cover; Overwatch holds in cover with reaction fire; Reposition sprints and dodges. The three form a rock-paper-scissors loop through hit-chance modifiers.
- **Fighter autonomy.** Each fighter re-thinks several times a second. Broken fighters flee, pinned fighters stay down, and frozen platoons stop following orders. Otherwise, fighters score taking cover, advancing, fighting or holding, from what they see and their state, tilted by their traits. They shoot on their own at whatever they see, with penalties for moving, suppression and injury.
- **Combat.** Health, armor, suppression, morale, deaths, and leader succession. Killing the enemy King wins.
- **Simulation architecture built for multiplayer.** Game logic runs in `FixedUpdate` at 20 ticks per second, headless-capable. Every change goes through a `CommandQueue` of `SimCommand`s, addressed by stable `UnitId`s. Randomness comes from a seeded `SimRng`. Rendering interpolates between ticks, so tile-to-tile movement looks smooth.

## Project layout

```
src/
  main.rs            App setup and plugins
  game.rs            Match setup / teardown (seed, spawning, resources)
  faction.rs         Humans / Termbots, local player
  map/
    mod.rs           Tiles, terrain rules (can_step, move cost), sectors
    generate.rs      Procedural mirrored map generation + validation
    noise.rs         Seeded value noise
    render.rs        Placeholder map texture, sector borders and labels
  sim/
    mod.rs           Fixed-tick simulation: command queue, movement, interpolation
    classes.rs       Fighter class stats and trait ranges
    fighter.rs       Health, condition (morale, suppression, decision), statuses
    platoon.rs       Platoons, missions, signals, confusion, spawning forces
    ai.rs            Fighter autonomy (utility AI)
    combat.rs        Hit chance, shooting, morale, deaths
    perception.rs    Unit vision and line of sight
  fog.rs             Per-sector commander fog + overlay
  pathfinding.rs     A* on the tile grid
  commander.rs       Placeholder Termbot opening orders
  units.rs           Unit visuals: sprites, status icons, health bars, tracers
  input.rs           Selection, mission buttons, signals -> commands; pause
  camera.rs          Pan / zoom
  hud.rs             HUD, hover tooltip, victory/defeat banner
```

Game logic (map generation, pathfinding, line of sight) has no rendering code and is covered by unit tests.

## Tests

`cargo test` also runs a two-minute headless battle and checks that the same seed replays identically. For a balance summary across a few seeds, run `cargo test battle_report -- --ignored --nocapture`.

## Next: step 4

Supply and escalating class costs, hidden setup (placing your army before the battle), territory control, and the match timer.
