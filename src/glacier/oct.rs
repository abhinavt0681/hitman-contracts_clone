//! Glacier 1 `.OCT` spatial physics octree.
//!
//! Reverse-engineered from `HitmanContracts.exe`:
//! - Scene archive dispatcher: 0x00425750 (entity groups), 0x00425814 (global trees), 0x00425855 (link records)
//! - `ZTreeGroup::Load`: 0x00456030 (loops 5 tree slots per entity group)
//! - `ZTree::Load`: 0x00629a30
//! - Global tree loader (+0x54): 0x006e3dd0
//! - Global link table loader (+0x48): 0x006e3370
//! - Node traversal & raycast: 0x006292f0, 0x006e2590, 0x006e36d0, 0x006e4140
//! - Vtable: 0x00715cd0 (`ZTree`), 0x0073e518 (Glacier physics/octree world)
//!
//! An `.OCT` file contains:
//! 1. 16-byte initial header.
//! 2. Tree groups corresponding to the level's `ZTreeGroup` entities (`0x100020` / `0x2003e9`),
//!    with 5 tree slots per group.
//! 3. Global scene octrees (starting after entity groups and terminated by `0xffffffff`).
//! 4. Array of 8-byte spatial link records (`poly_ref`, `primitive_offset`) mapping octree
//!    triangles to scene PRM primitives.

use super::ReadLe;
use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub struct OctNode {
    pub poly_count: usize,
    pub flags: u16,
    pub child_index: u16,
    pub poly_start: u16,
}

#[derive(Debug, Clone)]
pub struct OctPoly {
    pub poly_ref: u32,
    pub min_q: [u16; 3],
    pub max_q: [u16; 3],
}

impl OctPoly {
    pub fn world_bounds(&self, origin: [f32; 3], scale: f32) -> ([f32; 3], [f32; 3]) {
        let dequant = |q: [u16; 3]| {
            [
                origin[0] + (q[0] as f32 - 32768.0) * scale,
                origin[1] + (q[1] as f32 - 32768.0) * scale,
                origin[2] + (q[2] as f32 - 32768.0) * scale,
            ]
        };
        (dequant(self.min_q), dequant(self.max_q))
    }
}

#[derive(Debug, Clone)]
pub struct OctTree {
    pub origin: [f32; 3],
    pub scale: f32,
    pub inv_scale: f32,
    pub nodes: Vec<OctNode>,
    pub polys: Vec<OctPoly>,
}

#[derive(Debug, Clone, Default)]
pub struct OctTreeGroup {
    pub trees: [Option<OctTree>; 5],
}

#[derive(Debug, Clone, Default)]
pub struct OctFile {
    pub entity_groups: Vec<OctTreeGroup>,
    pub global_trees: Vec<OctTree>,
    pub spatial_links: Vec<[u32; 2]>,
}

fn parse_tree_payload(payload: &[u8]) -> Result<Option<OctTree>> {
    if payload.len() < 0x14 {
        return Ok(None);
    }
    let poly_table_rel = payload.u32_at(0)? as usize;
    if poly_table_rel < 0x14 || poly_table_rel > payload.len() {
        return Ok(None);
    }

    let origin = [payload.f32_at(4)?, payload.f32_at(8)?, payload.f32_at(12)?];
    let inv_scale = payload.f32_at(16)?;
    let scale = if inv_scale != 0.0 { 1.0 / inv_scale } else { 0.0 };

    // Node table: 0x14 .. poly_table_rel (6 bytes per node, padded to 16 bytes before poly table)
    let node_bytes = poly_table_rel - 0x14;
    let node_count = node_bytes / 6;
    let mut nodes = Vec::with_capacity(node_count);
    for i in 0..node_count {
        let noff = 0x14 + i * 6;
        let fc = payload.u16_at(noff)?;
        let child = payload.u16_at(noff + 2)?;
        let poly = payload.u16_at(noff + 4)?;
        nodes.push(OctNode {
            poly_count: ((fc >> 3) & 0xfff) as usize,
            flags: fc & 7,
            child_index: child,
            poly_start: poly,
        });
    }

    // Poly table: poly_table_rel .. payload.len(), 16 bytes each
    let poly_bytes = payload.len() - poly_table_rel;
    let poly_count = poly_bytes / 16;
    let mut polys = Vec::with_capacity(poly_count);
    for i in 0..poly_count {
        let poff = poly_table_rel + i * 16;
        let pref = payload.u32_at(poff)?;
        let x0 = payload.u16_at(poff + 4)?;
        let y0 = payload.u16_at(poff + 6)?;
        let z0 = payload.u16_at(poff + 8)?;
        let x1 = payload.u16_at(poff + 10)?;
        let y1 = payload.u16_at(poff + 12)?;
        let z1 = payload.u16_at(poff + 14)?;
        polys.push(OctPoly {
            poly_ref: pref,
            min_q: [x0, y0, z0],
            max_q: [x1, y1, z1],
        });
    }

    Ok(Some(OctTree {
        origin,
        scale,
        inv_scale,
        nodes,
        polys,
    }))
}

impl OctFile {
    /// Parses an `.OCT` file given the count of `ZTreeGroup` entities in the scene.
    pub fn parse_with_group_count(data: &[u8], num_groups: usize) -> Result<Self> {
        if data.len() < 0x20 {
            bail!("OCT file too small: {} bytes", data.len());
        }

        let mut p = 0x10; // First 16 bytes are header flags
        let mut entity_groups = Vec::with_capacity(num_groups);

        // 1. Entity tree groups
        for _ in 0..num_groups {
            if p + 4 > data.len() {
                break;
            }
            let mut group = OctTreeGroup::default();
            for slot in 0..5 {
                if p + 4 > data.len() {
                    break;
                }
                let size = data.u32_at(p)? as usize;
                if size == 0 {
                    p += 16;
                } else {
                    if p + 32 + size > data.len() {
                        bail!("Truncated tree in slot {} at offset {:#x}", slot, p);
                    }
                    let payload = &data[p + 32..p + 32 + size];
                    group.trees[slot] = parse_tree_payload(payload)?;
                    let aligned = (size + 15) & !15;
                    p += 32 + aligned;
                }
            }
            entity_groups.push(group);
        }

        // 2. Global scene trees up to 0xffffffff terminator (0x006e3dd0)
        let mut global_trees = Vec::new();
        while p + 4 <= data.len() {
            let size = data.u32_at(p)?;
            if size == 0xffff_ffff {
                p += 4;
                break;
            }
            let sz = size as usize;
            if sz >= 32 && p + sz <= data.len() {
                let payload = &data[p + 32..p + sz];
                if let Some(tree) = parse_tree_payload(payload)? {
                    global_trees.push(tree);
                }
            }
            p += sz;
        }

        // 3. Array of 8-byte spatial link records (0x006e3370)
        let rem_bytes = data.len().saturating_sub(p);
        let link_count = rem_bytes / 8;
        let mut spatial_links = Vec::with_capacity(link_count);
        for i in 0..link_count {
            let loff = p + i * 8;
            spatial_links.push([data.u32_at(loff)?, data.u32_at(loff + 4)?]);
        }

        Ok(Self {
            entity_groups,
            global_trees,
            spatial_links,
        })
    }

    /// Default parse auto-detecting the entity group boundary.
    pub fn parse(data: &[u8]) -> Result<Self> {
        // In Hitman: Contracts scenes, each ZTreeGroup entity in GMS consumes 5 slots.
        // If unknown, we detect where group-format transitions to global-tree format.
        // Group format: slots advance by 16 (empty) or 32 + ((size+15)&!15).
        // Global format: total chunk size is `size`, advancing by `size`.
        let p = 0x10;
        let mut candidate_p = p;
        let mut groups = 0;
        while candidate_p + 4 <= data.len() {
            let mut test_p = candidate_p;
            let mut valid_group = true;
            for _ in 0..5 {
                if test_p + 4 > data.len() {
                    valid_group = false;
                    break;
                }
                let sz = data.u32_at(test_p).unwrap_or(0) as usize;
                if sz == 0 {
                    test_p += 16;
                } else if sz == 0xffff_ffff || test_p + 32 + sz > data.len() {
                    valid_group = false;
                    break;
                } else {
                    let aligned = (sz + 15) & !15;
                    test_p += 32 + aligned;
                }
            }
            if !valid_group {
                break;
            }
            // Check if test_p looks like global tree (magic 0x0134bd24 at test_p + 4) or 0xffffffff
            let next_word = data.u32_at(test_p).unwrap_or(0);
            let next_magic = data.u32_at(test_p + 4).unwrap_or(0);
            groups += 1;
            candidate_p = test_p;
            if next_word == 0xffff_ffff || next_magic == 0x0134_bd24 {
                break;
            }
        }

        Self::parse_with_group_count(data, groups)
    }

    /// Total number of valid non-empty trees across both entity groups and global trees.
    pub fn all_trees(&self) -> impl Iterator<Item = &OctTree> {
        self.entity_groups
            .iter()
            .flat_map(|g| g.trees.iter().filter_map(|t| t.as_ref()))
            .chain(self.global_trees.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_parse_c02_1_oct() {
        let path = Path::new("data/extracted/SCENES/C02-1/C02-1_MAIN.OCT");
        if !path.exists() {
            return;
        }
        let data = std::fs::read(path).expect("reading OCT file");
        let oct = OctFile::parse_with_group_count(&data, 53).expect("parsing OCT file");
        println!(
            "Parsed {} entity groups, {} global trees, {} spatial links",
            oct.entity_groups.len(),
            oct.global_trees.len(),
            oct.spatial_links.len()
        );
        assert_eq!(oct.entity_groups.len(), 53);
        assert_eq!(oct.global_trees.len(), 898);
        assert_eq!(oct.spatial_links.len(), 22752);
    }
}
