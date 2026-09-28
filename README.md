# Termbots vs Humans

A real-time 2D strategy commander game built with Rust and [Bevy](https://bevy.org) 0.19.

This covers **steps 1–4** of the build order in the design doc:
- a procedurally generated mirrored map with fog of war
- platoons of 1–5 fighters with rolled traits
- missions and signals with confusion
- autonomous fighters that move, shoot, take cover and flee on their own
- Supply with escalating prices
- a hidden setup phase for placing your army
- reinforcements on foot or by vehicle (the vehicle parks as cover)
- territory control and a 15-minute timer

The Termbots follow a fixed placeholder script until the real Termbot AI arrives in step 7. They place the default army, open with two assaults and an overwatch, and buy a reinforcement every 75 seconds.

## How a match goes

1. **Setup.** Build a platoon in the army panel (right): add 1–5 fighters, then click anywhere in your half to place it. Repeat until your 1000 Supply is spent, or click *Place default army*. *Undo last placement* refunds the last platoon. Press **START BATTLE** when ready. The enemy can't see your placements.
2. **Battle (15:00).** Command platoons with missions and signals. Supply grows at 3/s: build a platoon, choose **On foot** (they walk from base to the target) or **Vehicle** (+150: a transport drives them there and then parks as a solid obstacle that gives cover), and click an explored tile.
3. **End.** Kill the enemy King to win. If time runs out, whoever holds more sectors wins. A sector changes hands when only one side has fighters in it. Sector names are tinted by owner.

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
| Army panel buttons, then left-click the map | Place (setup) or deploy (battle) the platoon you built. Right-click cancels |
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
- **Platoons.** 1–5 fighters of any classes, with call signs Alpha, Bravo, and so on. Fighters roll Accuracy, IQ, Tactics, Bravery and Leadership within their class's range. The best leader leads, and the next one steps up if they fall.
- **Economy.** 1000 starting Supply, +3/s for both sides. Base prices: Assaulter 50, Heavy Weapons 80, Sniper 90, Juggernaut 120, and +150 to deliver by vehicle. Each living fighter of a class raises that class's price by 25%.
- **Vehicles.** A vehicle drives at 6 tiles/s, has armor and can be shot on the way. On arrival it unloads its platoon in Overwatch and parks as a solid, cover-giving obstacle. If it's destroyed on the way, the passengers bail out hurt and shaken, and the wreck becomes the obstacle.
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
    platoon.rs       Platoons, missions, signals, confusion, spawning
    economy.rs       Supply, prices, setup phase, deployments, territory, timer
    vehicle.rs       Troop vehicles: drive, unload, park as obstacles
    ai.rs            Fighter autonomy (utility AI)
    combat.rs        Hit chance, shooting, morale, deaths
    perception.rs    Unit vision and line of sight
  fog.rs             Per-tile commander fog + overlay
  pathfinding.rs     A* on the tile grid
  commander.rs       Default army; placeholder Termbot script
  army_panel.rs      Platoon builder: setup placement and reinforcements
  units.rs           Unit visuals: sprites, status icons, health bars, tracers
  input.rs           Selection, mission buttons, signals -> commands; pause
  camera.rs          Pan / zoom
  hud.rs             HUD, hover tooltip, victory/defeat banner
```

Game logic (map generation, pathfinding, line of sight) has no rendering code and is covered by unit tests.

## Tests

`cargo test` also runs headless matches. They check that setup places both armies, that a vehicle delivers its platoon and parks as an obstacle, that the timer ends the match on territory, and that the same seed replays a two-minute battle identically. For a balance summary across a few seeds, run `cargo test battle_report -- --ignored --nocapture`.

## Next: step 5

Engineers, with mines, bombs, turrets and artillery emplacements, and traps that stay hidden until a fighter detects them.
