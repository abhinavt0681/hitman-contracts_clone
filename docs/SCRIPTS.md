# Glacier 1 scripts (`.sdl` → `.ppo`)

The AI, cutscenes and level logic of Hitman: Contracts are `.sdl` scripts compiled to
bytecode (`.ppo`) for a stack VM in `HitmanContracts.exe`. A scene's scripts sit back to back
in `<ID>_MAIN.ZGF` (compressed like the GMS: `u32 size, u32, u8 stored?` then deflate).
`src/glacier/ppo.rs` decodes them; `glacier-dump <ID> --scripts <dir>` writes one listing per
script (for C02-1 into gitignored `data/re/scripts/`).

## Where in the exe (Ghidra, `data/re/ghidra/`)

| What | Address |
|---|---|
| `.ppo` loader (`PPO!` check) | `0x006607fd`, `0x00664b95` |
| `IIMV` image loader | `FUN_00679410` |
| Interpreter (one `switch`, opcodes 0x00–0xA1) | `FUN_0066a2b0` |
| Name lookup up the inheritance chain | `FUN_0067a090` |
| Function-table lookup | `FUN_006788a0` |
| Code address → module | `FUN_00669ff0` |
| State table loader (`0x11223344`) | `FUN_00677f80` |
| Change state | `FUN_00668dc0` |

Headless Ghidra setup: `~/tools/ghidra_12.1.4_PUBLIC`, JDK 21 in `~/tools/jdk-21*`, export
script `tools/ghidra/ExportAll.java` (decompiled C per 64 KB, `functions.tsv`,
`strings_xrefs.tsv`).

## File layout

`.ppo`: 32-byte header (`"PPO!"`, `0x20`, `f32 1.0`, defaults length, image length, `0x20`,
schema end, image offset), the property schema (`LONG m_iHitpoints=20;…`), a defaults block,
then the `IIMV` image:

- `+0x00` `"IIMV"`, `u32` size, `f32` version 1.1, `u32` date, `u32` time, 4×`u16`, 2×16 bytes,
  6×`u32`.
- `+0x54` eleven `u32` section offsets into the image, `+0x80` image length; image at `+0x84`.
- Image: own source name at 0. Sections:

| # | Field | Content |
|---|---|---|
| 0 | `+0x64` | parent script's source name (empty for roots) |
| 1 | `+0x68` | function table: `u32 n`, n absolute code addresses (the full inherited dispatch table) |
| 2 | `+0x6c` | state machine: `0x11223344`, size, ?, `n`, ?, `d`, ?, `e`; then entries `[n]`, state ids `[n]`, `[d]`, handlers `[3n]`, u16 data whose first `n` are state name indices, … |
| 3, 4 | `+0x70`, `+0x74` | signature tables (u16 pairs; not decoded) |
| 5 | `+0x78` | names: `u32 base`, `u32 n`, n offsets, NUL-terminated blob |
| 6, 7 | `+0x7c`, `+0x80` | object-typed members: `(u32 offset, u16 type name)` for construction |
| 9 | `+0x88` | ? |
| 10 | `+0x8c` | code: `u32 length`, bytecode |

After the image: a table of `u32` source line numbers.

## Linking

Scripts derive from one another (`Being → Man → Armed → Guard → Soldier`, …). Name indices and
code addresses are global along a chain: a script's names start at its `base`, and its code
starts after all its ancestors' code (Being at 0, Man at 0x37ef, …). Jumps and calls use these
absolute addresses; `call_func` / `set_state_func` / `goto_state_func` index the function table.

## The VM

Stack of 8-byte values: `u32` payload + type tag (1 int, 2 float, 3 code address, 4 object
with refcounting). Variable operands are `u16`: bit 15 clear = byte offset into the object's
data (`this+0x42c`); set = slot, bit 14 frame-relative, else global.

Opcodes (mnemonics as `ppo.rs` prints them):

- 0x03–0x15 logic and arithmetic (int/float pairs); 0x0D `fuzzy_not` = `1 − clamp(x)`;
  0x89 `fuzzy_clamp`.
- 0x16–0x25 `var op= imm`; 0x40–0x49 `*ref op= pop`; 0x8B–0x92 `var op= var`; 0x26/0x27 max/min.
- 0x2A/0x2B push float/int; 0x2C–0x2F, 0x83/0x84 loads; 0x32/0x33, 0x62–0x69 stores.
- 0x56–0x61 comparisons.
- 0x4F jump; 0x50/0x51 conditional (`addr, mode`); 0x76/0x77 flag jumps; 0x6D/0x6E switch tables.
- 0x4A/0x78 call address; 0x4B call function-table entry; 0x4D return; 0x93 frame alloc.
- 0x98 `call_native Class::Method`, 0x99 `call_method obj.Method` (u16 class/object, u16 name,
  u16 signature, `u32` arg/ret counts + flags); 0x9A push name; 0x85/0x88 typed default/cast.
- 0x53–0x55 set state; 0x75 start state; 0x7A/0x7B goto state; 0x79 state end.
- 0x6F/0x70 sleep; 0x71 wait for event; 0x72 wait for named event; 0x7C/0x7D halt/yield.

All 32 C02-1 scripts decode end to end with no unknown opcodes and every jump landing on an
instruction boundary.

## Not yet done

- Signature tables (3, 4), event handler tables in the state machine, handler slot meanings.
- Naming non-state functions; mapping schema properties to `this+` offsets.
- Turning stack code into structured pseudo-source (expressions, `if`/`while`).
