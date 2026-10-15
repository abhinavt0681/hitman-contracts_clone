# Hitman: Contracts (Glacier 1) level formats

Notes from reverse engineering `Scenes/C02-1` (The Bjarkhov Bomb). All values are little-endian and all
offsets are absolute within their file unless noted. The readers in `src/glacier/` are the executable
version of these notes.

## Scene list

| Scene | Mission |
|---|---|
| C02-1 | The Bjarkhov Bomb (airfield, barracks, submarine) |
| C03-1 | Beldingford Manor |

Identify scenes by the `.Pof` location names inside the inflated GMS (`C2-1_Inside_Sub`,
`C2-1_Outside_Airport`, …). Don't use `.LOC`: it is a shared string table and mentions every mission.

## Container: `<ID>_MAIN.ZIP`

A zip holding `SCENES/<ID>/<ID>_MAIN.{GMS,BUF,PRM,TEX,ANM,AMS,SND,LOC,OCT,RMC,RMI,SGP,SUP,ZGF}`.
Most entries are stored; some are deflated. `_Load.ZIP` holds the loading-screen scene.

## TEX: textures

- Header: `u32 table_offset, u32 table2_offset, u32, u32`.
- `table_offset` → 2048 × `u32` entry offsets, indexed by texture id (0 = unused; ids start at 32).
- Entry: `u32 size, char[4] fourcc (reversed: "1TXD"), char[4] fourcc, u32 id, u16 height,
  u16 width, u32 mip_count, u32 flags[3], char name[]` then `mip_count` × (`u32 size, bytes`).
- Formats: `DXT1`, `DXT3`, `RGBA` (stored R,G,B,A), `PALN` (8-bit indices; after the mips,
  `u32 count` + `count` × RGBA palette).

## PRM: meshes

- Header: `u32 ?, u32 table_offset, u32 table_offset, u32 count`.
- `table_offset` → `count` × `u32` offsets. Each is the **last** sub-mesh record of one model.
- Sub-mesh record (relative offsets):

| Offset | Type | Meaning |
|---|---|---|
| 0x00 | u16 | flags, 0x181 for ordinary geometry |
| 0x04 | u32 | texture: TEX id in bits 0–10; bit 0x800 = alpha-blended using vertex alpha |
| 0x08 | u32 | previous sub-mesh record of this model (0 ends the chain) |
| 0x0E | u16 | vertex count |
| 0x10 | u32 | model-info block shared by all sub-meshes; bbox max xyz at +0x20, min at +0x2C |
| 0x14 | u32 | vertices: 36 bytes each — pos f32×3, normal f32×3, BGRA u8×4, uv f32×2 |
| 0x20 | u32 | render state; bits 0xC000 mark collision proxies (not decoded further) |
| 0x34 | u32 | index block: `u16 prim_count`, then per prim `u16 n, u16 idx[n]` |
| 0x38 | u32 | index block length in u16 words |

- Prims are triangle strips (alternate winding); `n == 2` prims are line lists (debug boxes).
  Index blocks may be shared between sub-meshes.
- Sub-meshes with texture id 0 are **collision proxies**: a cylinder for a drum, boxes for doors
  and fences. They are never drawn.
- **Vertex X and Z are swapped** relative to the GMS entity matrices (see below).

## GMS: scene graph

- File: `u32 inflated_size, u32 compressed_size, u8 is_raw`, then raw DEFLATE (no zlib header).
- Body `+0x00` → entity table: `u32 count`, then `count` × `(u32 packed, u32 ?)`.
  `packed & 0xFFFFFF` is the entity offset in 4-byte words; the top byte is tree info.
- Entity record:

| Offset | Type | Meaning |
|---|---|---|
| 0x00 | u32 | name, offset into `.BUF` |
| 0x04 | u32 | → 9 × f32 rotation. Applies to row vectors (`v·M`), determinant −1 |
| 0x08 | u32 | → 3 × f32 world position, centimeters, Y up |
| 0x0C | u32 | PRM byte offset of the model's last sub-mesh record (if it is a table entry) |
| 0x10 | u32 | class id |
| 0x18 | u32 | flags; 0x400 = effect template spawned at runtime |

- Placement: `world = (P·v)·M + p` where `P` swaps vertex X and Z. P explains the determinant −1;
  the evidence is 12 m fence segments spaced along world X, whose meshes are 12 m on vertex X but
  carry an x↔z matrix. Symmetric matrices hide the row-vector convention; the plane's hold
  segments (`PlaneTransport_Inside_*`) only line up with it.
- Notable classes: `0x200002` static geometry, `0x200015` room portals (`Gate_*`), `0x20001C`
  room bounds (`InsideBound_*`), `0x100001` templates/groups, `0x2000E1` trigger boxes,
  `0x800023/24` lights.
- 47's template is the `Hitman2` entity; Commander Bjarkhov is `CommanderBjarkhov_01`.

## PRM: skinned characters (`kind == 1`)

Actor entities in GMS (e.g. `CommanderBjarkhov_01`, `RussianSoldierArctic_s1_06`) are followed by
a child named `Ground` whose primitive is a skinned model. `_HMas` actors are the disguise outfits
47 can wear, and `_NUDE` actors are stripped bodies; both are templates at the origin. 47's own
model hangs off the `Hitman2` template.

- Records are 0x98 bytes and are **not** in the PRM table. They share the static fields (texture,
  prev, vertex count, vertices, indices) and add:

| Offset | Meaning |
|---|---|
| 0x74 | bone count (72 for the standard human rig, 52 for some civilians) |
| 0x78 | → local bind matrices, 48 B/bone |
| 0x7C | → local bind pose, 28 B/bone: quaternion xyzw + position (cm) |
| 0x80 | → model-space bind matrices, 48 B/bone (position at +36) |
| 0x88 | → bone records, 48 B: name at +28, animation channel id at +46 |
| 0x8C | → channel id → bone index table (96 B, 0xFF unused) |
| 0x90 / 0x94 | palette run count / → runs of `(u32 len, u32 start)`, both ÷12 = bones |

- Vertices are 52 B: pos f32×3, weights f32×3, palette bytes u8×4 (slot = byte ÷ 3), normal f32×3,
  BGRA, uv f32×2. The palette is the concatenation of the runs; for the body it is
  PELVIS…JAW, FOREHEAD, eyebrows/eyelids, left arm, right arm, legs. Hands use their own palette.
- No parent indices are stored. The rig is the engine's fixed human skeleton (GROUND → PELVIS →
  SPINE → SPINE_1 → NECK → HEAD, clavicles off NECK, fingers off HAND, lower lips off JAW, other
  face bones off HEAD). Composing the local poses this way reproduces the stored model-space
  matrices exactly (0.000 cm error on all 18 character models in C02-1).
- Composition is standard: `world_q = parent_q · q`, `world_p = parent_p + rotate(parent_q, p)`.
  Same X/Z swap as static meshes: positions → `swap_xz`, quaternions → (−z, −y, −x, w).

## ANM: animations (partial)

- `"ANM\0"` magic (stored reversed), `u32 size|0x80000000, u32 size`, then per animation set a
  node `{u32 0x0A, u32 size|flag, u32 size, u32 1, u32 count, u32 count*4, count × ptr}` followed by
  the set's path (`Worlds\Characters\_Anims\Hero\Hero.anm`, `Reg_Male`, `Bjarkov`, …).
- Rotation keys are 4 signed bytes (x, y, z, w ÷ 127). Tracks are contiguous streams of variable
  length (6–9 keys seen for a 28-frame animation), so key times live elsewhere.
- Near the end: packed strings, first bone names prefixed with their channel id byte, then
  animation names in alphabetical order (`Change_Clothes`, `Drag_Ambient`, …, `Walk_Left`). Then
  a graph of small `u16 type, u16 size` records with relative offsets, animation nodes
  `{u32 id, u16 0x87, u16 frames, u32 track_count, ptrs}`, and 40-byte descriptors at the very end.
  Not yet decoded far enough to play back; characters currently use a procedural gait.

## GMS: actor properties (partial)

Actor records carry a property blob after the usual fields: the animation set path, the actor's
name, its script (`scripts\C02-1\SoldierPatrol.sdl`), the declaration string (`LONG
m_iHitpoints=20;…;ZGEOMREF WayPoints;…`), then values. The encoding isn't Blood Money's PRP
bytecode; reference values (`0x19F208`, …) aren't plain entity offsets.

## Not yet decoded

- `OCT`: collision octree (quantized; origin + 0.49 scale in the header). The clone currently
  collides against render meshes, collision proxies and `Coli*` hulls instead.
- `ANM` playback (see above), `AMS`, `SND`/`WAV`/`WHD` audio, `ZGF`/`SGP`/`RMC`/`RMI`/`SUP`,
  actor property values (patrol waypoints, sentry flags).
