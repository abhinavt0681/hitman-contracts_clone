//! Skinned character models in `.PRM` (sub-mesh records with `kind == 1`).
//!
//! Characters are referenced from GMS like static models, but their records are 0x98 bytes
//! and not listed in the PRM table. Each record carries the shared fields of a static
//! sub-mesh (texture +0x04, prev +0x08, vertex count +0x0E, vertices +0x14, indices +0x34)
//! plus:
//!
//! - `+0x74 u32 bone_count` (72 for the standard human skeleton)
//! - `+0x78 ptr` local bind matrices, 48 bytes per bone (3×3 + position)
//! - `+0x7C ptr` local bind pose, 28 bytes per bone: quaternion xyzw, position xyz (cm)
//! - `+0x80 ptr` model-space bind matrices, 48 bytes per bone
//! - `+0x88 ptr` bone records, 48 bytes: name at +28 (16 chars), channel id at +46
//! - `+0x8C ptr` channel id → bone index table (96 bytes, 0xFF = unused)
//! - `+0x90 u32 run_count`, `+0x94 ptr` palette runs: `run_count` × (`u32 len, u32 start`),
//!   both in units of 12 bytes, each naming `len/12` consecutive bones from `start/12`.
//!   Concatenated, they form the sub-mesh's bone palette.
//!
//! Vertices are 52 bytes: position f32×3, weights f32×3, palette bytes u8×4 (shader register
//! offsets: slot = byte / 3), normal f32×3, BGRA u8×4, uv f32×2.
//!
//! Parents aren't stored. The skeleton is the engine's fixed human rig, so they follow from
//! bone names; composing the local poses that way reproduces the model-space bind matrices.

use super::{ReadLe, prm::{PrmArchive, read_strips}};
use anyhow::{Result, bail};

const RECORD_SKINNED_KIND: u16 = 1;
const VERTEX_STRIDE: usize = 52;
const MAX_SUBMESHES: usize = 64;

#[derive(Clone)]
pub struct Bone {
    pub name: String,
    /// Animation channel id (what ANM tracks are keyed by).
    pub channel: u8,
    pub parent: Option<usize>,
    /// Local bind pose: rotation xyzw and translation (cm), relative to the parent.
    pub rotation: [f32; 4],
    pub translation: [f32; 3],
    /// Model-space bind position as stored in the file (for validating the hierarchy).
    pub model_position: [f32; 3],
}

#[derive(Clone)]
pub struct SkinVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Skeleton bone indices (already resolved through the palette) and their weights.
    pub bones: [u16; 4],
    pub weights: [f32; 4],
}

#[derive(Clone)]
pub struct SkinSubMesh {
    pub texture: u32,
    pub blended: bool,
    /// Detail levels this part is drawn at (bit 0 = closest); see `prm`.
    pub lod_mask: u8,
    pub vertices: Vec<SkinVertex>,
    pub triangles: Vec<[u16; 3]>,
}

#[derive(Clone)]
pub struct SkinnedModel {
    pub bones: Vec<Bone>,
    pub submeshes: Vec<SkinSubMesh>,
}

impl PrmArchive {
    /// Loads the skinned model whose last sub-mesh record is at `offset`.
    pub fn skinned_model(&self, offset: u32) -> Result<SkinnedModel> {
        let d = self.data();
        let head = offset as usize;
        if self.kind_at(offset) != Some(RECORD_SKINNED_KIND) {
            bail!("{offset:#x} is not a skinned sub-mesh record");
        }
        let bones = read_skeleton(d, head)?;

        let mut submeshes = Vec::new();
        let mut rec = head;
        while rec != 0 {
            if submeshes.len() >= MAX_SUBMESHES || self.kind_at(rec as u32) != Some(RECORD_SKINNED_KIND) {
                bail!("bad skinned sub-mesh chain at {rec:#x} (model {offset:#x})");
            }
            if let Some(sm) = read_skin_submesh(d, rec, bones.len())? {
                submeshes.push(sm);
            }
            rec = d.u32_at(rec + 0x08)? as usize;
        }
        submeshes.reverse();
        // Keep only the most detailed version of each part.
        submeshes.retain(|s| s.lod_mask == 0 || s.lod_mask & 1 != 0);
        Ok(SkinnedModel { bones, submeshes })
    }
}

fn read_skeleton(d: &[u8], rec: usize) -> Result<Vec<Bone>> {
    let count = d.u32_at(rec + 0x74)? as usize;
    let local = d.u32_at(rec + 0x7c)? as usize;
    let names = d.u32_at(rec + 0x88)? as usize;
    let model = d.u32_at(rec + 0x80)? as usize;
    if count == 0 || count > 256 {
        bail!("implausible bone count {count} at {rec:#x}");
    }
    let mut bones = Vec::with_capacity(count);
    for i in 0..count {
        let nrec = names + i * 48;
        let raw = d.get(nrec + 28..nrec + 44).unwrap_or(&[]);
        let name = String::from_utf8_lossy(raw.split(|&c| c == 0).next().unwrap_or(&[])).into_owned();
        let l = local + i * 28;
        let f = |k: usize| d.f32_at(l + k * 4);
        bones.push(Bone {
            name,
            channel: d.get(nrec + 46).copied().unwrap_or(0xff),
            parent: None,
            rotation: [f(0)?, f(1)?, f(2)?, f(3)?],
            translation: [f(4)?, f(5)?, f(6)?],
            model_position: [d.f32_at(model + i * 48 + 36)?, d.f32_at(model + i * 48 + 40)?, d.f32_at(model + i * 48 + 44)?],
        });
    }
    let index = |n: &str| bones.iter().position(|b| b.name == n);
    let parents: Vec<Option<usize>> = bones.iter().map(|b| parent_name(&b.name).and_then(|p| index(&p))).collect();
    for (b, p) in bones.iter_mut().zip(parents) {
        b.parent = p;
    }
    Ok(bones)
}

/// The engine's human rig. Face bones hang off HEAD, except the lower lips which ride the JAW.
fn parent_name(name: &str) -> Option<String> {
    let fixed = match name {
        "GROUND" => return None,
        "PELVIS" => "GROUND",
        "SPINE" => "PELVIS",
        "SPINE_1" => "SPINE",
        "NECK" => "SPINE_1",
        "HEAD" => "NECK",
        "LOWER_LIP" | "LEFT_LOWER_LIP" | "RIGHT_LOWER_LIP" => "JAW",
        _ => "",
    };
    if !fixed.is_empty() {
        return Some(fixed.to_owned());
    }
    for side in ["LEFT", "RIGHT"] {
        let Some(rest) = name.strip_prefix(side).and_then(|r| r.strip_prefix('_')) else { continue };
        let p = match rest {
            "CLAVICLE" => "NECK".to_owned(),
            "UPPER_ARM" => format!("{side}_CLAVICLE"),
            "FOREARM" => format!("{side}_UPPER_ARM"),
            "HAND" => format!("{side}_FOREARM"),
            "THIGH" => "PELVIS".to_owned(),
            "CALF" => format!("{side}_THIGH"),
            "FOOT" => format!("{side}_CALF"),
            "TOE" => format!("{side}_FOOT"),
            _ => match rest.strip_prefix("FINGER_").map(|f| f.split('_').collect::<Vec<_>>()) {
                Some(parts) if parts.len() == 1 => format!("{side}_HAND"),
                Some(parts) if parts.len() == 2 => match parts[1] {
                    "1" => format!("{side}_FINGER_{}", parts[0]),
                    k => format!("{side}_FINGER_{}_{}", parts[0], k.parse::<u32>().unwrap_or(1) - 1),
                },
                _ => "HEAD".to_owned(), // face bones such as LEFT_EYE
            },
        };
        return Some(p);
    }
    Some("HEAD".to_owned())
}

fn read_skin_submesh(d: &[u8], rec: usize, bone_count: usize) -> Result<Option<SkinSubMesh>> {
    let texword = d.u32_at(rec + 0x04)?;
    let vcount = d.u16_at(rec + 0x0e)? as usize;
    let vbase = d.u32_at(rec + 0x14)? as usize;
    let ibase = d.u32_at(rec + 0x34)? as usize;
    if vcount == 0 || vbase == 0 || ibase == 0 {
        return Ok(None);
    }

    let runs = d.u32_at(rec + 0x90)? as usize;
    let runs_at = d.u32_at(rec + 0x94)? as usize;
    let mut palette = Vec::new();
    for r in 0..runs {
        let len = d.u32_at(runs_at + r * 8)? as usize / 12;
        let start = d.u32_at(runs_at + r * 8 + 4)? as usize / 12;
        palette.extend((start..start + len).map(|b| b.min(bone_count - 1) as u16));
    }
    if palette.is_empty() {
        palette.push(0);
    }

    let mut vertices = Vec::with_capacity(vcount);
    for i in 0..vcount {
        let v = vbase + i * VERTEX_STRIDE;
        let f = |k: usize| d.f32_at(v + k * 4);
        let w = [f(3)?, f(4)?, f(5)?];
        let slots = d.get(v + 24..v + 28).unwrap_or(&[0; 4]);
        let mut bones = [0u16; 4];
        for k in 0..3 {
            bones[k] = palette[(slots[k] as usize / 3).min(palette.len() - 1)];
        }
        let sum: f32 = w.iter().sum();
        let weights = if sum > 1e-4 { [w[0] / sum, w[1] / sum, w[2] / sum, 0.0] } else { [1.0, 0.0, 0.0, 0.0] };
        vertices.push(SkinVertex {
            pos: [f(0)?, f(1)?, f(2)?],
            normal: [f(7)?, f(8)?, f(9)?],
            uv: [f(11)?, f(12)?],
            bones,
            weights,
        });
    }
    let triangles = read_strips(d, ibase, vcount)?;
    if triangles.is_empty() {
        return Ok(None);
    }
    let lod_mask = (d.u32_at(rec + 0x2c)? >> 16) as u8;
    Ok(Some(SkinSubMesh { texture: texword & 0x7ff, blended: texword & 0x800 != 0, lod_mask, vertices, triangles }))
}

impl SkinnedModel {
    /// Adds `other`'s parts that pass `keep` to this model, re-targeting their bone indices by
    /// name (rigs order bones differently). Used to give a stripped body its owner's head.
    pub fn with_parts_from(&self, other: &SkinnedModel, keep: impl Fn(&SkinSubMesh) -> bool) -> SkinnedModel {
        let index: std::collections::HashMap<&str, u16> = self.bones.iter().enumerate().map(|(i, b)| (b.name.as_str(), i as u16)).collect();
        let mut out = self.clone();
        for part in other.submeshes.iter().filter(|p| keep(p)) {
            let mut p = part.clone();
            for v in &mut p.vertices {
                for b in &mut v.bones {
                    *b = other.bones.get(*b as usize).and_then(|ob| index.get(ob.name.as_str()).copied()).unwrap_or(0);
                }
            }
            out.submeshes.push(p);
        }
        out
    }

    /// Largest distance (cm) between bind positions composed from local poses and the stored
    /// model-space ones. Near zero means the parent rules match this rig.
    pub fn hierarchy_error(&self) -> f32 {
        let mut world: Vec<([f32; 4], [f32; 3])> = Vec::with_capacity(self.bones.len());
        let mut worst = 0.0f32;
        for b in &self.bones {
            let w = match b.parent.and_then(|p| world.get(p)) {
                Some(&(pq, pt)) => {
                    let r = quat_rotate(pq, b.translation);
                    (quat_mul(pq, b.rotation), [pt[0] + r[0], pt[1] + r[1], pt[2] + r[2]])
                }
                None => (b.rotation, b.translation),
            };
            let e = (0..3).map(|k| (w.1[k] - b.model_position[k]).powi(2)).sum::<f32>().sqrt();
            worst = worst.max(e);
            world.push(w);
        }
        worst
    }
}

fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [aw * bx + ax * bw + ay * bz - az * by, aw * by - ax * bz + ay * bw + az * bx, aw * bz + ax * by - ay * bx + az * bw, aw * bw - ax * bx - ay * by - az * bz]
}

fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let t = [2.0 * (y * v[2] - z * v[1]), 2.0 * (z * v[0] - x * v[2]), 2.0 * (x * v[1] - y * v[0])];
    [v[0] + w * t[0] + y * t[2] - z * t[1], v[1] + w * t[1] + z * t[0] - x * t[2], v[2] + w * t[2] + x * t[1] - y * t[0]]
}

impl SkinSubMesh {
    /// Lowest vertex height (cm) in the bind pose; heads sit above ~140.
    pub fn min_height(&self) -> f32 {
        self.vertices.iter().map(|v| v.pos[1]).fold(f32::MAX, f32::min)
    }
}
