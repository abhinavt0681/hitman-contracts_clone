//! Readers for Glacier 1 (Hitman: Contracts, 2004) scene data.
//!
//! A level ships as `Scenes/<ID>/<ID>_MAIN.ZIP`, a store-only zip holding one file per
//! resource kind. Format notes live next to each reader; `docs/FORMATS.md` collects them.

pub mod anm;
pub mod audio;
pub mod gms;
pub mod level;
pub mod loc;
pub mod oct;
pub mod ppo;
pub mod prm;
pub mod skin;
pub mod tex;

use anyhow::{Context, Result, bail};

/// Little-endian cursor-free readers over a byte slice; offsets are absolute.
pub(crate) trait ReadLe {
    fn u16_at(&self, off: usize) -> Result<u16>;
    fn u32_at(&self, off: usize) -> Result<u32>;
    fn f32_at(&self, off: usize) -> Result<f32>;
    fn cstr_at(&self, off: usize) -> Result<&str>;
}

impl ReadLe for [u8] {
    fn u16_at(&self, off: usize) -> Result<u16> {
        let b = self.get(off..off + 2).with_context(|| format!("u16 read out of bounds at {off:#x}"))?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32_at(&self, off: usize) -> Result<u32> {
        let b = self.get(off..off + 4).with_context(|| format!("u32 read out of bounds at {off:#x}"))?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn f32_at(&self, off: usize) -> Result<f32> {
        Ok(f32::from_bits(self.u32_at(off)?))
    }

    fn cstr_at(&self, off: usize) -> Result<&str> {
        let tail = self.get(off..).with_context(|| format!("string out of bounds at {off:#x}"))?;
        let Some(end) = tail.iter().position(|&c| c == 0) else {
            bail!("unterminated string at {off:#x}");
        };
        // Names are plain ASCII; anything else is a sign we followed a bad pointer.
        std::str::from_utf8(&tail[..end]).with_context(|| format!("non-UTF8 string at {off:#x}"))
    }
}
