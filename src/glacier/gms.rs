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

use super::ReadLe;
use anyhow::{Context, Result, bail};
use std::io::Read;

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ScriptProps {
    pub script: String,
    pub hitpoints: i32,
    pub character_type: i32,
    pub waypoints: u32,
    pub weapon_template: u32,
    pub guard_point: u32,
    pub is_sentry: i32,
    pub sentry_radius: f32,
    pub relaxed_vis_range: f32,
    pub alert_vis_range: f32,
    pub wait_time: f32,
    pub int_props: HashMap<String, i32>,
    pub float_props: HashMap<String, f32>,
    pub ref_props: HashMap<String, u32>,
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
                let entity_idx = ref_val & 0x00ff_ffff;
                p += 4;
                match fname.as_str() {
                    "WayPoints" => props.waypoints = entity_idx,
                    "WeaponTemplate" => props.weapon_template = entity_idx,
                    "GuardPoint" => props.guard_point = entity_idx,
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
        let next_e = sorted_offsets.iter().copied().find(|&off| off > e).unwrap_or(body.len());
        let props = if next_e > e + 0x20 {
            parse_script_props(&body[e + 0x20..next_e])
        } else {
            None
        };

        out.push(Entity {
            name: buf.cstr_at(body.u32_at(e)? as usize).unwrap_or("<bad name>").to_owned(),
            type_id: body.u32_at(e + 0x10)?,
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
