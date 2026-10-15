//! `.ANM` animation packs. Layout recovered from `ZAnimFilePack` in HitmanContracts.exe
//! (vtable 0x72D0C4; loader 0x61C0A0, sampler 0x61B930).
//!
//! The file is a tree of chunks: `u32 type, u32 size|flags`. Bit 31 marks a container
//! (`u32 data_offset` at +8, child count at +0xC, children at +0x10 or, with bit 30, after a
//! `u32 n` + n×u32 index at +0x10). With either top bit set the payload is at
//! `chunk + [chunk+8]`, otherwise at `chunk + 8`. The root (type `"ANM\0"`) holds:
//!
//! - type 0x0A, one per animation set: payload = set path (`…\Hero\Hero.anm`); first child is
//!   `u32 count` + `count` × offsets of animation records in chunk 5.
//! - type 0x04: strings. Bone names are stored as `u8 channel` + name.
//! - type 0x05: animation records and their track tables.
//! - type 0x06: bit-packed key tables. 0x07: rotation keys. 0x08: position keys.
//! - type 0x0B: static positions (f32×3). 0x0D: static rotations (f32×4).
//!
//! Animation record (40 bytes): `+0x14 u32` track table (chunk 5), `+0x24 u32` name (chunk 4).
//! Track table: `u32 n`, then n×u32 offsets (from the start of chunk 5) of packed tracks:
//!
//! - header: `u16 bone_name` (chunk 4), `u16 0`, `u16 static_pos`, `u16 static_rot`,
//!   `u16 frames`, `u16 flags` — 0x1 rotations as 4×i16 (÷32512), 0x2 as 4×i8 (÷127),
//!   0x4 keyed positions, 0x8 static position, 0x10 static rotation
//! - unless 0x10: `u32 keys` (chunk 7), `u32 frame_to_key`, `u32 key_to_frame` (chunk 6)
//! - if 0x4 and not 0x8: the same three for positions (keys in chunk 8, 12 bytes each)
//!
//! Key tables start with an `i8` bit width, then MSB-first packed values. Key k sits at frame
//! `k == 0 ? 0 : key_to_frame[k-1]`; the last key is at `frames`. Rotations replace the bone's
//! local rotation; positions its local translation. 30 frames per second.

use super::ReadLe;
use anyhow::{Context, Result, bail};
use std::collections::HashMap;

pub const FPS: f32 = 30.0;
const MAX_KEYS: usize = 4096;

pub struct Track {
    pub bone: String,
    pub channel: u8,
    /// (frame, xyzw) sorted by frame; one entry for static rotations.
    pub rotations: Vec<(f32, [f32; 4])>,
    /// (frame, xyz); empty if the track never sets a position.
    pub positions: Vec<(f32, [f32; 3])>,
}

pub struct Animation {
    pub name: String,
    pub frames: u16,
    pub tracks: Vec<Track>,
}

impl Animation {
    pub fn duration(&self) -> f32 {
        self.frames as f32 / FPS
    }
}

pub struct AnimSet {
    pub path: String,
    pub animations: HashMap<String, Animation>,
}

#[derive(Default)]
pub struct AnmFile {
    pub sets: Vec<AnimSet>,
}

struct Chunks {
    strings: usize,
    records: usize,
    bits: usize,
    rot_keys: usize,
    pos_keys: usize,
    static_pos: usize,
    static_rot: usize,
}

fn payload(d: &[u8], c: usize) -> Result<usize> {
    let sf = d.u32_at(c + 4)?;
    Ok(if sf & 0xc000_0000 != 0 { c + d.u32_at(c + 8)? as usize } else { c + 8 })
}

fn children(d: &[u8], c: usize) -> Result<Vec<usize>> {
    let sf = d.u32_at(c + 4)?;
    if sf & 0x8000_0000 == 0 {
        return Ok(Vec::new());
    }
    let n = d.u32_at(c + 0xc)? as usize;
    let mut k = if sf & 0x4000_0000 != 0 { c + 0x14 + d.u32_at(c + 0x10)? as usize * 4 } else { c + 0x10 };
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(k);
        let size = (d.u32_at(k + 4)? & 0x3fff_ffff) as usize;
        if size == 0 {
            bail!("zero-sized ANM chunk at {k:#x}");
        }
        k += size;
    }
    Ok(out)
}

impl AnmFile {
    pub fn parse(d: &[u8]) -> Result<Self> {
        if d.get(0..3) != Some(b"MNA") {
            bail!("not an ANM file");
        }
        let kids = children(d, 0)?;
        let find = |t: u32| -> Result<usize> {
            for &k in &kids {
                if d.u32_at(k)? == t {
                    return payload(d, k);
                }
            }
            bail!("ANM chunk type {t:#x} missing")
        };
        let c = Chunks {
            strings: find(0x4)?,
            records: find(0x5)?,
            bits: find(0x6)?,
            rot_keys: find(0x7)?,
            pos_keys: find(0x8)?,
            static_pos: find(0xb)?,
            static_rot: find(0xd)?,
        };

        let mut sets = Vec::new();
        for &k in &kids {
            if d.u32_at(k)? != 0xa {
                continue;
            }
            let path = d.cstr_at(payload(d, k)?)?.to_owned();
            let list = k + 0x10;
            let count = d.u32_at(list)? as usize;
            let mut animations = HashMap::new();
            for i in 0..count {
                let rec = c.records + d.u32_at(list + 8 + i * 4)? as usize;
                let anim = read_animation(d, &c, rec)?;
                animations.insert(anim.name.clone(), anim);
            }
            sets.push(AnimSet { path, animations });
        }
        Ok(Self { sets })
    }

    /// The set whose path ends with `file_name` (e.g. `"Hero.anm"`).
    pub fn set(&self, file_name: &str) -> Option<&AnimSet> {
        self.sets.iter().find(|s| s.path.rsplit('\\').next().is_some_and(|f| f.eq_ignore_ascii_case(file_name)))
    }
}

fn read_bits(d: &[u8], table: usize, i: usize) -> Result<usize> {
    let w = d.get(table).copied().unwrap_or(0) as i8 as i32;
    if w <= 0 || w > 24 {
        bail!("bad key table width {w} at {table:#x}");
    }
    let bit = i * w as usize;
    let p = table + 1 + (bit >> 3);
    let b = d.get(p..p + 3).unwrap_or(&[0, 0, 0]);
    let v = ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8);
    Ok(((v >> (32 - (bit & 7) as i32 - w)) & ((1u32 << w) - 1)) as usize)
}

/// Frame numbers of every key in a stream, ending with the key at `frames`.
fn key_frames(d: &[u8], key_to_frame: usize, frames: usize) -> Result<Vec<usize>> {
    let mut out = vec![0];
    while *out.last().unwrap() < frames {
        let f = read_bits(d, key_to_frame, out.len() - 1)?;
        // Some face tracks point at the shared all-zero table: a constant, held from frame 0.
        if f <= *out.last().unwrap() {
            break;
        }
        if out.len() > MAX_KEYS {
            bail!("runaway key table {key_to_frame:#x} (frames {frames})");
        }
        out.push(f);
    }
    Ok(out)
}

fn read_animation(d: &[u8], c: &Chunks, rec: usize) -> Result<Animation> {
    let name = d.cstr_at(c.strings + d.u32_at(rec + 0x24)? as usize)?.to_owned();
    let table = c.records + d.u32_at(rec + 0x14)? as usize;
    let n = d.u32_at(table)? as usize;
    if n > 256 {
        bail!("{name}: implausible track count {n}");
    }
    let mut tracks = Vec::with_capacity(n);
    let mut frames = 0;
    for i in 0..n {
        let t = c.records + d.u32_at(table + 4 + i * 4)? as usize;
        let name_at = c.strings + d.u16_at(t)? as usize;
        let channel = d.get(name_at).copied().unwrap_or(0);
        let bone = d.cstr_at(name_at + 1)?.to_owned();
        let static_pos = d.u16_at(t + 4)? as usize;
        let static_rot = d.u16_at(t + 6)? as usize;
        let nframes = d.u16_at(t + 8)? as usize;
        let flags = d.u16_at(t + 10)?;
        frames = frames.max(nframes);
        let mut p = t + 12;

        let rotations = if flags & 0x10 != 0 {
            let r = c.static_rot + static_rot * 16;
            vec![(0.0, [d.f32_at(r)?, d.f32_at(r + 4)?, d.f32_at(r + 8)?, d.f32_at(r + 12)?])]
        } else {
            let keys = c.rot_keys + d.u32_at(p)? as usize;
            let k2f = c.bits + d.u32_at(p + 8)? as usize;
            p += 12;
            let wide = flags & 0x1 != 0;
            key_frames(d, k2f, nframes)
                .with_context(|| format!("{name}/{bone} rotations"))?
                .into_iter()
                .enumerate()
                .map(|(k, f)| {
                    let q = if wide {
                        let o = keys + k * 8;
                        [0, 2, 4, 6].map(|i| d.u16_at(o + i).map(|v| v as i16 as f32 / 32512.0).unwrap_or(0.0))
                    } else {
                        let o = keys + k * 4;
                        [0, 1, 2, 3].map(|i| d.get(o + i).map_or(0.0, |&v| v as i8 as f32 / 127.0))
                    };
                    (f as f32, q)
                })
                .collect()
        };

        let positions = if flags & 0x8 != 0 {
            let s = c.static_pos + static_pos * 12;
            vec![(0.0, [d.f32_at(s)?, d.f32_at(s + 4)?, d.f32_at(s + 8)?])]
        } else if flags & 0x4 != 0 {
            let keys = c.pos_keys + d.u32_at(p)? as usize;
            let k2f = c.bits + d.u32_at(p + 8)? as usize;
            key_frames(d, k2f, nframes)
                .with_context(|| format!("{name}/{bone} positions"))?
                .into_iter()
                .enumerate()
                .map(|(k, f)| {
                    let o = keys + k * 12;
                    Ok((f as f32, [d.f32_at(o)?, d.f32_at(o + 4)?, d.f32_at(o + 8)?]))
                })
                .collect::<Result<_>>()?
        } else {
            Vec::new()
        };

        tracks.push(Track { bone, channel, rotations, positions });
    }
    Ok(Animation { name, frames: frames as u16, tracks })
}

/// Linear position / normalized-lerp rotation sample at `frame` (keys are dense enough).
pub fn sample_rot(keys: &[(f32, [f32; 4])], frame: f32) -> [f32; 4] {
    sample(keys, frame, |a, b, x| {
        // Take the short way round.
        let dot: f32 = (0..4).map(|i| a[i] * b[i]).sum();
        let s = if dot < 0.0 { -1.0 } else { 1.0 };
        let mut q = [0.0; 4];
        for i in 0..4 {
            q[i] = a[i] + (b[i] * s - a[i]) * x;
        }
        let len = q.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-6);
        q.map(|v| v / len)
    })
}

pub fn sample_pos(keys: &[(f32, [f32; 3])], frame: f32) -> [f32; 3] {
    sample(keys, frame, |a, b, x| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * x))
}

fn sample<T: Copy>(keys: &[(f32, T)], frame: f32, lerp: impl Fn(T, T, f32) -> T) -> T {
    let i = keys.partition_point(|(f, _)| *f <= frame);
    if i == 0 {
        return keys[0].1;
    }
    if i >= keys.len() {
        return keys[keys.len() - 1].1;
    }
    let (f0, a) = keys[i - 1];
    let (f1, b) = keys[i];
    lerp(a, b, ((frame - f0) / (f1 - f0).max(1e-6)).clamp(0.0, 1.0))
}
