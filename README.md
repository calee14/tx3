# Termbots vs Humans

A real-time 2D strategy commander game built with Rust and [Bevy](https://bevy.org) 0.19.

This is **step 1** of the build order in the design doc: a procedurally generated mirrored map, a top-down camera, per-sector fog of war, and the fixed-timestep simulation with a command queue. Each side has a King, and you can move yours to scout and reveal sectors.

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
| Left-click | Select your King |
| Right-click | Order the selected King to move |
| WASD / arrow keys / middle-mouse drag | Pan the camera |
| Mouse wheel | Zoom |
| Space | Pause / resume (single-player only). Orders given while paused apply on resume |
| F | Debug: toggle fog of war off to see the whole map |
| N | Debug: generate a new map (next seed) |

## What's in step 1

- **Mirrored procedural maps.** 128×128 tiles, symmetric under a 180° rotation so both sides get identical terrain. Three elevation levels with ledges and ramps, forests, and light and heavy cover placed near sector borders. Every map is validated: every tile is reachable, and no high ground overlooks a spawn. A seed that fails validation is rerolled.
- **12 sectors** (A1–C4). Rows 1–2 are the Humans' half and rows 3–4 are the Termbots'.
- **Two layers of knowledge.** Each unit has its own vision: radius-based, extended by high ground, blocked by higher ledges and forest depth. The commander's fog is per sector. A sector is revealed permanently once any of your units sees into it. Enemy units show up only in revealed sectors. Opening reveal: your own half plus the enemy's center-forward sector (B3 for Humans).
- **Simulation architecture built for multiplayer.** Game logic runs in `FixedUpdate` at 20 ticks per second. Every change goes through a `CommandQueue` of `SimCommand`s, addressed by stable `UnitId`s. Randomness comes from a seeded `SimRng`. Rendering interpolates between ticks, so tile-to-tile movement looks smooth.

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
    perception.rs    Unit vision and line of sight
  fog.rs             Per-sector commander fog + overlay
  pathfinding.rs     A* on the tile grid
  units.rs           Unit spawning (placeholder sprites)
  input.rs           Selection and orders -> commands; pause; debug keys
  camera.rs          Pan / zoom
  hud.rs             Debug HUD
```

Game logic (map generation, pathfinding, line of sight) has no rendering code and is covered by unit tests.

## Next: step 2

One platoon of Assaulters with rolled traits (Accuracy, IQ, Tactics, Bravery, Leadership), perception, and autonomous missions (Assault, Overwatch, Reposition) driven by utility AI.
