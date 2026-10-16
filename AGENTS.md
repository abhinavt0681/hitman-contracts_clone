# Working notes for contributors and AI agents

Read this before changing anything. [README.md](README.md) covers setup and the project's
purpose. This file covers how the code works, its conventions, and the traps that already
cost time.

## Ground rules

- **No game content in git.** `data/` (the user's copy of the game, the decompiler output,
  caches, reference images) and `Hitman Contracts/` are gitignored. Don't paste decompiled
  code, extracted text or assets into source or docs either: describe behaviour, and cite
  addresses, offsets and names.
- **The original is the spec.** Take numbers and behaviour from the level data or the exe
  where they can be found, and say where each came from in a comment. If you have to guess,
  label it as a guess (`// assumption: …`).
- **Atmosphere over performance.** Never thin the snow, fog, cloth or effects to gain frame
  rate unless the maintainer asks. The look is deliberately dark: a night blizzard in teal and
  blue-grey with sodium lamp pools (see `docs/ATMOSPHERE.md`).
- **Keep it punishing.** Guards are dangerous, and mistakes cost. Modernising the controls is
  fine; making the game easier is not.
- **Look, don't assume.** Check every visual or gameplay change with a scripted run and a
  screenshot (see [Testing](#testing)). Unit tests are few; scripted runs are how this
  project checks things.
- **Write it down.** Format and engine findings go in `docs/` and in module docs, as you
  make them.

## How it runs

`main.rs` loads scene `C02-1` through `glacier::level::Level` (from `data/Scenes`, or
`HITMAN_SCENES`), then `view.rs` spawns every placed mesh. Gameplay is a set of Bevy plugins:

| Plugin / module | Role |
|---|---|
| `flow` | `Phase` state: `Briefing` → `Playing` ⇄ `Paused` / `Map`. Gameplay systems run `run_if(in_state(Phase::Playing))`; the clock is `Time<Virtual>`, paused outside `Playing` |
| `game` (`mod.rs`, `mission.rs`, `actions.rs`, `ai.rs`, `weapons.rs`, `doors.rs`) | `Mission` resource (objectives, items, ammo, health, outfit, prompt, messages); `Actor` component on every NPC; anchors from named level entities |
| `player` | 47's controller; `OrbitCamera { yaw, pitch, aim }` is the main camera (query it with `With<OrbitCamera>`, never `With<Camera3d>`: there are three 3D cameras) |
| `character` | Skinned characters and the `Animator`: a full-body clip plus an upper-body layer (`play_upper`, `pose_upper`, `stop_upper`) |
| `atmosphere` | Lamps (`Lamps`), beacons, `Gust` (squalls), `Exposure` (how lit 47 is; whether he's indoors), the colour grade via `look()` |
| `snow`, `shaders/*.wesl` | GPU blizzard: streaks, sheets and drifts, occluded by a height map, lit by lamps |
| `cloth` | Verlet cloth for the flag and canvas (GMS class `0x2000F6`) |
| `audio`, `music`, `ambience`, `footsteps` | `SoundBank` (sounds by name from the level's `.WHD`/`.WAV` and `streams.wav`); the adaptive score; the level's 3D sound emitters; animation-timed footsteps by surface |
| `hud`, `map` | The HUD (with a weapon icon rendered live on render layer 2); the map screen (orthographic camera on render layer 1, showing the original's map meshes) |
| `ragdoll` | Rapier physics (`bevy_rapier3d`): the dead go limp (13 jointed capsules, knee and elbow hinges measured from the original clips); the level is one fixed trimesh; dragging pulls a wrist or ankle |
| `ladder` | Ladders from the level's `*Guide` entities (box sits 1.1 m above the floors it joins), facing from the ladder mesh; `Climb_Ladder` frames 0–23 are the looped stroke, 23–57 the climb-over |
| `cutscene` | Letterboxed dialogue scenes: `Cutscene::play(id, lines)`, `finished(id)`; lines are voice keys (`C02-1_GIHMI_01G`), subtitles from the string table; participants get `InScene` (AI skips them) |
| `tie` | 47's tie: the template's `Tie` (class 0x2000FF, simulated in the original) hung from his chest bone at rest, on a damped spring |
| `collision`, `nav` | Triangle soup with surfaces; a navigation grid cached in `data/cache/nav_C02-1_v{CACHE_VERSION}.bin` |
| `shot` | Scripted input and screenshots |

## Coordinate conventions (read before touching transforms)

- Glacier units are centimetres; `view::glacier_to_bevy` scales by 0.01 and mirrors Z.
- **Static meshes:** PRM vertices have `P` (a swap of X and Z) baked in when built; the
  instance transform is `S·R·0.01`.
- **Bones** are `P`-conjugated: quaternion `(x, y, z, w)` → `(−z, −y, −x, w)`.
- **Items held in the hand:** `SWAP_XZ * grip.inverse()`, where `grip` is the template's
  `PosBox_Hand` (see `weapons::hold_models`). Joints carry no `Visibility`, so insert
  `Visibility::Inherited` on a joint before parenting anything to it.
- Upper-body aim clips (`UB_Aim_Gun`, `UB_Aim_Rifle`) sweep through aim directions rather
  than playing as motion. The engine picks a frame; we hold the straight-ahead one
  (`WeaponDef::aim_frame`: 0.0 for the pistol, 0.6 s for the rifle).

## Format facts that aren't obvious

- **Scene ids:** The Bjarkhov Bomb is **C02-1**. C03-1 is Beldingford Manor; the shared
  `.LOC` string table makes this look otherwise.
- **PRM submeshes:**
  - Texture 0 marks an untextured collision hull.
  - Render-state bit `0x800000` marks a proxy that's never drawn, even when it has a texture
    (`SubMesh::is_proxy`). For example, the AN-124 has a ground-to-roof box inside its
    fuselage, and walkways and mess-hall floors have second copies. Drawing them hides the
    real models.
- **Non-solid names:** `Sky`, `Blowing`, `Fog`, `Glitter`, `GlowVol`, `GlowRain`, `Snow_`
  and `SnowBlowing` are effects. Map floor plans (`…MAP_…` meshes, `HMC_IngameMap_*`
  textures) go on the map layer only. Drawing them in the world caused the "cracked floor".
- **Collision:**
  - Upward-facing visible triangles always count.
  - Structures over 6 m also add their visible walls.
  - Proxies are solid.
  - Bump `nav::CACHE_VERSION` whenever collision input changes.
- **GMS property tags:**
  - `0x02/0x42` float, `0x03/0x83/0x43` int, `0x81/0x41` double
  - `0x89` reference table, `0x8b` reference, `0x06` enum/colour, `0x04/0x84` string
  - Classes: weapon template `0x100429`, ammo `0x1007d5`, sound emitter `0x200012` (tag
    `0x8b` at +9 is the `SndRef`), cloth `0x2000F6`.
- **ZGF:** `<ID>_MAIN.ZGF` holds the compiled `.ppo` scripts, deflated like the GMS. See
  `docs/SCRIPTS.md`.
- **String table extras:** item descriptions sit one level down (`Item_Key_Master_01` →
  `FreeText` → `1` → text); `Strings::description(item)` reads them. Voice lines are keyed by
  their sound names, so a scene's line key is both its voice file and its subtitle.
- **HUD art:**
  - The level's `.TEX` holds it: `HealthStealth_01a`, `Ammo_01a`, `AmmoBacklight_01a`,
    `OSDBar_*`.
  - Fonts are pre-rasterised Bank Gothic atlases with no metrics, hence Michroma.
  - `examples/tex_dump.rs` lists and dumps any `.TEX` file.

## The original engine (Ghidra)

Regenerate the decompilation locally with `tools/ghidra/decompile.sh` (see the README). The
key addresses, with more in `docs/SCRIPTS.md` and `docs/DECOMPILATION_OCT.md`:

| What | Where |
|---|---|
| Script interpreter (one `switch`, opcodes `0x00`–`0xA1`) | `FUN_0066a2b0` |
| `IIMV` image loader | `FUN_00679410` |
| Name lookup up the inheritance chain | `FUN_0067a090` |
| Function table | `FUN_006788a0` |
| State table loader (`0x11223344`) / change state | `FUN_00677f80` / `FUN_00668dc0` |
| Action controller messages (the action menu) | around `FUN_00490280` |
| Dual-weapon lookup and inventory item offsets | `FUN_0047f810` |

Script properties give the starting facts: characters have `m_iHitpoints=20`, and guards
have `RelaxedVisRange=3000`.

## Gameplay numbers and where they come from

- **Weapons** (`game/weapons.rs`): from the GMS weapon and ammo templates.
  - Silverballer: semi-auto, 14 rounds, 0.35°, 30/80 m, 2.1→0.7 damage, 300 rpm.
  - AK-47: full auto, 30 rounds, 0.6°, 150/200 m, 1.8→1.2 damage, 600 rpm.
  - Fiber wire (`Carrot`): 47 plays `Carrot` and the victim `Die_Carrot`, 4.7 s.
- **Assumptions,** until the script VM can confirm them:
  - `DAMAGE_SCALE` = 10.
  - Hit-zone multipliers: head ×5, limbs ×0.5.
  - Guard reaction time 0.8–1.4 s, bursts of 3–6 rounds.
  - Silenced shots alarm within 5 m and are heard at 15 m; loud shots carry 60 m.
- **Sight** (`game/ai.rs`):
  - In light, 30 m; in the dark, 9 m.
  - Outdoors the blizzard caps sight at `WHITEOUT` 22 m, or `SQUALL_WHITEOUT` 6 m in a full
    squall, even for hunting guards. Indoors, no cap.
  - Hearing drops by up to 60% in a squall.

## Bevy 0.20 notes

- Shaders are WESL, embedded with `embedded_asset!`, using
  `@group(constants::MATERIAL_BIND_GROUP)`.
- `RenderTarget` is its own component; `AmbientLight` is a per-camera component, and
  `GlobalAmbientLight` is the resource.
- UI hover is `bevy::picking::hover::Hovered`; the old `Button` / `Interaction` are
  deprecated.
- Text uses `TextFont { font: FontSource, font_size: FontSize::Px(..) }`, and `LetterSpacing`
  is a component.
- A system takes at most 16 parameters; group them in tuples.
- Spatial audio uses `SpatialScale`; rodio falls off by the inverse square.
- `Mesh::compute_aabb` is private; read `Mesh::ATTRIBUTE_POSITION` instead.
- Physics is `bevy_rapier3d` 0.37, the release for Bevy 0.20. Upgrading Bevy means waiting
  for a matching Rapier release. Ragdoll bodies are top-level entities; `ragdoll::pose_bones`
  copies them into the skeleton after Rapier's writeback and before transform propagation.
  A second joint on a body goes on a child entity (see `drag_by_limb`).

## Testing

`src/shot.rs` documents scripted runs. The variables most used:

- **Capture:** `HITMAN_SCREENSHOT=out.png`, then exit. `HITMAN_SHOT_FRAME=n` captures at a
  later frame.
- **Camera:** `HITMAN_CAM="x,y,z,tx,ty,tz"` places a fixed camera, and turns off fog unless
  `HITMAN_CAM_FOG=1`.
- **47:** `HITMAN_START="x,y,z"` sets where he starts and `HITMAN_YAW` his facing (radians;
  0 faces −Z).
- **Input:**
  - `HITMAN_KEYS="E@0.2,RMB@0.8-5,LMB@3"` taps or holds keys and mouse buttons at given times.
  - `HITMAN_AUTOWALK=secs[@start]` holds W; add `HITMAN_AUTORUN` to run, or set
    `HITMAN_AUTOKEY` to hold another key.
- **Setup:**
  - `HITMAN_ITEM=AssaultRifle` sets the starting item and `HITMAN_OUTFIT=Soldier` the outfit.
  - `HITMAN_BEHIND=<actor>` starts 47 behind that actor.
  - `HITMAN_ALL_OBJECTIVES`, `HITMAN_TNT` and `HITMAN_BOMBS_PLACED` jump ahead in the mission.
  - `HITMAN_PHASE` / `HITMAN_SKIP_BRIEFING` control the briefing (scripted runs skip it).
- **Look:**
  - `HITMAN_NO_SNOW`, `HITMAN_SNOW_LAYERS`, `HITMAN_GUST=0..1` control the weather.
  - `HITMAN_LOOK=faithful|mid` picks an alternative grade (the default is the dark one).
  - `HITMAN_TONEMAP`, `HITMAN_SHADOWS`.
- **Debugging the level:**
  - `HITMAN_LIST_NEAR="x,z,r"` (with `RUST_LOG=info`) logs meshes, textures and render states.
  - `HITMAN_HIDE_TEX=ids` hides parts by texture id; `HITMAN_LIST_TEX=id` logs the entities
    that use a texture.
  - `HITMAN_COVERAGE` and `HITMAN_HOLES` show ground coverage and holes.
  - `HITMAN_PROBE` casts a collision ray.
- **Fly camera:** F1 toggles it only when launched with `HITMAN_FREECAM=1` (a stray F1 from a
  laptop media key used to detach the camera mid-game).
- **Outfit tests:** `HITMAN_OUTFIT=Fuchs` (no model swap) triggers the gate, frisk and Bjarkhov
  scenes at their anchors (logged at load as `anchors:`); `HITMAN_UNARMED` starts without the
  pistol (to pass the frisk).
- **Ragdolls:** `HITMAN_KILL="Name@secs"` shoots a character dead at that time;
  `HITMAN_PHYSICS_DEBUG` draws the physics colliders.
- **Stealth in tests:** starting 47 right behind a guard while standing in a suit gets him
  heard and the base alarmed. Add `C@0.1` to `HITMAN_KEYS` (sneak) and `HITMAN_OUTFIT=Soldier`.

Useful places:
- 47 near the start crates: `46,1,-22`.
- The AN-124: around `(66, 2.5, 3)`, nose and front ramp towards +X.
- The exit trigger is inside the cargo hold.

## Roadmap

1. **Script VM:** execute `.ppo` (`glacier/ppo.rs` already decodes and links all 32 C02-1
   scripts with no misaligned jumps). Start with the guards and Bjarkhov. The open pieces
   are listed in `docs/SCRIPTS.md`: signature tables, event-handler tables, property
   offsets, and native calls to port.
2. Trespass zones; the original guard reactions and alert levels.
3. Cutscenes and scripted dialogue.
4. Collision from `.OCT` (parsed in `glacier/oct.rs`, unused).
5. The action menu (see the action controller above).
6. Other missions (C03-1, C06-1 and C08-1 already parse in `glacier-dump`).

The maintainer decided against dual-wielded Silverballers and the first-person camera, even
though both exist in the data.
