//! Sanity-checks the Glacier readers against a level and optionally dumps textures.
//!
//! Usage: `glacier-dump [LEVEL] [--textures OUT_DIR]`

use anyhow::Result;
use hitman_clone::glacier::level::Level;
use std::{collections::BTreeMap, io::Write, path::Path};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let id = args.first().filter(|a| !a.starts_with("--")).map_or("C02-1", String::as_str);
    let level = Level::load(Path::new("data/Scenes"), id)?;

    let mut by_type: BTreeMap<u32, usize> = BTreeMap::new();
    let (mut meshes, mut bad, mut tris) = (0, 0, 0usize);
    for e in &level.entities {
        *by_type.entry(e.type_id).or_default() += 1;
        if e.primitive == 0 || !level.prm.is_record(e.primitive) {
            continue;
        }
        match level.prm.model(e.primitive) {
            Ok(m) => {
                meshes += 1;
                tris += m.submeshes.iter().map(|s| s.triangles.len()).sum::<usize>();
            }
            Err(err) => {
                bad += 1;
                if bad <= 5 {
                    eprintln!("{}: {err:#}", e.name);
                }
            }
        }
    }
    println!("{id}: {} entities, {} PRM records, {meshes} mesh instances ({tris} tris), {bad} failed", level.entities.len(), level.prm.len());
    let with_props = level.entities.iter().filter(|e| e.props.is_some()).count();
    let with_waypoints = level.entities.iter().filter(|e| e.props.as_ref().is_some_and(|p| p.waypoints > 0)).count();
    println!("  entities with script properties: {with_props}, with waypoints: {with_waypoints}");
    for (t, n) in by_type.iter().filter(|(_, n)| **n > 50) {
        println!("  type {t:#08x}: {n}");
    }

    if args.iter().any(|a| a == "--types") {
        // Per type: how many instances, how many untextured, mean model size.
        let mut agg: BTreeMap<u32, (usize, usize, f32, String)> = BTreeMap::new();
        for e in &level.entities {
            if e.primitive == 0 || !level.prm.is_record(e.primitive) {
                continue;
            }
            let m = level.prm.model(e.primitive)?;
            let size = (0..3).map(|k| m.bbox_max[k] - m.bbox_min[k]).fold(0.0, f32::max);
            let a = agg.entry(e.type_id).or_insert((0, 0, 0.0, e.name.clone()));
            a.0 += 1;
            a.1 += m.submeshes.iter().all(|s| s.texture == 0) as usize;
            a.2 += size;
        }
        let mut flags: BTreeMap<(bool, u32), (usize, Vec<String>)> = BTreeMap::new();
        for e in level.entities.iter().filter(|e| e.type_id == 0x200002 && level.prm.is_record(e.primitive)) {
            let m = level.prm.model(e.primitive)?;
            let f = flags.entry((m.submeshes.iter().all(|s| s.texture == 0), e.flags)).or_default();
            f.0 += 1;
            if f.1.len() < 4 {
                f.1.push(e.name.clone());
            }
        }
        for ((untex, fl), (n, names)) in flags.iter().filter(|(_, v)| v.0 > 5) {
            println!("  static untextured={untex} flags={fl:#06x} n={n} {names:?}");
        }
        for (t, (n, untex, size, ex)) in agg {
            println!("  {t:#08x} n={n:5} untextured={untex:5} avg_size={:8.0}cm e.g. {ex}", size / n as f32);
        }
    }

    if args.iter().any(|a| a == "--mission-items") {
        for (i, e) in level.entities.iter().enumerate() {
            let name_l = e.name.to_lowercase();
            if name_l.contains("soup") || name_l.contains("poison") || name_l.contains("tnt") || name_l.contains("bomb") || name_l.contains("remote") || name_l.contains("fuchs") || name_l.contains("bjarkhov") {
                let prm_info = if e.primitive != 0 && level.prm.is_record(e.primitive) {
                    format!("prm=0x{:x}", e.primitive)
                } else {
                    "no-prm".to_string()
                };
                let (bx, by, bz) = (e.position[0] * 0.01, e.position[1] * 0.01, -e.position[2] * 0.01);
                println!("[{i:4}] type={:#08x} {:<32} pos=({:.1}, {:.1}, {:.1}) bevy=({:.2}, {:.2}, {:.2}) {prm_info}",
                    e.type_id, e.name, e.position[0], e.position[1], e.position[2], bx, by, bz);
            }
        }
    }

    if let Some(i) = args.iter().position(|a| a == "--clip-ends") {
        let pat = args.get(i + 1).map_or("Die", String::as_str);
        for set in &level.anm.sets {
            for (n, a) in set.animations.iter().filter(|(n, _)| n.contains(pat)) {
                let at = |bone: &str| a.tracks.iter().find(|t| t.bone == bone).and_then(|t| t.positions.last()).map(|p| p.1);
                println!("  {}/{n}: {} frames, GROUND {:?} PELVIS {:?}", set.path.rsplit('\\').next().unwrap_or(""), a.frames, at("GROUND"), at("PELVIS"));
            }
        }
    }

    if args.iter().any(|a| a == "--anims") {
        for set in &level.anm.sets {
            let mut names: Vec<_> = set.animations.keys().collect();
            names.sort();
            println!("  {} — {} animations", set.path, names.len());
            for n in names.iter().filter(|n| n.starts_with("Walk") || n.starts_with("Run") || n.starts_with("Stand")).take(12) {
                let a = &set.animations[*n];
                let root = a.tracks.iter().find(|t| t.bone == "GROUND").and_then(|t| t.positions.last()).map(|p| p.1);
                println!("      {n}: {} frames ({:.2}s), {} tracks, root end {root:?}", a.frames, a.duration(), a.tracks.len());
            }
        }
    }

    if args.iter().any(|a| a == "--skins") {
        let mut seen = std::collections::HashSet::new();
        for (i, e) in level.entities.iter().enumerate() {
            if level.prm.kind_at(e.primitive) != Some(1) || !seen.insert(e.primitive) {
                continue;
            }
            let owner = i.checked_sub(1).map_or("?", |j| level.entities[j].name.as_str());
            match level.prm.skinned_model(e.primitive) {
                Ok(m) => println!(
                    "  {owner:<40} {:#x}: {} bones, {} parts, {} verts, hierarchy error {:.3}cm",
                    e.primitive,
                    m.bones.len(),
                    m.submeshes.len(),
                    m.submeshes.iter().map(|s| s.vertices.len()).sum::<usize>(),
                    m.hierarchy_error()
                ),
                Err(err) => println!("  {owner}: {err:#}"),
            }
        }
    }

    if let Some(i) = args.iter().position(|a| a == "--range") {
        let start: usize = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0);
        let end: usize = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(start + 20);
        for idx in start..end.min(level.entities.len()) {
            let e = &level.entities[idx];
            let prm_info = if e.primitive != 0 && level.prm.is_record(e.primitive) {
                format!("prm=0x{:x}", e.primitive)
            } else {
                "no-prm".to_string()
            };
            println!("[{idx:4}] tree=0x{:02x} type={:#08x} {} pos=({:.1}, {:.1}, {:.1}) {prm_info}\n  rot={:?}",
                e.tree_bits, e.type_id, e.name, e.position[0], e.position[1], e.position[2], e.rotation);
        }
    }

    if let Ok(m) = level.prm.model(0x4d23e) {
        println!("Model 0x4d23e bbox_min={:?} bbox_max={:?} submeshes={}", m.bbox_min, m.bbox_max, m.submeshes.len());
        for (i, sm) in m.submeshes.iter().enumerate() {
            let (mut min_v, mut max_v) = ([f32::MAX; 3], [f32::MIN; 3]);
            for v in &sm.vertices {
                for k in 0..3 {
                    min_v[k] = min_v[k].min(v.pos[k]);
                    max_v[k] = max_v[k].max(v.pos[k]);
                }
            }
            println!("  submesh {i}: tex={} verts={} min={min_v:?} max={max_v:?}", sm.texture, sm.vertices.len());
        }
    }

    if args.iter().any(|a| a == "--tree") {
        #[allow(dead_code)]
        struct Frame {
            name: String,
            world_pos: [f32; 3],
            rot: [[f32; 3]; 3],
            is_physical: bool,
        }
        let mut stack: Vec<Frame> = Vec::new();
        let mut door_count = 0;
        for (i, e) in level.entities.iter().enumerate() {
            let pops = (e.tree_bits >> 1) as usize;
            for _ in 0..pops {
                stack.pop();
            }
            let (world_pos, is_child_of_physical) = if let Some(parent) = stack.last() {
                if parent.is_physical {
                    // child pos transformed by parent rotation: v * R
                    let v = e.position;
                    let r = parent.rot;
                    let rotated = [
                        v[0] * r[0][0] + v[1] * r[1][0] + v[2] * r[2][0],
                        v[0] * r[0][1] + v[1] * r[1][1] + v[2] * r[2][1],
                        v[0] * r[0][2] + v[1] * r[1][2] + v[2] * r[2][2],
                    ];
                    ([
                        parent.world_pos[0] + rotated[0],
                        parent.world_pos[1] + rotated[1],
                        parent.world_pos[2] + rotated[2],
                    ], true)
                } else {
                    (e.position, false)
                }
            } else {
                (e.position, false)
            };

            let is_physical = is_child_of_physical || e.position[0].abs() > 1.0 || e.position[1].abs() > 1.0 || e.position[2].abs() > 1.0;

            if (e.tree_bits & 1) != 0 {
                stack.push(Frame {
                    name: e.name.clone(),
                    world_pos,
                    rot: e.rotation,
                    is_physical,
                });
            }

            if e.name.starts_with("DDoor") || e.name.starts_with("Door_") || (is_child_of_physical && (e.name.starts_with("D1") || e.name.starts_with("D2") || e.name.starts_with("D0"))) {
                door_count += 1;
                let (bx, by, bz) = (world_pos[0] * 0.01, world_pos[1] * 0.01, -world_pos[2] * 0.01);
                println!("[{i:4}] {:<22} world=({:.1}, {:.1}, {:.1}) bevy=({:.2}, {:.2}, {:.2}) local=({:.1}, {:.1}, {:.1})",
                    e.name, world_pos[0], world_pos[1], world_pos[2], bx, by, bz, e.position[0], e.position[1], e.position[2]);
            }
        }
        println!("Found {door_count} door entities. Final stack len: {}", stack.len());
    }

    if let Some(i) = args.iter().position(|a| a == "--textures") {
        let out = Path::new(&args[i + 1]);
        std::fs::create_dir_all(out)?;
        for tid in level.tex.ids() {
            let Some(t) = level.tex.decode(tid)? else { continue };
            // PAM keeps alpha and needs no encoder dependency.
            let mut f = std::fs::File::create(out.join(format!("{tid:04}_{}.pam", t.name.replace(['/', '\\', '#'], "_"))))?;
            write!(f, "P7\nWIDTH {}\nHEIGHT {}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n", t.width, t.height)?;
            f.write_all(&t.rgba)?;
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--strings") {
        let filter = args.get(i + 1).map(String::as_str).unwrap_or("");
        println!("Strings matching '{}':", filter);
        for (k, v) in level.strings.iter() {
            if filter.is_empty() || k.to_lowercase().contains(&filter.to_lowercase()) || v.to_lowercase().contains(&filter.to_lowercase()) {
                println!("  {k} = {v}");
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--entities") {
        let filter = args.get(i + 1).map(String::as_str).unwrap_or("");
        println!("Entities matching '{}':", filter);
        for (idx, e) in level.entities.iter().enumerate() {
            if filter.is_empty() || e.name.to_lowercase().contains(&filter.to_lowercase()) {
                println!("  [{idx:4}] {:<32} type={:#08x} pos=({:.2}, {:.2}, {:.2}) prm=0x{:x}",
                    e.name, e.type_id, e.position[0], e.position[1], e.position[2], e.primitive);
            }
        }
    }
    if args.iter().any(|a| a == "--test-audio") {
        let whd_path = Path::new("data/Scenes/C02-1/C02-1_MAIN.WHD");
        let wav_path = Path::new("data/Scenes/C02-1/C02-1_MAIN.WAV");
        let streams_path = Path::new("data/streams.wav");
        let archive = hitman_clone::glacier::audio::AudioArchive::load(whd_path, wav_path, streams_path)?;
        println!("Loaded AudioArchive with {} sound entries", archive.entries.len());
        for name in &["Door_WoodOpen_01.wav", "Door_MetalOpen_01.wav", "W_HardballerSD_Shot_01.wav", "Music_Winter&Night.wav", "02-1_FOS_01Fuchs.wav"] {
            match archive.decode(name) {
                Some(bytes) => println!("  Decoded {name}: {} bytes", bytes.len()),
                None => println!("  Failed to decode {name}"),
            }
        }
    }
    Ok(())
}
