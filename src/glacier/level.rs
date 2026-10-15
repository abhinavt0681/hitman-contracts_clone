//! Loads one scene's `_MAIN.ZIP` and exposes its parsed archives.

use super::{anm::AnmFile, gms, loc::Strings, prm::PrmArchive, tex::TexArchive};
use anyhow::{Context, Result};
use std::{fs::File, io::Read, path::Path};

pub struct Level {
    pub id: String,
    pub entities: Vec<gms::Entity>,
    pub prm: PrmArchive,
    pub tex: TexArchive,
    pub anm: AnmFile,
    pub strings: Strings,
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
        Ok(Self { id: id.to_owned(), entities, prm: PrmArchive::parse(read("PRM")?)?, tex: TexArchive::parse(read("TEX")?)?, anm, strings })
    }
}
