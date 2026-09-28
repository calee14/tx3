# Termbots vs Humans

A real-time 2D strategy commander game built with Rust and [Bevy](https://bevy.org) 0.19.

This covers **steps 1–7** of the build order in the design doc (everything except networking):
- a procedurally generated random (but balanced) map with fog of war
- platoons of 1–5 fighters with rolled traits
- missions and signals with confusion
- autonomous fighters that move, shoot, take cover and flee on their own
- Supply with escalating prices
- a hidden setup phase for placing your army
- reinforcements on foot or by vehicle (the vehicle parks as cover)
- territory control and a 15-minute timer
- engineers, hidden mines and bombs, turrets, artillery and air strikes
- hidden loadouts, damage and armor types, destructible terrain and veterancy
- an AI Termbot commander that reads your moves and counters them live

## The Termbot commander

The Termbots are commanded by an AI that plays by your rules. It only knows about your units on ground it has explored. It never sees your missions, and it reads your intent from how your units move and whether they're shooting. It commands through the same signals, so its platoons get confused if it over-orders them.

- **Personality:** each match it picks a style: aggressive, balanced or defensive. The style decides its army, where it sets up (hidden from your opening view), and how readily it attacks.
- **Live counters:** every 2 seconds it groups your visible units and reads what they're doing:
  - Your units advance: it digs in (Overwatch).
  - Your units rush: it assaults where they're heading, to catch them in the open.
  - Your units sit and fire: it flanks out of your firing lane, or slips away if it's weaker.
  - Now and then it picks a bolder option, so it isn't perfectly predictable.
- **Posture:** it scouts early, holds and probes your line, pushes your weakest sector when it has the edge or is behind on territory late, runs down your King when clearly winning, and rushes back to defend its own King.
- **Spending:** it buys reinforcements that counter what it has seen, sends them behind its front (by vehicle when far away or urgent), and regroups when losing badly.
- **Gadgets:** it hides traps (and, if it's careful, a turret) during setup, calls air strikes on big, slow groups of yours away from its own troops, and has its engineers build turrets and bombs now and then. It only knows about your gadgets once its fighters have found them, and brings engineers when it runs into your minefields.

Press **F** for the debug view: the whole map, plus the commander's recent decisions and reasoning.

## How a match goes

1. **Setup.** Build a platoon in the army panel (right): add 1–5 fighters, then click anywhere in your half to place it. Pick a gadget (Mine, Bomb, Turret, Artillery) and click to hide it in your half. Repeat until your 1000 Supply is spent, or click *Place default army*. *Undo last placement* refunds the last platoon or gadget. Press **START BATTLE** when ready. The enemy can't see your placements.
2. **Battle (15:00).** Command platoons with missions and signals. Supply grows at 3/s: build a platoon, choose **On foot** (they walk from base to the target) or **Vehicle** (+150: a transport drives them there and then parks as a solid obstacle that gives cover), and click an explored tile. Platoons with an engineer can be ordered to build gadgets, and from the one-minute mark you can call an **air strike** (250 Supply, 90 s cooldown).
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
| Army panel gadget buttons (setup), then left-click | Hide a mine, bomb, turret or artillery piece in your half. Stays selected so you can place several; right-click to stop |
| *Air strike* button (battle), then left-click | Five bombs in a line through that tile, along the line from your King, after a 4 s warning |
| Left-click a fighter | Select its platoon (or your King, or your artillery) |
| 1 / 2 / 3, or the buttons | Choose a mission: Assault / Overwatch / Reposition |
| 4 / 5 / 6 / 7, or the buttons | Platoons with an engineer: build a Mine / Bomb / Turret / Artillery. Then right-click the site |
| Right-click | Signal the selected platoon: the chosen mission (or its current one) at that tile, or the build order. Moves the King if it's selected. Aims your artillery if that's selected |
| Hover a fighter | Details: your fighters show traits, morale, suppression and what they're doing; enemies show only what's visible |
| Hover a tile | The HUD shows its terrain and the directions it has heavy cover from |
| WASD / arrow keys / middle-mouse drag | Pan the camera |
| Mouse wheel | Zoom |
| Space | Pause / resume (single-player only). Orders given while paused apply on resume |
| F | Debug view: fog off, plus the Termbot commander's thoughts |
| N | New map (next seed) |

Signals take **1 s** to travel from your King to the platoon; a small pulse shows each one on its way. Until it arrives, the platoon keeps doing what it was doing, so plan ahead rather than reacting to every enemy move. Every signal also adds confusion when it arrives, and contradictory signals in quick succession add a lot. Confused platoons act on orders late, and very confused ones freeze.

**Overhead icons:** `RUN` broken · `??` frozen · `!!` pinned · `?` confused · `!` suppressed · `++`/`+` injured · `~` shaken · `^` brave · `*` veteran · `**` elite. Gold diamond = platoon leader. Cyan streaks are sniper and turret shots. Red rings with an X are incoming strikes (the inner ring closes as the impact nears); grey puffs are smoke.

## What's in it

- **Random, balanced maps.** 128×128 tiles, fully random rather than mirrored. Each half gets the same share of hills, high ground and forest, and the same number of each kind of obstacle. Maps whose halves still differ too much are rerolled. The terrain has three elevation levels with ledges and ramps, and forests. Obstacles include ruined buildings with doorways, rock outcrops, concrete barrier walls, boulders and sandbag lines. Rocks and walls are solid: they block movement and sight. **Cover is directional:** an obstacle next to a fighter gives heavy cover only against shots from that side (within about 37°). Fighters can peek over cover they're hugging. Sandbags and trees give light cover. Every map is validated: every walkable tile is reachable, no high ground overlooks a spawn, and the halves are balanced. A seed that fails validation is rerolled.
- **12 sectors** (A1–C4), used for territory control. Rows 1–2 are the Humans' half and rows 3–4 are the Termbots'.
- **Two layers of knowledge.** Each unit has its own vision: radius-based, extended by high ground, blocked by higher ledges, rocks, walls and forest depth. The commander's fog is per tile. Each side starts with a large circle around its King revealed, peeking a few tiles past the middle, and every tile a unit sees stays explored. Unexplored ground is completely dark. Enemy units show up only on explored ground.
- **Platoons.** 1–5 fighters of any classes, with call signs Alpha, Bravo, and so on. Fighters roll Accuracy, IQ, Tactics, Bravery and Leadership within their class's range. The best leader leads, and the next one steps up if they fall.
- **Economy.** 1000 starting Supply, +3/s for both sides. Base prices: Assaulter 50, Engineer 70, Heavy Weapons 80, Sniper 90, Juggernaut 120, and +150 to deliver by vehicle. Each living fighter of a class raises that class's price by 25%.
- **Vehicles.** A vehicle drives at 6 tiles/s, has armor and can be shot on the way. On arrival it unloads its platoon in Overwatch and parks as a solid, cover-giving obstacle. If it's destroyed on the way, the passengers bail out hurt and shaken, and the wreck becomes the obstacle.
- **Missions.** Assault bounds toward the target cover to cover; Overwatch holds in cover with reaction fire; Reposition sprints and dodges, then switches to Overwatch once it arrives. The three form a rock-paper-scissors loop through hit-chance modifiers. Enemy missions are never shown: you read them from how the enemy moves and fires.
- **Fighter autonomy.** Each fighter re-thinks several times a second. Broken fighters flee, pinned fighters stay down, and frozen platoons stop following orders. Otherwise, fighters score taking cover, advancing, fighting or holding, from what they see and their state, tilted by their traits. They choose cover that puts obstacles between themselves and the enemies their platoon knows about, weighting closer threats more. They pick spots they can shoot from when engaging, and out of sight when hurt or pinned. They shoot on their own at whatever they see, with penalties for moving, suppression and injury.
- **Combat.** Health, armor, suppression, morale, deaths, and leader succession. Killing the enemy King wins.
- **Damage and armor types.** Weapons deal Standard, Piercing (strong against light armor), Explosive (strong against heavy armor) or Suppression damage (weaker hits, much more suppression). Units are Light (assaulters, snipers, engineers), Medium (heavy weapons, artillery) or Heavy (juggernauts, vehicles, turrets).
- **Hidden loadouts.** Every fighter rolls a loadout when bought, and the enemy only learns it when it's used: assaulters carry a rifle, grenades (thrown at dug-in enemies) or a medkit; snipers piercing rounds or a smoke kit (smoke blocks sight); heavy weapons a machine gun (Suppression) or rockets (Explosive, against armor and people behind walls); juggernauts a shotgun or a breaching charge (cracks the rock or wall an enemy hides behind); engineers a toolkit (60% faster work) or a mine layer (plants two mines on its own). Your tooltip says whether the enemy knows yours.
- **Engineers and gadgets.** Engineer (70): builds what you order and defuses enemy traps it finds. Building takes time (mine 3 s, bomb 4 s, turret 20 s, artillery 30 s), and the engineer stands still at the site while it works. Every other free engineer in the same platoon comes to help, and each one adds its own speed: two engineers build twice as fast, three three times as fast. Mine (40): explodes under the first enemy to step on it. Bomb (70): big blast when enemies come within 2 tiles. Turret (150): automatic gun, tough, never scared. Artillery (220): shells the biggest enemy group your side can see, or where you aim it, with a 3 s warning. All gadgets start hidden: an enemy engineer spots traps from 5 tiles, anyone else only when right next to one; turrets and artillery are spotted within 6 tiles or when they fire. Fighters walk around traps their side knows about.
- **Strikes, explosions and terrain.** Artillery shells, air strikes, grenades, rockets and breaching charges land after a visible warning, and fighters who see it coming try to get clear (smarter ones react faster). Blasts hurt everyone in the radius, friendly or not, and pin fighters down. They also break terrain: walls and parked vehicles become rubble, sandbags are flattened, trees burn, and breaching charges crack rock. The map updates live.
- **Veterancy.** Fighters earn experience from damage dealt and kills. Veterans (`*`) and elites (`**`) get +5 to every trait at each rank.
- **Simulation architecture built for multiplayer.** Game logic runs in `FixedUpdate` at 20 ticks per second, headless-capable. Every change goes through a `CommandQueue` of `SimCommand`s, addressed by stable `UnitId`s. Randomness comes from a seeded `SimRng`. Rendering interpolates between ticks, so tile-to-tile movement looks smooth.

## Project layout

```
src/
  main.rs            App setup and plugins
  game.rs            Match setup / teardown (seed, spawning, resources)
  faction.rs         Humans / Termbots, local player
  map/
    mod.rs           Tiles, terrain rules (can_step, move cost), sectors
    generate.rs      Procedural random map generation + balance validation
    noise.rs         Seeded value noise
    render.rs        Placeholder map texture, sector borders and labels
  sim/
    mod.rs           Fixed-tick simulation: command queue, movement, interpolation
    classes.rs       Fighter class stats, trait ranges, loadouts, damage/armor types
    fighter.rs       Health, condition (morale, suppression, decision), veterancy, statuses
    platoon.rs       Platoons, missions, signals, confusion, spawning
    economy.rs       Supply, prices, setup phase, deployments, territory, timer
    vehicle.rs       Troop vehicles: drive, unload, park as obstacles
    gadgets.rs       Mines, bombs, turrets, artillery, air strikes, explosions,
                     loadout abilities, engineer work, trap detection
    gadget_tests.rs  Scenario tests for all of the above
    ai.rs            Fighter autonomy (utility AI)
    combat.rs        Hit chance, shooting, morale, deaths
    perception.rs    Unit vision and line of sight
  fog.rs             Per-tile commander fog + overlay
  pathfinding.rs     A* on the tile grid
  commander.rs       The default army (setup button)
  termbot/
    mod.rs           The AI commander: posture, counters, King safety, spending
    intel.rs         What it sees; reading enemy groups' behaviour
    planning.rs      Its style, hidden setup placement, reinforcement choice
  army_panel.rs      Platoon builder: setup placement and reinforcements
  units.rs           Unit visuals: sprites, status icons, health bars, tracers
  input.rs           Selection, mission buttons, signals -> commands; pause
  camera.rs          Pan / zoom
  hud.rs             HUD, hover tooltip, victory/defeat banner
```

Game logic (map generation, pathfinding, line of sight) has no rendering code and is covered by unit tests.

## Tests

`cargo test` also runs headless matches. They check that setup places both armies, that a vehicle delivers its platoon and parks as an obstacle, that the timer ends the match on territory, and that the same seed replays a two-minute battle identically. The tests also check that the AI commander only knows what its side has seen, hides gadgets during setup, never confuses its own platoons into freezing, and plays identically from the same seed. Scenario tests on an open sandbox map cover every gadget and ability: mines going off, engineers finding and defusing traps and building on orders, turrets revealing themselves by firing, artillery and its warning, fighters dodging strikes, air strike costs and cooldowns, walls breaking, grenades, rockets, breaching charges, smoke, loadouts staying secret until used, setup undo and experience.

Debug reports: `AI_VS_AI=1 SEED=42 MINS=10 cargo test ai_report -- --ignored --nocapture` (watch the AI's decisions, optionally commanding both sides), `cargo test battle_report -- --ignored --nocapture` (battle summary per seed), `cargo test cover_report -- --ignored --nocapture` (how often fighters in contact are in cover), and `SEED=42 SECS=90 cargo test fighter_report -- --ignored --nocapture` (every fighter's state at a moment).

## Next: step 8

Networking: two players over the network, using the command queue and deterministic simulation that everything already runs on.
