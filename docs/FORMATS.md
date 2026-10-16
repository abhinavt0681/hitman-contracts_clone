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
| 0x20 | u32 | render state; bit 0x800000 marks proxies that are never drawn (untextured hulls, and textured stand-ins such as the box inside the AN-124 fuselage or second copies of walkway and mess-hall floors); bits 0xC000 also mark untextured collision hulls; the rest is not decoded |
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

## ANM: animations

Recovered from `ZAnimFilePack` in `HitmanContracts.exe` (vtable 0x72D0C4, loader 0x61C0A0, sampler
0x61B930); `src/glacier/anm.rs` is the reference implementation.

- A tree of chunks: `u32 type, u32 size|flags`. Bit 31 = container (child count at +0xC, children
  at +0x10, or after an index table when bit 30 is set); with a top bit set the payload is at
  `chunk + [chunk+8]`, else at `chunk + 8`. The root has type `"ANM\0"`.
- Root children: one `0x0A` per animation set (payload = set path such as
  `Worlds\Characters\_Anims\Hero\Hero.anm`; its first child is `u32 count` + offsets of animation
  records), `0x04` strings, `0x05` animation records and tracks, `0x06` bit-packed key tables,
  `0x07` rotation keys, `0x08` position keys, `0x0B` static positions (f32×3), `0x0D` static
  rotations (f32×4).
- Animation record (40 B, in chunk 5): `+0x14` track table offset, `+0x24` name offset (chunk 4).
  Track table: `u32 n` + n offsets (from the start of chunk 5) of tracks.
- Track: `u16 bone_name` (chunk 4; the byte before the name is the bone's channel id), `u16 0`,
  `u16 static_pos`, `u16 static_rot`, `u16 frames`, `u16 flags` (0x1 rotations as 4×i16 ÷ 32512,
  0x2 as 4×i8 ÷ 127, 0x4 keyed positions, 0x8 static position, 0x10 static rotation); unless 0x10,
  `u32 keys, u32 frame_to_key, u32 key_to_frame`; if 0x4 without 0x8, the same for positions.
- Key tables: `i8` bit width, then MSB-first packed values. Key k is at frame
  `k == 0 ? 0 : key_to_frame[k-1]`; the last key is at `frames`. 30 frames per second.
- Rotations replace the bone's local rotation; positions its local translation. The GROUND track
  carries root motion (Walk_Forward covers 1.54 m/s). Death clips end lying down only when that
  root motion is applied.
- C02-1 holds 7 sets / 528 animations: Hero (47, 228), Reg_Male (257), Worker_C2-1, FabianFuchs,
  Chef_C2-1, Bjarkov, Bomb_Builder.

## GMS: actor properties and paths

- Actor records carry, after `+0x20`: the animation set path, the actor name, the script
  (`scripts\C02-1\SoldierPatrol.sdl`), a declaration string (`LONG m_iHitpoints=20;…;ZGEOMREF
  WayPoints;…`), then one tagged value per declared field: tag `3` i32, `2` f32, `8` entity
  reference, `4`/`0x84`/`0x85` NUL-terminated string.
- **Entity references are `0x80000000 | (index + 1)`**: 1-based indices into the entity table, 0 =
  none. (Read 0-based, every reference lands one entity late: `WeaponTemplate` → `Slide` instead
  of `Rifle_AK47_01`.)
- Paths (class `0x080000C8`, e.g. `SoldierPatrol_01`) point from `+0x20` at a node list:
  `u32 size, u32 header, u8 0`, then `(size − 22) / 4` references in walking order, then a 13-byte
  trailer. Nodes are ordinary entities (`Arrow#03`, `07_walk`, `16_Wait_hanginbar`); a guard's
  `WayPoints` refers to its path. All 11 patrols in C02-1 resolve this way.
- Light entities (`0x800023` spot, `0x800024` omni) carry only a position; their shared property
  blob is empty, so colour and radius must come from elsewhere (not found yet).
- Mission anchors are named entities: `Box_01..03` (bomb triggers on the sub's deck, under the
  `bombicon_*` map icons), `SubRadiactionBox` (sic), `RadiationWarning`, `Exitbox`, `TNT_*`,
  `cb_BombWorker_01` (radiation suit). The cargo plane's lights: `RedTailLight`, `RedFrontLight`,
  `RedPosLight`, `TailStrobeLight`, `LeftStrobeLight`, `RightStrobeLight`.

## WHD / WAV / streams.wav: sound

- `<ID>_MAIN.WHD` indexes `<ID>_MAIN.WAV`: from 0x10, records of a NUL-terminated name padded to
  4 bytes plus 48 bytes of metadata (`+8` format tag, `+12` sample rate, `+16` bits, `+20`
  decoded size, `+24` stored size, `+28` channels, `+32` data offset, `+40` block align).
- Sound effects and dialogue are mono IMA ADPCM; music lives in the global `streams.wav` as Ogg
  Vorbis (format tag bit 31; the decoder also treats offsets beyond the WAV as streamed).
- C02-1 has 1530 entries. Dialogue names follow `C02-1_<scene>_<line><speaker>` and the LOC file
  maps them to subtitles (`C02-1_BGHM_01CB` = "So good to meet you at last Fabian…"); the base PA
  is `02-1_LS_01..09` (news, weather, the intruder alarm).

## OCT: spatial physics octrees

Reverse-engineered from `HitmanContracts.exe` (vtable `0x00715cd0` `ZTree`, vtable `0x0073e518` collision manager):
- Scene loader: `0x00425750` (entity groups), `0x00425814` (global trees), `0x00425855` (spatial links).
- Entity tree group loader: `0x00456030` (`ZTreeGroup::Load`, 5 tree slots per entity).
- Octree loader: `0x00629a30` (`ZTree::Load`).
- Global tree loader: `0x006e3dd0` (method `+0x54`).
- Spatial links loader: `0x006e3370` (method `+0x48`).
- Node traverser & ray-triangle intersection: `0x006292f0`, `0x006e2590`, `0x006e36d0`, `0x006e4140`.

An `.OCT` file consists of three sequential sections:

### 1. Entity tree groups
- Starts at offset `0x10` (after a 16-byte file header).
- Loops over scene entities derived from `ZTreeGroup` (`0x2003e9` / `0x100020`).
- Each group has **5 tree slots**:
  - **Empty slot** (`size == 0`): 16 bytes (`size: u32 = 0`, plus 12 padding bytes).
  - **Non-empty slot** (`size > 0`):
    - `+0x00`: `u32 size` (payload size).
    - `+0x04`: 28-byte outer bounding header (`scale`, `origin` xyz, extra padding).
    - `+0x20`: payload of `size` bytes.
    - Slot advances by `32 + ((size + 15) & !15)`.

### 2. Octree payload layout
Both entity trees and global trees decode their payload (`+0x20` onwards) with `ZTree::Load`:
- `+0x00`: `u32 poly_table_rel` (byte offset from payload start to polygon table; always 16-byte aligned).
- `+0x04`: `[f32; 3] origin` (world origin in cm).
- `+0x10`: `f32 inv_scale` (scale factor `scale = 1.0 / inv_scale`).
- `+0x14`: **Node table** (`poly_table_rel - 0x14` bytes, 6 bytes per node, padded to 16 bytes before poly table):
  - `+0x00`: `u16 flags_and_count` (`(fc >> 3) & 0xfff` polygon count, `fc & 7` octant mask).
  - `+0x02`: `u16 child_index` (index of first child node in node table; 0 = leaf node).
  - `+0x04`: `u16 poly_start` (start index in polygon table).
- `+poly_table_rel`: **Polygon table** (`(payload.len() - poly_table_rel) / 16` entries, 16 bytes each):
  - `+0x00`: `u32 poly_ref` (packed PRM submesh index and triangle index).
  - `+0x04`: `[u16; 3] min_q` (quantized min XYZ centered at 32768).
  - `+0x0a`: `[u16; 3] max_q` (quantized max XYZ centered at 32768).
  - Quantization formula: `world = origin + (q - 32768.0) * scale`.

### 3. Global scene octrees
- Follows the entity tree groups.
- Each entry has `u32 total_size` followed by a 28-byte header and octree payload at `+0x20`.
- Advances directly by `total_size` bytes per chunk until `size == 0xffffffff`.

### 4. Spatial link table
- Extends from the `0xffffffff` terminator to EOF.
- Array of **8-byte records**: `u32 poly_ref, u32 primitive_id/offset`.
- Maps octree polygon references to concrete PRM submesh models for exact triangle vertex lookup.

## Not yet decoded

- `AMS`, `SND`, `ZGF`/`SGP`/`RMC`/`RMI`/`SUP`, light colour/radius, and the `.sdl` scripts that
  drive AI and cutscenes.
