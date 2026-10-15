//! `.TEX` texture archive.
//!
//! Layout:
//! - `u32 table_offset, u32 table2_offset, u32, u32` header.
//! - At `table_offset`: 2048 × `u32` absolute offsets, indexed by texture id (0 = unused).
//!   Ids start at 32; PRM sub-meshes reference textures by this id.
//! - Each entry: `u32 entry_size, [u8;4] fourcc (reversed, e.g. "1TXD"), [u8;4] fourcc again,
//!   u32 id, u16 height, u16 width, u32 mip_count, u32 ×3 flags, NUL-terminated name`,
//!   then `mip_count` × (`u32 size, size bytes`).
//! - `PALN` entries follow their mips with `u32 count` and `count` × RGBA palette colors.

use super::ReadLe;
use anyhow::{Result, bail};

const TABLE_LEN: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Dxt1,
    Dxt3,
    Rgba,
    Paln,
    Other([u8; 4]),
}

pub struct Texture {
    pub id: u32,
    pub name: String,
    pub format: Format,
    pub width: u32,
    pub height: u32,
    /// Top mip level decoded to straight RGBA8.
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub struct TexArchive {
    data: Vec<u8>,
    offsets: Vec<u32>,
}

impl TexArchive {
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        let table = data.u32_at(0)? as usize;
        let offsets = (0..TABLE_LEN).map(|i| data.u32_at(table + i * 4)).collect::<Result<Vec<_>>>()?;
        Ok(Self { data, offsets })
    }

    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.offsets.iter().enumerate().filter(|(_, o)| **o != 0).map(|(i, _)| i as u32)
    }

    pub fn decode(&self, id: u32) -> Result<Option<Texture>> {
        let Some(&off) = self.offsets.get(id as usize) else { return Ok(None) };
        if off == 0 {
            return Ok(None);
        }
        let d = &self.data[..];
        let o = off as usize;
        let mut cc = [0u8; 4];
        cc.copy_from_slice(&d[o + 4..o + 8]);
        cc.reverse();
        let format = match &cc {
            b"DXT1" => Format::Dxt1,
            b"DXT3" => Format::Dxt3,
            b"RGBA" => Format::Rgba,
            b"PALN" => Format::Paln,
            _ => Format::Other(cc),
        };
        let height = d.u16_at(o + 16)? as u32;
        let width = d.u16_at(o + 18)? as u32;
        let mips = d.u32_at(o + 20)?;
        let name = d.cstr_at(o + 36)?.to_owned();
        let mut p = o + 36 + name.len() + 1;
        let mip0_size = d.u32_at(p)? as usize;
        let mip0 = &d[p + 4..p + 4 + mip0_size];
        let (w, h) = (width as usize, height as usize);

        let rgba = match format {
            Format::Dxt1 | Format::Dxt3 => {
                let bc = if format == Format::Dxt1 { texpresso::Format::Bc1 } else { texpresso::Format::Bc2 };
                if mip0_size < bc.compressed_size(w, h) {
                    bail!("texture {id} '{name}': mip0 too small for {w}x{h} {format:?}");
                }
                let mut out = vec![0u8; w * h * 4];
                bc.decompress(mip0, w, h, &mut out);
                out
            }
            // Already R, G, B, A in memory (skin tones go blue if treated as BGRA).
            Format::Rgba => mip0.to_vec(),
            Format::Paln => {
                for _ in 0..mips {
                    let s = d.u32_at(p)? as usize;
                    p += 4 + s;
                }
                let count = d.u32_at(p)? as usize;
                let pal = &d[p + 4..p + 4 + count * 4];
                mip0.iter()
                    .flat_map(|&i| {
                        let c = pal.get(i as usize * 4..i as usize * 4 + 4).unwrap_or(&[255, 0, 255, 255]);
                        [c[0], c[1], c[2], c[3]]
                    })
                    .collect()
            }
            Format::Other(_) => return Ok(None),
        };
        Ok(Some(Texture { id, name, format, width, height, rgba }))
    }
}
