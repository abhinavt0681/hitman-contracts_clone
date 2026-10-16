//! Glacier 1 compiled scripts (`.ppo`): the `.sdl` programs that drive AI, cutscenes and
//! level logic, stored back to back in a scene's `_MAIN.ZGF`.
//!
//! Reverse-engineered from `HitmanContracts.exe` (see `docs/SCRIPTS.md`):
//! - `.ppo` loader: magic `PPO!` checked at 0x006607fd / 0x00664b95.
//! - VM image loader: `FUN_00679410`, magic `IIMV`, version 1.1.
//! - Interpreter: `FUN_0066a2b0`, one `switch` over a one-byte opcode (0x00–0xA1).
//!
//! A `.ppo` is: a 32-byte header, the property schema (`LONG m_iHitpoints=20;…`), a block of
//! defaults, then the `IIMV` image. The image holds the source names of the script and its
//! parents, entry/state tables, the import name table and finally the bytecode. The VM is a
//! stack machine over 8-byte tagged values (1 int, 2 float, 3 code address, 4 object);
//! variable operands are u16: bit 15 set = slot (bit 14: frame-relative, else global),
//! clear = byte offset into the object's own data.

use super::ReadLe;
use anyhow::{Context, Result, bail, ensure};

/// One compiled script.
pub struct Script {
    /// Byte offset of the `PPO!` header in the container.
    pub at: usize,
    /// `LONG m_iHitpoints=20;…`: the properties the level editor exposes.
    pub schema: String,
    /// Source file of this script.
    pub source: String,
    /// Source file of the script it derives from (`Guard` derives from `Armed`, …).
    pub parent_source: Option<String>,
    /// Image section offsets (`IIMV` header fields +0x64..+0x8c).
    pub sections: [u32; 11],
    pub image: Vec<u8>,
    /// Names the bytecode refers to by index (classes, methods, constants). Indices are
    /// global along the inheritance chain: this script's start at `name_base`.
    pub names: Vec<String>,
    pub name_base: u32,
    /// The state machine: each state's name and code.
    pub states: Vec<State>,
    /// Entry points that `call_table`, `set_state_name` and `goto_state_name` index
    /// (section `+0x68`): absolute VM addresses.
    pub functions: Vec<u32>,
    /// The bytecode.
    pub code: Vec<u8>,
    /// Offset of `code` in `image`.
    pub code_at: usize,
}

/// All scripts in a `.ZGF` (inflated) container.
pub fn scripts(zgf: &[u8]) -> Vec<Result<Script>> {
    let magic = b"PPO! \0\0\0";
    (0..zgf.len().saturating_sub(8)).filter(|&i| &zgf[i..i + 8] == magic).map(|at| Script::parse(zgf, at)).collect()
}

impl Script {
    pub fn parse(data: &[u8], at: usize) -> Result<Self> {
        let schema_end = data.u32_at(at + 0x18)? as usize;
        let iimv = at + data.u32_at(at + 0x1c)? as usize;
        ensure!(data.get(iimv..iimv + 4) == Some(b"IIMV"), "no IIMV image at {iimv:#x}");
        let schema = data.cstr_at(at + 0x20).unwrap_or("").to_owned();
        ensure!(at + schema_end <= iimv, "schema overruns image");
        let mut sections = [0u32; 11];
        for (k, s) in sections.iter_mut().enumerate() {
            *s = data.u32_at(iimv + 0x54 + 4 * k)?;
        }
        let image_len = data.u32_at(iimv + 0x80)? as usize;
        let image = data.get(iimv + 0x84..iimv + 0x84 + image_len).context("image out of bounds")?.to_vec();

        // The image opens with the script's own source name; the parent's is at section 0.
        let source = image.cstr_at(0).unwrap_or("?").to_owned();
        let parent_source = image.cstr_at(sections[0] as usize).ok().filter(|s| !s.is_empty()).map(str::to_owned);

        let code_at = sections[10] as usize;
        let code_len = image.u32_at(code_at)? as usize;
        let code = image.get(code_at + 4..code_at + 4 + code_len).context("code out of bounds")?.to_vec();
        let (name_base, names) = find_name_table(&image, &sections).unwrap_or_default();
        let states = parse_states(&image, sections[2] as usize).unwrap_or_default();
        let f = sections[1] as usize;
        let functions = (0..image.u32_at(f).unwrap_or(0) as usize).filter_map(|k| image.u32_at(f + 4 + 4 * k).ok()).collect();
        Ok(Script { at, schema, source, parent_source, sections, image, names, name_base, states, functions, code, code_at: code_at + 4 })
    }
}

/// One state of a script's state machine.
#[derive(Clone, Debug)]
pub struct State {
    /// Global name index (`Guard.FollowHitman.lostHim`).
    pub name: u16,
    /// Absolute VM address where the state starts.
    pub entry: u32,
    /// Three handler addresses (`u32::MAX` = none); the third usually follows the entry.
    pub handlers: [u32; 3],
}

/// State table, image section `+0x6c` (`FUN_00677f80`): `0x11223344`, size, then
/// `[2] ?`, `[3] n states`, `[4] ?`, `[5] d`, `[6] ?`, `[7] e`, followed by arrays: entries
/// `[n]`, state ids `[n]`, offsets `[d]`, handlers `[3n]`, then u16 data whose first `n`
/// entries are the states' name indices.
fn parse_states(image: &[u8], at: usize) -> Option<Vec<State>> {
    if image.u32_at(at).ok()? != 0x1122_3344 {
        return None;
    }
    let n = image.u32_at(at + 12).ok()? as usize;
    let d = image.u32_at(at + 20).ok()? as usize;
    let entries = at + 32;
    let handlers = entries + 8 * n + 4 * d;
    let names = handlers + 12 * n;
    (0..n)
        .map(|i| {
            Some(State {
                name: image.u16_at(names + 2 * i).ok()?,
                entry: image.u32_at(entries + 4 * i).ok()?,
                handlers: [0, 1, 2].map(|k| image.u32_at(handlers + 12 * i + 4 * k).unwrap_or(u32::MAX)),
            })
        })
        .collect()
}

/// A level's scripts linked into their inheritance chains, which is how the VM sees them:
/// name indices and code addresses run on from the parent's.
pub struct Program {
    pub scripts: Vec<Script>,
    /// Index of each script's parent.
    pub parent: Vec<Option<usize>>,
    /// Absolute VM address of each script's first code byte.
    pub code_base: Vec<u32>,
}

/// Lower-cased path components, to compare `..\..\AllLevels\Std\Man.sdl` with `Man.sdl`.
fn path_parts(p: &str) -> Vec<String> {
    p.split(['\\', '/']).filter(|c| !c.is_empty() && *c != "..").map(str::to_lowercase).collect()
}

impl Program {
    pub fn link(scripts: Vec<Script>) -> Self {
        let parts: Vec<Vec<String>> = scripts.iter().map(|s| path_parts(&s.source)).collect();
        let parent: Vec<Option<usize>> = scripts
            .iter()
            .map(|s| {
                let want = path_parts(s.parent_source.as_deref()?);
                // Same file name; among several, the one sharing the most trailing components.
                (0..scripts.len())
                    .filter(|&k| parts[k].last() == want.last())
                    .max_by_key(|&k| parts[k].iter().rev().zip(want.iter().rev()).take_while(|(a, b)| a == b).count())
            })
            .collect();
        let mut code_base = vec![0u32; scripts.len()];
        for k in 0..scripts.len() {
            let (mut base, mut at) = (0u32, parent[k]);
            let mut guard = 0;
            while let Some(p) = at {
                base += scripts[p].code.len() as u32;
                at = parent[p];
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
            code_base[k] = base;
        }
        Program { scripts, parent, code_base }
    }

    /// Name `index` as seen from script `k`: its own table, else up the chain.
    pub fn name(&self, k: usize, index: u16) -> String {
        let mut at = Some(k);
        while let Some(s) = at {
            let script = &self.scripts[s];
            let i = index as i64 - script.name_base as i64;
            if i >= 0 && (i as usize) < script.names.len() {
                return script.names[i as usize].clone();
            }
            at = self.parent[s];
        }
        format!("name#{index}")
    }

    /// Entry `index` of script `k`'s function table (its whole inherited dispatch table): the
    /// state starting there if any, else the address.
    pub fn function(&self, k: usize, index: u16) -> String {
        let Some(&addr) = self.scripts[k].functions.get(index as usize) else { return format!("func#{index}") };
        let mut at = Some(k);
        while let Some(s) = at {
            if let Some(st) = self.scripts[s].states.iter().find(|st| st.entry == addr) {
                return format!("state {}", self.name(s, st.name));
            }
            at = self.parent[s];
        }
        format!("func#{index} {}", self.label(k, addr))
    }

    /// A VM code address as seen from script `k`: a local label, or `Script@offset` up the chain.
    pub fn label(&self, k: usize, addr: u32) -> String {
        let mut at = Some(k);
        while let Some(s) = at {
            let base = self.code_base[s];
            if addr >= base && addr < base + self.scripts[s].code.len() as u32 {
                let off = addr - base;
                return if s == k { format!("L{off:04x}") } else { format!("{}@{off:04x}", short_name(&self.scripts[s].source)) };
            }
            at = self.parent[s];
        }
        format!("?{addr:#x}")
    }
}

/// `..\AllLevels\Std\Guard.sdl` → `Guard`.
pub fn short_name(source: &str) -> &str {
    source.rsplit(['\\', '/']).next().unwrap_or(source).trim_end_matches(".sdl")
}

/// The name table, image section `+0x78`: `u32 base`, `u32 count`, `count` offsets, then a
/// blob of NUL-terminated names sorted alphabetically (the loader keeps `p[0]`, `p[1]`,
/// `p + 2` and `p + 2 + count`). `base` is the global index of the first name.
fn find_name_table(image: &[u8], sections: &[u32; 11]) -> Option<(u32, Vec<String>)> {
    let s = sections[5] as usize;
    let base = image.u32_at(s).ok()?;
    let count = image.u32_at(s + 4).ok()? as usize;
    let blob = s + 8 + 4 * count;
    // Names are mostly ASCII; dialogue paths may carry Latin-1 accents.
    let latin1 = |at: usize| -> Option<String> {
        let tail = image.get(at..)?;
        let end = tail.iter().position(|&c| c == 0)?;
        Some(tail[..end].iter().map(|&c| c as char).collect())
    };
    let names = (0..count).map(|k| image.u32_at(s + 8 + 4 * k).ok().and_then(|o| latin1(blob + o as usize))).collect::<Option<Vec<_>>>()?;
    Some((base, names))
}

/// How an opcode's operands are laid out after it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Operands {
    None,
    /// u16 variable.
    Var,
    /// u16 variable, u16 variable.
    VarVar,
    /// u16 variable, i32 immediate.
    VarInt,
    /// u16 variable, f32 immediate.
    VarFloat,
    Int,
    Float,
    /// u32 code address.
    Addr,
    /// u32 code address, u32 mode.
    AddrMode,
    /// u16 name index.
    Name,
    /// u16 function table index.
    Func,
    /// NUL-terminated string.
    Str,
    /// i32, then u32 code address.
    IntAddr,
    /// u16 class name, u16 method name, u16 signature, u32 argument/return counts.
    Call,
    /// u16 object variable, u16 method name, u16 signature, u32 counts.
    MethodCall,
    /// u32 count, u32 default address, then `count` (i32 value, u32 address).
    SwitchInt,
    /// u32 count, u32 default address, then `count` (f32 bound, u32 address).
    SwitchFloat,
    /// u32 element size, u32 offset, u32 array variable.
    Array,
    /// u16 function table index, i32.
    FuncInt,
    /// Raw bytes of a fixed size.
    Bytes(usize),
}

/// Mnemonic and operand layout of every opcode the interpreter accepts.
pub fn opcode(op: u8) -> Option<(&'static str, Operands)> {
    use Operands::*;
    Some(match op {
        0x00 | 0x94 | 0xa0 => ("nop", None),
        0x01 => ("inc", Var),
        0x02 => ("dec", Var),
        0x03 => ("not", None),
        0x04 => ("and", None),
        0x05 => ("or", None),
        0x06 => ("fand", None),
        0x07 => ("for", None),
        0x08 => ("bnot", None),
        0x09 => ("band", None),
        0x0a => ("bor", None),
        0x0b => ("ineg", None),
        0x0c => ("fneg", None),
        0x0d => ("fuzzy_not", None),
        0x0e => ("fadd", None),
        0x0f => ("iadd", None),
        0x10 => ("fsub", None),
        0x11 => ("isub", None),
        0x12 => ("fmul", None),
        0x13 => ("imul", None),
        0x14 => ("fdiv", None),
        0x15 => ("idiv", None),
        0x16 => ("fadd_var_imm", VarFloat),
        0x17 => ("iadd_var_imm", VarInt),
        0x18 => ("fsub_var_imm", VarFloat),
        0x19 => ("isub_var_imm", VarInt),
        0x1a => ("fmul_var_imm", VarFloat),
        0x1b => ("imul_var_imm", VarInt),
        0x1c => ("fdiv_var_imm", VarFloat),
        0x1d => ("idiv_var_imm", VarInt),
        0x1e => ("fadd_ref_imm", Float),
        0x1f => ("iadd_ref_imm", Int),
        0x20 => ("fsub_ref_imm", Float),
        0x21 => ("isub_ref_imm", Int),
        0x22 => ("fmul_ref_imm", Float),
        0x23 => ("imul_ref_imm", Int),
        0x24 => ("fdiv_ref_imm", Float),
        0x25 => ("idiv_ref_imm", Int),
        0x26 => ("fmax", None),
        0x27 => ("fmin", None),
        0x28 => ("op28_var", VarInt),
        0x29 => ("op29_var", VarInt),
        0x2a => ("push_float", Float),
        0x2b | 0x97 => ("push_int", Int),
        0x2c => ("load_int_ref", None),
        0x2d => ("load_float_ref", None),
        0x2e => ("load_float", Var),
        0x2f | 0x8a => ("load_int", Var),
        0x30 => ("op30_var", Var),
        0x31 => ("op31_var", Var),
        0x32 => ("store_float", Var),
        0x33 => ("store_int", Var),
        0x34 => ("op34_var", Var),
        0x35 => ("op35_var", Var),
        0x36 => ("op36_var", Var),
        0x37 => ("op37_var", Var),
        0x38 => ("op38_var", Var),
        0x39 => ("op39_var", Var),
        0x3a => ("op3a_var", Var),
        0x3b => ("op3b_var", Var),
        0x3c => ("op3c_var", Var),
        0x3d => ("op3d_var", Var),
        0x3e => ("op3e_var", Var),
        0x3f => ("op3f_var", Var),
        0x40 => ("fadd_ref", None),
        0x41 => ("iadd_ref", None),
        0x42 => ("fsub_ref", None),
        0x43 => ("isub_ref", None),
        0x44 => ("fmul_ref", None),
        0x45 => ("imul_ref", None),
        0x46 => ("fdiv_ref", None),
        0x47 => ("idiv_ref", None),
        0x48 => ("fmin_ref", None),
        0x49 => ("fmax_ref", None),
        0x4a | 0x78 => ("call", Addr),
        0x4b => ("call_func", Func),
        0x4c => ("call_var", Var),
        0x4d => ("ret", None),
        0x4e => ("pop_n", Int),
        0x4f => ("jmp", Addr),
        0x50 => ("jmp_if", AddrMode),
        0x51 => ("jmp_ifnot", AddrMode),
        0x52 => ("wait_until", Addr),
        0x53 => ("set_state", Int),
        0x54 => ("set_state_var", Int),
        0x55 => ("set_state_func", Func),
        0x56 => ("ilt", None),
        0x57 => ("igt", None),
        0x58 => ("ige", None),
        0x59 => ("ile", None),
        0x5a => ("ieq", None),
        0x5b => ("ine", None),
        0x5c => ("flt", None),
        0x5d => ("fgt", None),
        0x5e => ("fge", None),
        0x5f => ("fle", None),
        0x60 => ("feq", None),
        0x61 => ("fne", None),
        0x62 => ("store_ref_int_imm", Int),
        0x63 => ("store_ref_float_imm", Float),
        0x64 => ("store_ref_int", None),
        0x65 => ("store_ref_float", None),
        0x66 => ("set_float", VarFloat),
        0x67 => ("set_int", VarInt),
        0x68 => ("copy_int", VarVar),
        0x69 => ("copy_float", VarVar),
        0x6a => ("break", None),
        0x6b => ("int_to_float", Int),
        0x6c => ("swap", None),
        0x6d => ("switch", SwitchInt),
        0x6e => ("switch_float", SwitchFloat),
        0x6f => ("sleep", None),
        0x70 => ("sleep_imm", Bytes(8)),
        0x71 => ("wait_event", None),
        0x72 => ("wait_named_event", Str),
        0x73 => ("budget", Int),
        0x74 => ("jmp_set", IntAddr),
        0x75 => ("set_start_state", Int),
        0x76 => ("jmp_flag", Addr),
        0x77 => ("jmp_noflag", Addr),
        0x79 => ("state_end", None),
        0x7a => ("goto_state", IntAddr),
        0x7b => ("goto_state_func", FuncInt),
        0x7c => ("halt", None),
        0x7d | 0xa1 => ("yield", None),
        0x7e => ("op7e", Int),
        0x7f => ("index", Array),
        0x80 => ("push_time", None),
        0x81 => ("op81_var", Var),
        0x82 => ("op82", None),
        0x83 => ("push_var", Var),
        0x84 => ("push_ref", None),
        0x85 => ("push_default", Str),
        0x88 => ("cast", Str),
        0x89 => ("fuzzy_clamp", None),
        0x8b => ("iadd_var_var", VarVar),
        0x8c => ("isub_var_var", VarVar),
        0x8d => ("imul_var_var", VarVar),
        0x8e => ("idiv_var_var", VarVar),
        0x8f => ("fadd_var_var", VarVar),
        0x90 => ("fsub_var_var", VarVar),
        0x91 => ("fmul_var_var", VarVar),
        0x92 => ("fdiv_var_var", VarVar),
        0x93 => ("frame", Int),
        0x95 => ("op95", None),
        0x96 => ("budget_set", Int),
        0x98 => ("call_native", Call),
        0x99 => ("call_method", MethodCall),
        0x9a => ("push_name", Name),
        0x9b => ("deref", None),
        0x9c => ("op9c", None),
        0x9d => ("op9d", None),
        0x9e => ("lock", None),
        0x9f => ("unlock", None),
        _ => return Option::None,
    })
}

/// One decoded instruction.
pub struct Instr {
    /// Offset in the code.
    pub at: usize,
    pub op: u8,
    pub mnemonic: &'static str,
    /// Operands, rendered.
    pub args: String,
    /// Code addresses this instruction may jump or call to (absolute VM addresses).
    pub targets: Vec<u32>,
    pub len: usize,
}

fn var(v: u16) -> String {
    if v & 0x8000 != 0 {
        if v & 0x4000 != 0 { format!("frame[{}]", v as i16 as i32 & !0) } else { format!("global[{}]", !(v | 0x4000)) }
    } else {
        format!("this+{v:#x}")
    }
}

/// Decodes the instruction at `pc` of script `k`.
pub fn decode(program: &Program, k: usize, pc: usize) -> Result<Instr> {
    let script = &program.scripts[k];
    let name = |i: u16| program.name(k, i);
    let label = |a: u32| program.label(k, a);
    let c = &script.code;
    let op = *c.get(pc).context("pc out of code")?;
    let Some((mnemonic, operands)) = opcode(op) else { bail!("invalid opcode {op:#04x} at {pc:#x}") };
    let a = pc + 1;
    let mut targets = Vec::new();
    let (args, len) = match operands {
        Operands::None => (String::new(), 0),
        Operands::Var => (var(c.u16_at(a)?), 2),
        Operands::VarVar => (format!("{}, {}", var(c.u16_at(a)?), var(c.u16_at(a + 2)?)), 4),
        Operands::VarInt => (format!("{}, {}", var(c.u16_at(a)?), c.u32_at(a + 2)? as i32), 6),
        Operands::VarFloat => (format!("{}, {}", var(c.u16_at(a)?), c.f32_at(a + 2)?), 6),
        Operands::Int => (format!("{}", c.u32_at(a)? as i32), 4),
        Operands::Float => (format!("{}", c.f32_at(a)?), 4),
        Operands::Addr => {
            let t = c.u32_at(a)?;
            targets.push(t);
            (label(t), 4)
        }
        Operands::AddrMode => {
            let t = c.u32_at(a)?;
            targets.push(t);
            (format!("{}, {}", label(t), c.u32_at(a + 4)? as i32), 8)
        }
        Operands::Name => (format!("\"{}\"", name(c.u16_at(a)?)), 2),
        Operands::Func => (program.function(k, c.u16_at(a)?), 2),
        Operands::Str => {
            let s = c.cstr_at(a).unwrap_or("?");
            (format!("{s:?}"), s.len() + 1)
        }
        Operands::IntAddr => {
            let t = c.u32_at(a)?;
            targets.push(t);
            (format!("{}, {}", label(t), c.u32_at(a + 4)? as i32), 8)
        }
        Operands::Call => {
            let (class, method, sig, counts) = (c.u16_at(a)?, c.u16_at(a + 2)?, c.u16_at(a + 4)?, c.u32_at(a + 6)?);
            (format!("{}::{} args={} ret={} flags={:#x} sig#{sig}", name(class), name(method), counts & 0xff, (counts >> 8) & 0xff, counts >> 16), 10)
        }
        Operands::MethodCall => {
            let (obj, method, sig, counts) = (c.u16_at(a)?, c.u16_at(a + 2)?, c.u16_at(a + 4)?, c.u32_at(a + 6)?);
            (format!("{}.{} args={} ret={} flags={:#x} sig#{sig}", var(obj), name(method), counts & 0xff, (counts >> 8) & 0xff, counts >> 16), 10)
        }
        Operands::SwitchInt | Operands::SwitchFloat => {
            let n = c.u32_at(a)? as usize;
            let default = c.u32_at(a + 4)?;
            ensure!(n < 10_000, "switch with {n} cases at {pc:#x}");
            let mut s = format!("default {}", if default == 0xffff_ffff { "-".to_owned() } else { label(default) });
            if default != 0xffff_ffff {
                targets.push(default);
            }
            for k in 0..n {
                let (v, t) = (a + 8 + 8 * k, c.u32_at(a + 12 + 8 * k)?);
                targets.push(t);
                if operands == Operands::SwitchInt {
                    s += &format!(", {} => {}", c.u32_at(v)? as i32, label(t));
                } else {
                    s += &format!(", <{} => {}", c.f32_at(v)?, label(t));
                }
            }
            (s, 8 + 8 * n)
        }
        Operands::Array => (format!("size={} offset={} array={}", c.u32_at(a)?, c.u32_at(a + 4)?, var(c.u32_at(a + 8)? as u16)), 12),
        Operands::FuncInt => (format!("{}, {}", program.function(k, c.u16_at(a)?), c.u32_at(a + 2)? as i32), 6),
        Operands::Bytes(n) => (c.get(a..a + n).context("operand out of code")?.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "), n),
    };
    ensure!(a + len <= c.len(), "operands of {mnemonic} at {pc:#x} run past the code");
    Ok(Instr { at: pc, op, mnemonic, args, targets, len: 1 + len })
}

/// Linear disassembly of script `k`'s code; stops at the first undecodable byte.
pub fn disassemble(program: &Program, k: usize) -> (Vec<Instr>, Option<anyhow::Error>) {
    let mut out = Vec::new();
    let mut pc = 0;
    while pc < program.scripts[k].code.len() {
        match decode(program, k, pc) {
            Ok(i) => {
                pc += i.len;
                out.push(i);
            }
            Err(e) => return (out, Some(e)),
        }
    }
    (out, None)
}
