//! `.PRM` primitive (mesh) archive.
//!
//! Layout:
//! - Header: `u32 ?, u32 table_offset, u32 table_offset (dup), u32 count`.
//! - At `table_offset`: `count` × `u32` absolute offsets of sub-mesh records.
//! - Everything is addressed by absolute file offset. The table lists one record per model:
//!   its *last* sub-mesh. GMS entities reference models by that offset; the remaining
//!   sub-meshes chain backwards through `prev`.
//!
//! Sub-mesh record (0x70 bytes, offsets relative to the record):
//! - `+0x00 u16 flags` (0x181 for ordinary textured geometry), `+0x02 u16 kind`
//! - `+0x04 u32 texture` — TEX id in the low 11 bits; bit 0x800 = alpha-blended, with
//!   opacity taken from vertex alpha (fog cards, shadow decals, glass, water, icicles)
//! - `+0x20 u32 render_state` — low byte looks like a blend/shading mode; bit 0x800000 marks
//!   proxies that are never drawn (see `SubMesh::is_proxy`); the rest is not decoded yet
//! - `+0x2C u32` — bits 16–23: LOD mask, bit i set = drawn at detail level i (0 = closest).
//!   Models carry several overlapping versions; draw only one level.
//! - `+0x08 u32 prev` — previous sub-mesh record of the same model, 0 terminates
//! - `+0x0E u16 vertex_count`
//! - `+0x10 u32 model_info` — shared per-model block: bounding box (max xyz, min xyz) at +0x20
//! - `+0x14 u32 vertices` — `vertex_count` × 36 bytes: pos f32×3, normal f32×3, BGRA u8×4, uv f32×2
//! - `+0x34 u32 indices` — `u16 prim_count`, then per prim `u16 n` + `n` × u16 indices.
//!   Prims are triangle strips; `n == 2` marks line lists (bounding boxes, debug).
//!   Index blocks can be shared between identical sub-meshes.
//! - `+0x38 u32 index_words` — total u16 count of the index block.

use super::ReadLe;
use anyhow::{Result, bail};
use std::collections::HashMap;

const VERTEX_STRIDE: usize = 36;
/// Guards against walking a corrupt `prev` chain forever.
const MAX_SUBMESHES: usize = 256;

pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [u8; 4],
    pub uv: [f32; 2],
}

pub struct SubMesh {
    pub texture: u32,
    pub blended: bool,
    pub render_state: u32,
    pub lod_mask: u8,
    pub vertices: Vec<Vertex>,
    /// Triangle list, already unrolled from strips with winding fixed up.
    pub triangles: Vec<[u16; 3]>,
}

impl SubMesh {
    /// Part of the most detailed version of the model (parts without a mask always are).
    pub fn in_lod0(&self) -> bool {
        self.lod_mask == 0 || self.lod_mask & 1 != 0
    }

    /// A simplified stand-in never drawn: untextured collision hulls, and parts whose render
    /// state has bit 0x800000 (the same bit the untextured hulls carry) — e.g. the textured box
    /// inside the AN-124's fuselage, reaching to the ground.
    pub fn is_proxy(&self) -> bool {
        self.texture == 0 || self.render_state & 0x80_0000 != 0
    }
}

pub struct Model {
    pub bbox_min: [f32; 3],
    pub bbox_max: [f32; 3],
    pub submeshes: Vec<SubMesh>,
}

pub struct PrmArchive {
    data: Vec<u8>,
    offsets: Vec<u32>,
    index_of: HashMap<u32, usize>,
}

impl PrmArchive {
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        let table = data.u32_at(4)? as usize;
        let count = data.u32_at(12)? as usize;
        let offsets = (0..count).map(|i| data.u32_at(table + i * 4)).collect::<Result<Vec<_>>>()?;
        let index_of = offsets.iter().enumerate().map(|(i, &o)| (o, i)).collect();
        Ok(Self { data, offsets, index_of })
    }

    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub(crate) fn data(&self) -> &[u8] {
        &self.data
    }

    /// `kind` field of the sub-mesh record at `offset`: 0 static geometry, 1 skinned character.
    pub fn kind_at(&self, offset: u32) -> Option<u16> {
        let o = offset as usize;
        if self.data.u16_at(o).ok()? & 0xff != 0x81 {
            return None;
        }
        self.data.u16_at(o + 2).ok()
    }

    /// True if `offset` is the start of a sub-mesh record (i.e. a valid model reference).
    pub fn is_record(&self, offset: u32) -> bool {
        self.index_of.contains_key(&offset)
    }

    /// Loads the model whose last sub-mesh record sits at `offset`.
    pub fn model(&self, offset: u32) -> Result<Model> {
        let d = &self.data[..];
        let mut submeshes = Vec::new();
        let mut rec = offset as usize;
        let info = d.u32_at(rec + 0x10)? as usize;
        while rec != 0 {
            // Only the last record of each model is in the table, so earlier ones are
            // checked by shape: same model-info block as the head record.
            if submeshes.len() >= MAX_SUBMESHES || d.u32_at(rec + 0x10)? as usize != info {
                bail!("bad sub-mesh chain at {rec:#x} (model {offset:#x})");
            }
            if let Some(sm) = self.submesh(rec)? {
                submeshes.push(sm);
            }
            rec = d.u32_at(rec + 0x08)? as usize;
        }
        submeshes.reverse();
        let f = |o: usize| -> Result<[f32; 3]> { Ok([d.f32_at(o)?, d.f32_at(o + 4)?, d.f32_at(o + 8)?]) };
        Ok(Model { bbox_max: f(info + 0x20)?, bbox_min: f(info + 0x2c)?, submeshes })
    }

    fn submesh(&self, rec: usize) -> Result<Option<SubMesh>> {
        let d = &self.data[..];
        let texword = d.u32_at(rec + 0x04)?;
        let vcount = d.u16_at(rec + 0x0e)? as usize;
        let vbase = d.u32_at(rec + 0x14)? as usize;
        let ibase = d.u32_at(rec + 0x34)? as usize;
        if vcount == 0 || vbase == 0 || ibase == 0 {
            return Ok(None);
        }

        let mut vertices = Vec::with_capacity(vcount);
        for i in 0..vcount {
            let v = vbase + i * VERTEX_STRIDE;
            let c = d.get(v + 24..v + 28).unwrap_or(&[255; 4]);
            vertices.push(Vertex {
                pos: [d.f32_at(v)?, d.f32_at(v + 4)?, d.f32_at(v + 8)?],
                normal: [d.f32_at(v + 12)?, d.f32_at(v + 16)?, d.f32_at(v + 20)?],
                color: [c[2], c[1], c[0], c[3]],
                uv: [d.f32_at(v + 28)?, d.f32_at(v + 32)?],
            });
        }

        let triangles = read_strips(d, ibase, vcount)?;
        if triangles.is_empty() {
            return Ok(None);
        }

        Ok(Some(SubMesh { texture: texword & 0x7ff, blended: texword & 0x800 != 0, render_state: d.u32_at(rec + 0x20)?, lod_mask: (d.u32_at(rec + 0x2c)? >> 16) as u8, vertices, triangles }))
    }
}

/// Unrolls an index block (`u16 prim_count`, per prim `u16 n` + indices) into triangles.
/// Strips alternate winding; `n < 3` prims are debug lines and are skipped.
pub(crate) fn read_strips(d: &[u8], ibase: usize, vcount: usize) -> Result<Vec<[u16; 3]>> {
    let mut triangles = Vec::new();
    let prims = d.u16_at(ibase)? as usize;
    let mut p = ibase + 2;
    for _ in 0..prims {
        let n = d.u16_at(p)? as usize;
        p += 2;
        let strip = (0..n).map(|k| d.u16_at(p + k * 2)).collect::<Result<Vec<_>>>()?;
        p += n * 2;
        if n < 3 {
            continue;
        }
        for k in 0..n - 2 {
            let (a, b, c) = (strip[k], strip[k + 1], strip[k + 2]);
            if a == b || b == c || a == c {
                continue; // degenerate stitch between strips
            }
            if [a, b, c].iter().any(|&i| i as usize >= vcount) {
                bail!("index out of range in index block {ibase:#x}");
            }
            triangles.push(if k % 2 == 0 { [a, b, c] } else { [b, a, c] });
        }
    }
    Ok(triangles)
}
