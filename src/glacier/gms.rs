//! `.GMS` scene description (the "geom" tree: every placed object in the level).
//!
//! On disk: `u32 uncompressed_size, u32 compressed_size, u8 is_raw`, then a raw DEFLATE
//! stream (no zlib header). All offsets below are into the inflated body.
//!
//! Body header: `+0x00 u32 entity_table`. The entity table is `u32 count` followed by
//! `count` × `(u32 packed, u32 ?)`, where `packed & 0xFFFFFF` is the entity's offset in
//! 4-byte words and the top byte holds tree info (bit 0 = group root, rest = depth delta).
//!
//! Entity record:
//! - `+0x00 u32 name` — offset into `.BUF` (NUL-terminated)
//! - `+0x04 u32 rotation` — offset of 3×3 f32 matrix applied to row vectors (`v·M`),
//!   determinant −1 because PRM vertices store X and Z swapped (see `view.rs`)
//! - `+0x08 u32 position` — offset of f32×3 world position, centimeters, Y up
//! - `+0x0C u32 primitive` — byte offset of the model's last sub-mesh record in `.PRM`
//!   (0 or a non-record offset for non-mesh entities)
//! - `+0x10 u32 type_id` — engine class id (e.g. 0x200002 static geometry)
//! - `+0x18 u32 flags`
//! - after `+0x20`, actors carry a property blob (see `parse_script_props`); path groups
//!   (class `0x080000C8`) point at their node list from `+0x20` (see `parse_path`)
//!
//! Entity references (`ZGEOMREF` values, path nodes) are `0x80000000 | (index + 1)`: 1-based
//! entity-table indices with a flag bit; 0 means "none".

use super::ReadLe;
use anyhow::{Context, Result, bail};
use std::io::Read;

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ScriptProps {
    pub script: String,
    pub hitpoints: i32,
    pub character_type: i32,
    pub waypoints: Option<usize>,
    pub weapon_template: Option<usize>,
    pub guard_point: Option<usize>,
    pub is_sentry: i32,
    pub sentry_radius: f32,
    pub relaxed_vis_range: f32,
    pub alert_vis_range: f32,
    pub wait_time: f32,
    pub int_props: HashMap<String, i32>,
    pub float_props: HashMap<String, f32>,
    pub ref_props: HashMap<String, usize>,
    pub str_props: HashMap<String, String>,
}

pub struct Entity {
    pub name: String,
    pub type_id: u32,
    pub flags: u32,
    pub primitive: u32,
    pub rotation: [[f32; 3]; 3],
    pub position: [f32; 3],
    pub tree_bits: u8,
    pub props: Option<ScriptProps>,
    /// For path groups (class `PATH_CLASS`): node entity indices in walking order.
    pub path: Vec<usize>,
    /// For cloth (class `CLOTH_CLASS`): its simulation settings.
    pub cloth: Option<ClothProps>,
    /// For sound emitters (class `SOUND_CLASS`): the sound, as an offset into the level's
    /// `.SND` table (see `Level::sound_name`).
    pub sound: Option<u32>,
}

/// Engine class of placed sound emitters (`enum Type={3D,…}; SndRef Sound; float
/// InnerConeAngle; float OuterConeAngle; float OuterConeVolume; float LPCutOffFreq; bool
/// AutoAttach; BOOL Loop; float Volume; float mindist; …`): tag `0x8b` + u32 at +9 of the
/// property block is `Sound`.
pub const SOUND_CLASS: u32 = 0x0020_0012;

/// Engine class `ZCloth`: a mesh simulated as cloth, pinned at its anchor boxes.
pub const CLOTH_CLASS: u32 = 0x0020_00f6;

/// The parts of `ZCloth`'s properties (`int MaxNumberVisible; int Iterations; enum UseWind;
/// double WindSpeed[3]; …; ZGEOMREFTAB AnchorBoxes; …; float Gravity=1500`) we use.
#[derive(Clone, Debug)]
pub struct ClothProps {
    /// Entities whose boxes hold the cloth in place.
    pub anchors: Vec<usize>,
    /// Wind the cloth feels, in the cloth's own units (relative strength).
    pub wind: [f32; 3],
}

/// Cloth property block (pointed to by the entity record at +0x20): tagged values, where
/// `0x89, u32 size, refs…` is the anchor table and `0x81 f64, 0x41 f64, 0x41 f64` the first
/// three-double array, `WindSpeed`.
fn parse_cloth(body: &[u8], at: usize) -> Option<ClothProps> {
    let block = body.get(at..(at + 512).min(body.len()))?;
    let table = block.iter().position(|&b| b == 0x89)?;
    let size = u32::from_le_bytes(block.get(table + 1..table + 5)?.try_into().ok()?) as usize;
    let anchors = (0..size.saturating_sub(4) / 4)
        .filter_map(|k| {
            let at = table + 5 + k * 4;
            entity_ref(u32::from_le_bytes(block.get(at..at + 4)?.try_into().ok()?))
        })
        .collect();
    let double = |at: usize| block.get(at..at + 8).map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32);
    let wind = (0..table)
        .find(|&i| block[i] == 0x81 && block.get(i + 9) == Some(&0x41) && block.get(i + 18) == Some(&0x41))
        .and_then(|i| Some([double(i + 1)?, double(i + 10)?, double(i + 19)?]))
        .unwrap_or([0.0; 3]);
    Some(ClothProps { anchors, wind })
}

/// Engine class of patrol paths: a group whose node list references waypoint entities.
pub const PATH_CLASS: u32 = 0x0800_00c8;

/// Decodes an entity reference: `0x80000000 | (index + 1)`, 0 = none.
pub fn entity_ref(raw: u32) -> Option<usize> {
    (raw & 0x00ff_ffff).checked_sub(1).map(|i| i as usize)
}

/// Path node list: `u32 size, u32 header, u8 0`, then `(size - 22) / 4` references, then a
/// 13-byte trailer.
fn parse_path(body: &[u8], at: usize) -> Vec<usize> {
    let Ok(size) = body.u32_at(at) else { return Vec::new() };
    let count = (size as usize).saturating_sub(22) / 4;
    (0..count.min(256)).filter_map(|k| body.u32_at(at + 9 + k * 4).ok()).take_while(|r| r >> 31 == 1).filter_map(entity_ref).collect()
}

pub fn inflate(raw: &[u8]) -> Result<Vec<u8>> {
    let size = raw.u32_at(0)? as usize;
    if raw.get(8) == Some(&1) {
        return Ok(raw[9..].to_vec());
    }
    let mut out = Vec::with_capacity(size);
    flate2::read::DeflateDecoder::new(&raw[9..]).read_to_end(&mut out).context("inflating GMS")?;
    if out.len() != size {
        bail!("GMS inflated to {} bytes, header says {size}", out.len());
    }
    Ok(out)
}

fn parse_script_props(chunk: &[u8]) -> Option<ScriptProps> {
    let needle = b".sdl\0";
    let sdl_pos = chunk.windows(needle.len()).position(|w| w == needle)?;
    let mut s_start = sdl_pos;
    while s_start > 0 && chunk[s_start - 1] != 0 && chunk[s_start - 1] != 4 && chunk[s_start - 1] != 0x84 && chunk[s_start - 1] != 0x85 {
        s_start -= 1;
    }
    let script = std::str::from_utf8(&chunk[s_start..sdl_pos + 4]).ok()?.to_owned();

    let mut decl_start = sdl_pos + 5;
    if decl_start < chunk.len() && (chunk[decl_start] == 4 || chunk[decl_start] == 0x84 || chunk[decl_start] == 0x85) {
        decl_start += 1;
    }
    let decl_len = chunk.get(decl_start..)?.iter().position(|&b| b == 0)?;
    let decl_str = std::str::from_utf8(&chunk[decl_start..decl_start + decl_len]).ok()?;

    let mut fields = Vec::new();
    for part in decl_str.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.len() >= 2 {
            let fname = tokens[1].split('=').next().unwrap_or("").trim();
            if !fname.is_empty() {
                fields.push((tokens[0], fname.to_owned()));
            }
        }
    }

    let mut p = decl_start + decl_len + 1;
    let mut props = ScriptProps {
        script,
        hitpoints: 20,
        character_type: 0,
        relaxed_vis_range: 3000.0,
        alert_vis_range: 5000.0,
        ..Default::default()
    };

    for (_ftype, fname) in fields {
        if p >= chunk.len() {
            break;
        }
        let tag = chunk[p];
        p += 1;
        match tag {
            3 => {
                if p + 4 > chunk.len() {
                    break;
                }
                let val = i32::from_le_bytes([chunk[p], chunk[p + 1], chunk[p + 2], chunk[p + 3]]);
                p += 4;
                match fname.as_str() {
                    "m_iHitpoints" => props.hitpoints = val,
                    "CharacterType" => props.character_type = val,
                    "IsSentry" => props.is_sentry = val,
                    _ => {}
                }
                props.int_props.insert(fname, val);
            }
            2 => {
                if p + 4 > chunk.len() {
                    break;
                }
                let val = f32::from_le_bytes([chunk[p], chunk[p + 1], chunk[p + 2], chunk[p + 3]]);
                p += 4;
                match fname.as_str() {
                    "RelaxedVisRange" => props.relaxed_vis_range = val,
                    "AlertVisRange" => props.alert_vis_range = val,
                    "SentryRadius" => props.sentry_radius = val,
                    "WaitTime" => props.wait_time = val,
                    _ => {}
                }
                props.float_props.insert(fname, val);
            }
            8 => {
                if p + 4 > chunk.len() {
                    break;
                }
                let ref_val = u32::from_le_bytes([chunk[p], chunk[p + 1], chunk[p + 2], chunk[p + 3]]);
                p += 4;
                let Some(entity_idx) = entity_ref(ref_val) else { continue };
                match fname.as_str() {
                    "WayPoints" => props.waypoints = Some(entity_idx),
                    "WeaponTemplate" => props.weapon_template = Some(entity_idx),
                    "GuardPoint" => props.guard_point = Some(entity_idx),
                    _ => {}
                }
                props.ref_props.insert(fname, entity_idx);
            }
            4 | 0x84 | 0x85 => {
                let Some(end) = chunk.get(p..)?.iter().position(|&b| b == 0) else { break };
                if let Ok(s) = std::str::from_utf8(&chunk[p..p + end]) {
                    props.str_props.insert(fname, s.to_owned());
                }
                p += end + 1;
            }
            _ => break,
        }
    }
    Some(props)
}

pub fn parse_entities(body: &[u8], buf: &[u8]) -> Result<Vec<Entity>> {
    let table = body.u32_at(0)? as usize;
    let count = body.u32_at(table)? as usize;
    let mut raw_offsets = Vec::with_capacity(count);
    for i in 0..count {
        let packed = body.u32_at(table + 4 + i * 8)?;
        let e = (packed & 0x00ff_ffff) as usize * 4;
        raw_offsets.push((packed, e));
    }
    let mut sorted_offsets: Vec<usize> = raw_offsets.iter().map(|&(_, e)| e).collect();
    sorted_offsets.sort_unstable();
    sorted_offsets.dedup();

    let mut out = Vec::with_capacity(count);
    for &(packed, e) in &raw_offsets {
        let rot_at = body.u32_at(e + 0x04)? as usize;
        let pos_at = body.u32_at(e + 0x08)? as usize;
        let mut rotation = [[0.0; 3]; 3];
        for (r, row) in rotation.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = body.f32_at(rot_at + (r * 3 + c) * 4)?;
            }
        }
        let next_e = sorted_offsets.get(sorted_offsets.partition_point(|&off| off <= e)).copied().unwrap_or(body.len());
        let props = if next_e > e + 0x20 {
            parse_script_props(&body[e + 0x20..next_e])
        } else {
            None
        };

        let type_id = body.u32_at(e + 0x10)?;
        let cloth = if type_id == CLOTH_CLASS { body.u32_at(e + 0x20).ok().and_then(|at| parse_cloth(body, at as usize)) } else { None };
        let sound = if type_id == SOUND_CLASS {
            body.u32_at(e + 0x20).ok().and_then(|at| (body.get(at as usize + 9) == Some(&0x8b)).then(|| body.u32_at(at as usize + 10).ok()).flatten())
        } else {
            None
        };
        let path = if type_id == PATH_CLASS { body.u32_at(e + 0x20).map(|at| parse_path(body, at as usize)).unwrap_or_default() } else { Vec::new() };
        out.push(Entity {
            name: buf.cstr_at(body.u32_at(e)? as usize).unwrap_or("<bad name>").to_owned(),
            type_id,
            path,
            cloth,
            sound,
            flags: body.u32_at(e + 0x18)?,
            primitive: body.u32_at(e + 0x0c)?,
            rotation,
            position: [body.f32_at(pos_at)?, body.f32_at(pos_at + 4)?, body.f32_at(pos_at + 8)?],
            tree_bits: (packed >> 24) as u8,
            props,
        });
    }
    Ok(out)
}
