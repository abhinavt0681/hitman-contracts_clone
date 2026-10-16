# Hitman: Contracts — The Bjarkhov Bomb, rebuilt

A fan project that rebuilds *Hitman: Contracts* (IO Interactive, 2004) in Rust and
[Bevy](https://bevyengine.org) 0.20, one mission at a time, to see how much of it can be
cloned. It starts with **The Bjarkhov Bomb** (scene `C02-1`): a Russian airbase in a blizzard,
a cargo plane, barracks and a nuclear submarine.

It doesn't ship any of the game. It reads the level, models, textures, animations, sounds and
Jesper Kyd's score straight out of **your own copy** of Hitman: Contracts, using format
readers reverse-engineered for this project.

> Not affiliated with or endorsed by IO Interactive or Square Enix. *Hitman* and all game
> content belong to their owners. You need a legitimately owned copy of the game to run this.

## What works

- **The level:** the whole of C02-1 from the original files (geometry, textures, lamps,
  the AN-124, the submarine), with collision and navigation built from the level's own data.
- **47:** walk, run, sneak, drag bodies, take outfits as disguises, all with the original
  animations. Footsteps are timed to the animation and sound different on each surface.
- **The mission:** the original's objectives (Fuchs, Bjarkhov, the dirty-bomb production,
  escape on the plane), a voiced briefing, a pause menu, a map screen drawn from the game's
  own floor plans, and mission ratings.
- **Weapons:**
  - The silenced Silverballer and the AK-47, using the original's magazine, precision,
    range, damage and rate of fire.
  - The fiber wire, using the original strangle animations.
  - Poison and the bomb remote.
- **Guards:**
  - They react to light, noise, disguises, bodies and drawn guns, then investigate, search
    and hunt you.
  - They fire in bursts, and they hit.
  - The blizzard blinds them: about 22 m of sight outdoors, down to 6 m in a squall.
- **Atmosphere:**
  - A GPU blizzard (wind-driven streaks, sheets, drifting snow) with gusts that come and go.
  - The flag and the canvas move as Verlet cloth.
  - The level's 95 placed sound emitters play in 3D, under an adaptive Kyd score.
- **HUD:** in the original's minimal style (health and suspicion bars, the weapon panel, a
  small crosshair).

**Not yet:**
- running the original level scripts (cutscenes, the scripted behaviour of Bjarkhov, Fuchs
  and the guards)
- trespass zones
- the original guard logic, collision from the `.OCT` files, and the other missions

See [Roadmap](#roadmap).

## Playing it

### 1. Get the game data

You need Hitman: Contracts (the Steam and GOG releases both work). From its install
directory, copy into `data/` in this repository:

```
data/
├── Scenes/          ← the whole Scenes folder (at least Scenes/C02-1 and Scenes/AllLevels)
└── streams.wav      ← the music and voice streams
```

On Linux with Steam the install is usually
`~/.local/share/Steam/steamapps/common/Hitman Contracts/`. `data/` is gitignored; never
commit anything from it.

### 2. Build and run

You need a recent stable Rust toolchain from [rustup](https://rustup.rs) (Rust edition 2024).
On Linux you also need Bevy's usual system packages; see Bevy's
[Linux dependencies](https://github.com/bevyengine/bevy/blob/main/docs/linux_dependencies.md)
(ALSA and udev headers, and Vulkan drivers).

```sh
cargo run --release
```

The first launch builds the navigation mesh and caches it in `data/cache/`. Use `--release`:
debug builds parse the level slowly.

It's developed on Linux with an RX 570. Windows and macOS should work through Bevy, but
nobody has tested them yet.

### Controls

| Key | Action |
|---|---|
| Mouse | look (click to capture the cursor) |
| WASD, Shift | move, run |
| Ctrl / C | sneak |
| E | interact: strangle from behind, take an outfit, pick up or place things, leave on the plane |
| F | take a dead soldier's AK-47 |
| G | drag / drop a body |
| 1–6, mouse wheel | holster, fiber wire, Silverballer, poison, bomb remote, AK-47 |
| Left click | use the item / fire |
| Right mouse | aim over the shoulder |
| R | reload |
| M / Tab | map and objectives |
| Esc | pause menu (objectives, briefing, restart, quit) |
| F1 | free-fly debug camera |

It's meant to be punishing: guards who see you holding a gun, or find a body, will hunt you,
and a burst of AK fire kills in seconds.

## Building on it

### Repository layout

| Path | What's there |
|---|---|
| `src/glacier/` | Readers for the original formats, also a library (`hitman_clone::glacier`): `gms` (level entities), `prm` (meshes), `tex`, `anm` (animation), `skin`, `loc` (strings), `audio` (`.WHD`/`.WAV`, `streams.wav`), `oct` (collision trees), `ppo` (compiled level scripts) |
| `src/view.rs` | Spawns the level: meshes, materials, collision, map geometry, held weapons |
| `src/game/` | Mission, guard AI (`ai.rs`), actions, weapons, doors |
| `src/player.rs`, `src/character.rs` | 47's movement and camera; skeletal animation with an upper-body layer |
| `src/atmosphere.rs`, `snow.rs`, `cloth.rs`, `shaders/` | Lighting, grade and fog, gusts, the blizzard, cloth |
| `src/audio.rs`, `music.rs`, `ambience.rs`, `footsteps.rs` | Sound bank, adaptive score, positional ambience, footsteps |
| `src/flow.rs`, `hud.rs`, `map.rs` | Briefing, pause menu, HUD, map screen |
| `src/collision.rs`, `nav.rs` | Ray and capsule collision, navigation grid |
| `src/shot.rs` | Scripted runs and screenshots for testing without a human (see below) |
| `src/bin/glacier-dump.rs` | CLI that inspects level data: entities, models, textures, animations, sounds, scripts |
| `docs/` | Everything learned about the formats and the engine (start here) |
| `tools/` | Ghidra export, small Python probes |

### What's documented

- [`docs/FORMATS.md`](docs/FORMATS.md): the Glacier 1 file formats, byte by byte, plus the
  coordinate conventions (they matter: mesh vertices, bones and held items each use a
  different axis swap).
- [`docs/SCRIPTS.md`](docs/SCRIPTS.md): the `.ppo` compiled-script format and its stack VM,
  with the interpreter's addresses in the exe. All 32 C02-1 scripts decode cleanly.
- [`docs/DECOMPILATION_OCT.md`](docs/DECOMPILATION_OCT.md): the collision octree (`.OCT`)
  and the engine classes behind it.
- [`docs/ATMOSPHERE.md`](docs/ATMOSPHERE.md): the target look, the score's cues, the colour
  grade.
- [`AGENTS.md`](AGENTS.md): working notes for contributors and their AI assistants:
  conventions, pitfalls, how to test, what's next.

### Decompiling the original yourself

The engine's behaviour (the script VM, weapons, AI messages) was studied in a Ghidra
decompilation of `HitmanContracts.exe`. That output isn't in this repository (see
[Legal](#legal)), but you can rebuild it from your own copy in about three minutes:

1. Install [Ghidra](https://ghidra-sre.org) 12.1.4 and JDK 21.
2. Copy `HitmanContracts.exe` from your install into `data/`.
3. Run `GHIDRA=/path/to/ghidra_12.1.4_PUBLIC tools/ghidra/decompile.sh`.

You get `data/re/ghidra/` with decompiled C for every function, a function index and a
string-reference table. Addresses in `docs/` refer to the Steam build of the exe, SHA-256
`189f8ce2e40603db1387266e960fcf102c479ad4de83d31b1cbfe3647b6cd702`; the script prints
yours so you can compare.

To get readable listings of the mission's level scripts:

```sh
cargo run --release --bin glacier-dump -- C02-1 --scripts data/re/scripts
```

### Testing without a human

Nearly everything can be checked by a scripted run that saves a screenshot, which is also how
AI assistants can see what they changed:

```sh
HITMAN_SCREENSHOT=out.png HITMAN_CAM="50,1.7,40,66,6,3" cargo run --release   # camera at x,y,z looking at x,y,z
HITMAN_START=46,1,-22 HITMAN_KEYS="3@0.3,RMB@0.8-5,LMB@3" HITMAN_SCREENSHOT=out.png cargo run --release
```

`src/shot.rs` documents the variables: camera placement, 47's start and facing, timed key
presses, auto-walking, and which frame to capture. Debug views live in `src/view.rs`
(`HITMAN_LIST_NEAR`, `HITMAN_HIDE_TEX`, `HITMAN_COVERAGE`), and `glacier-dump`'s options
(`--entities`, `--model NAME`, `--scripts DIR`, …) are at the top of `src/bin/glacier-dump.rs`.

### Contributing

- **Never commit game content:** nothing from `data/`, no extracted textures, sounds, models
  or text, and no decompiler output. Reference it by name, offset or address instead.
- **Prefer the original's numbers and behaviour** when they can be found in the data or the
  exe, and say where each one came from. Mark guesses as guesses in the code.
- **Don't trade atmosphere for frame rate.** Snow density, fog and effects stay as dense as
  they are.
- **Document format and engine findings** in `docs/` as you make them.
- **Verify visual changes with a scripted screenshot** before and after.

## Roadmap

1. **Script VM:** run the original `.ppo` level scripts, starting with the guards and
   Bjarkhov. The decoder and opcode map already exist.
2. Trespass zones and the original guard reactions, ported from the scripts.
3. Cutscenes and scripted dialogue.
4. Collision from the `.OCT` files (the format is parsed but not used yet).
5. The action menu as the original has it.
6. More missions: the readers are generic, and other scenes already load in `glacier-dump`.

## Legal

This repository holds only original code and documentation written for it. The
documentation describes file formats and the original engine's behaviour: what functions do
and where they are. It doesn't contain the game's code, decompiled or otherwise, or any of
its assets. Keep it that way, so the project can stay up.

The bundled font, Michroma, is under the SIL Open Font License (`src/fonts/OFL.txt`).

Licence for the code: _to be decided by the maintainer._

## Credits

- IO Interactive, for the game, and Jesper Kyd, for the score it can't do without.
- [ReGlacier](https://github.com/ReGlacier) (BMEdit, ReHitman) for public Glacier 1
  research that gave this a head start.
