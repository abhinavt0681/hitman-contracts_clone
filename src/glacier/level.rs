//! Loads one scene's `_MAIN.ZIP` and exposes its parsed archives.

use super::{anm::AnmFile, gms, loc::Strings, oct::OctFile, prm::PrmArchive, tex::TexArchive};
use anyhow::{Context, Result};
use std::{fs::File, io::Read, path::Path};

pub struct Level {
    pub id: String,
    pub entities: Vec<gms::Entity>,
    pub prm: PrmArchive,
    pub tex: TexArchive,
    pub anm: AnmFile,
    pub strings: Strings,
    /// The `.SND` table: sound records that emitters and scripts refer to by offset.
    pub snd: Vec<u8>,
    /// The `.ZGF`, inflated: the level's compiled scripts (see `ppo`).
    pub zgf: Vec<u8>,
    pub oct: Option<OctFile>,
}

impl Level {
    /// `scenes_dir` is the game's `Scenes` folder; `id` is e.g. `"C02-1"` (The Bjarkhov Bomb).
    pub fn load(scenes_dir: &Path, id: &str) -> Result<Self> {
        let path = scenes_dir.join(id).join(format!("{id}_MAIN.ZIP"));
        let mut zip = zip::ZipArchive::new(File::open(&path).with_context(|| format!("opening {}", path.display()))?)?;
        let mut read = |ext: &str| -> Result<Vec<u8>> {
            let name = format!("SCENES/{id}/{id}_MAIN.{ext}");
            let mut f = zip.by_name(&name).with_context(|| format!("{name} missing from {}", path.display()))?;
            let mut v = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut v)?;
            Ok(v)
        };
        let body = gms::inflate(&read("GMS")?)?;
        let entities = gms::parse_entities(&body, &read("BUF")?)?;
        let anm = AnmFile::parse(&read("ANM")?).context("parsing ANM")?;
        let strings = Strings::parse(&read("LOC")?);
        let snd = read("SND").unwrap_or_default();
        let zgf = read("ZGF").ok().and_then(|z| gms::inflate(&z).ok()).unwrap_or_default();
        let oct = read("OCT").ok().and_then(|bytes| OctFile::parse(&bytes).ok());
        Ok(Self { id: id.to_owned(), entities, prm: PrmArchive::parse(read("PRM")?)?, tex: TexArchive::parse(read("TEX")?)?, anm, strings, snd, zgf, oct })
    }

    /// Name of the sound whose `.SND` record starts at `offset` (records hold a pointer to
    /// their name at +4), as the sound bank knows it.
    pub fn sound_name(&self, offset: u32) -> Option<&str> {
        let at = offset as usize;
        let name_at = u32::from_le_bytes(self.snd.get(at + 4..at + 8)?.try_into().ok()?) as usize;
        let end = self.snd.get(name_at..)?.iter().position(|&b| b == 0)?;
        std::str::from_utf8(&self.snd[name_at..name_at + end]).ok()
    }
}
