//! Sanity-checks the Glacier readers against a level and optionally dumps textures.
//!
//! Usage: `glacier-dump [LEVEL] [--textures OUT_DIR]`

use anyhow::Result;
use hitman_clone::glacier::level::Level;
use std::{collections::BTreeMap, io::Write, path::Path};

fn bevy_math_vec(p: [f32; 3]) -> [f32; 3] { p }
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
trait CrossY { fn cross_y(self, o: [f32; 3]) -> f32; }
impl CrossY for [f32; 3] {
    /// Y component of the cross product.
    fn cross_y(self, o: [f32; 3]) -> f32 { self[2] * o[0] - self[0] * o[2] }
}

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
    let with_waypoints = level.entities.iter().filter(|e| e.props.as_ref().is_some_and(|p| p.waypoints.is_some())).count();
    println!("  entities with script properties: {with_props}, with waypoints: {with_waypoints}");
    for (t, n) in by_type.iter().filter(|(_, n)| **n > 50) {
        println!("  type {t:#08x}: {n}");
    }
    if let Some(oct) = &level.oct {
        let all_polys: usize = oct.all_trees().map(|t| t.polys.len()).sum();
        let all_nodes: usize = oct.all_trees().map(|t| t.nodes.len()).sum();
        println!(
            "  OCT physics: {} entity groups, {} global trees ({} nodes, {} polys), {} spatial links",
            oct.entity_groups.len(),
            oct.global_trees.len(),
            all_nodes,
            all_polys,
            oct.spatial_links.len()
        );
    }

    if let Some(i) = args.iter().position(|a| a == "--scripts") {
        // `--scripts [OUT_DIR]`: disassemble every compiled script; summary to stdout.
        use hitman_clone::glacier::ppo;
        let out = args.get(i + 1).filter(|a| !a.starts_with("--")).map(Path::new);
        if let Some(dir) = out {
            std::fs::create_dir_all(dir)?;
        }
        let scripts: Vec<ppo::Script> = ppo::scripts(&level.zgf).into_iter().filter_map(|s| s.map_err(|e| println!("  bad script: {e:#}")).ok()).collect();
        let program = ppo::Program::link(scripts);
        if let Ok(spec) = std::env::var("PPO_NAME") {
            // `PPO_NAME=k:i,i,…` prints global names as seen from script k (debugging).
            let (k, list) = spec.split_once(':').unwrap_or(("0", ""));
            let k: usize = k.parse().unwrap_or(0);
            for i in list.split(',').filter_map(|v| u16::from_str_radix(v.trim_start_matches("0x"), 16).ok()) {
                println!("  name {i:#x} = {}", program.name(k, i));
            }
        }
        for (k, script) in program.scripts.iter().enumerate() {
            let (code, err) = ppo::disassemble(&program, k);
            // Jumps must land on instruction starts.
            let starts: std::collections::HashSet<usize> = code.iter().map(|i| i.at).collect();
            let base = program.code_base[k];
            let misaligned = code
                .iter()
                .flat_map(|i| i.targets.iter())
                .filter(|&&t| t >= base && t < base + script.code.len() as u32 && !starts.contains(&((t - base) as usize)))
                .count();
            println!(
                "  #{k:2} {:<22} <- {:<22} base {:#06x} {:>6} bytes, {:>5} instrs, {:>3} names from {:>3}, {} misaligned jumps{}",
                ppo::short_name(&script.source),
                program.parent[k].map_or("-", |p| ppo::short_name(&program.scripts[p].source)),
                base,
                script.code.len(),
                code.len(),
                script.names.len(),
                script.name_base,
                misaligned,
                err.as_ref().map_or(String::new(), |e| format!("  STOPPED: {e}"))
            );
            if let Some(dir) = out {
                let mut f = std::fs::File::create(dir.join(format!("{k:02}_{}.txt", ppo::short_name(&script.source))))?;
                writeln!(f, "// {} (derives from {:?})\n// schema: {}\n", script.source, script.parent_source, script.schema)?;
                let labels: std::collections::HashSet<u32> = code.iter().flat_map(|i| i.targets.iter().copied()).collect();
                // State and handler entry points, by address.
                let mut marks: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
                for st in &script.states {
                    let name = program.name(k, st.name);
                    marks.insert(st.entry, format!("\n// ======== state {name}"));
                    for (h, &a) in st.handlers.iter().enumerate() {
                        if a != u32::MAX {
                            marks.entry(a).or_insert_with(|| format!("// ---- {name} handler {h}"));
                        }
                    }
                }
                writeln!(f, "// states: {}", script.states.iter().map(|st| program.name(k, st.name)).collect::<Vec<_>>().join(", "))?;
                for (i, &a) in script.functions.iter().enumerate() {
                    if a >= base && a < base + script.code.len() as u32 {
                        marks.entry(a).or_insert_with(|| format!("// ---- function #{i}"));
                    }
                }
                for ins in &code {
                    if let Some(m) = marks.get(&(base + ins.at as u32)) {
                        writeln!(f, "{m}")?;
                    }
                    if labels.contains(&(base + ins.at as u32)) {
                        writeln!(f, "L{:04x}:", ins.at)?;
                    }
                    writeln!(f, "    {:<20} {}", ins.mnemonic, ins.args)?;
                }
                if let Some(e) = err {
                    writeln!(f, "// STOPPED: {e}")?;
                }
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--model") {
        // `--model NAME`: per-submesh stats of the first entity with that name.
        let name = args.get(i + 1).map(String::as_str).unwrap_or("");
        if let Some(e) = level.entities.iter().find(|e| e.name == name) {
            let model = level.prm.model(e.primitive)?;
            println!("{} prim {:#x}: {} submeshes", e.name, e.primitive, model.submeshes.len());
            // Layer pairs with identical vertex counts: compare heights vertex by vertex.
            let lod0: Vec<_> = model.submeshes.iter().filter(|sm| sm.in_lod0()).collect();
            for (i, a) in lod0.iter().enumerate() {
                for b in &lod0[i + 1..] {
                    if a.vertices.len() == b.vertices.len() && a.texture != b.texture {
                        let d: Vec<f32> = a.vertices.iter().zip(&b.vertices).map(|(u, v)| u.pos[1] - v.pos[1]).collect();
                        let same_xz = a.vertices.iter().zip(&b.vertices).filter(|(u, v)| (u.pos[0] - v.pos[0]).abs() < 0.5 && (u.pos[2] - v.pos[2]).abs() < 0.5).count();
                        let below = d.iter().filter(|&&x| x < 0.0).count();
                        println!(
                            "  tex {} minus tex {}: {} verts share xz; dy min {:.2} max {:.2}; {} verts below",
                            a.texture, b.texture, same_xz, d.iter().cloned().fold(f32::MAX, f32::min), d.iter().cloned().fold(f32::MIN, f32::max), below
                        );
                    }
                }
            }
            for sm in &model.submeshes {
                let alphas: Vec<u8> = sm.vertices.iter().map(|v| v.color[3]).collect();
                let rgb: Vec<u8> = sm.vertices.iter().map(|v| v.color[0]).collect();
                let ys: Vec<f32> = sm.vertices.iter().map(|v| v.pos[1]).collect();
                let ny: Vec<f32> = sm.vertices.iter().map(|v| v.normal[1]).collect();
                let down = ny.iter().filter(|&&n| n < 0.0).count();
                println!("    normal y: {} down / {} up, min {:.2} max {:.2}", down, ny.len() - down, ny.iter().cloned().fold(1.0, f32::min), ny.iter().cloned().fold(-1.0, f32::max));
                // Winding vs normal: how many triangles' geometric normal disagrees with vertex normals.
                let flipped = sm.triangles.iter().filter(|t| {
                    let p = t.map(|i| bevy_math_vec(sm.vertices[i as usize].pos));
                    let n = sub(p[1], p[0]).cross_y(sub(p[2], p[0]));
                    let vn: f32 = t.iter().map(|&i| sm.vertices[i as usize].normal[1]).sum();
                    n * vn < 0.0
                }).count();
                println!("    triangles whose winding disagrees with their normals: {flipped} / {}", sm.triangles.len());
                let hist = |v: &[u8]| { let mut h = [0usize; 5]; for &a in v { h[(a as usize * 5 / 256).min(4)] += 1; } h };
                println!(
                    "  tex {:4} rs {:#010x} lod {:#x} blended {}: {} verts {} tris, y {:.1}..{:.1}, alpha hist {:?}, red hist {:?}",
                    sm.texture, sm.render_state, sm.lod_mask, sm.blended, sm.vertices.len(), sm.triangles.len(),
                    ys.iter().cloned().fold(f32::MAX, f32::min), ys.iter().cloned().fold(f32::MIN, f32::max), hist(&alphas), hist(&rgb)
                );
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--paths") {
        for name in args.iter().skip(i + 1) {
            for (idx, e) in level.entities.iter().enumerate().filter(|(_, e)| &e.name == name) {
                let wp = e.props.as_ref().and_then(|p| p.waypoints);
                println!("  [{idx}] {} type {:#x} path {:?} waypoints-ref {:?}", e.name, e.type_id, e.path, wp);
                if let Some(w) = wp {
                    let g = &level.entities[w];
                    println!("     -> [{w}] {} nodes {:?}", g.name, g.path.iter().map(|&n| &level.entities[n].name).collect::<Vec<_>>());
                }
            }
        }
    }

    if args.iter().any(|a| a == "--coplanar") {
        // Flat horizontal parts in world space (cm): (height mm, xz bounds, entity, texture, lod).
        let mut flats: Vec<(i64, [f32; 4], usize, u32, u8, bool)> = Vec::new();
        for (i, e) in level.entities.iter().enumerate() {
            if e.type_id != 0x200002 || !level.prm.is_record(e.primitive) {
                continue;
            }
            let Ok(m) = level.prm.model(e.primitive) else { continue };
            for sm in m.submeshes.iter().filter(|s| s.in_lod0() && s.texture != 0) {
                let ys: Vec<f32> = sm.vertices.iter().map(|v| v.pos[1]).collect();
                let (ylo, yhi) = ys.iter().fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
                if yhi - ylo > 0.5 || sm.vertices.len() < 3 {
                    continue;
                }
                // World xz via the entity's row-vector matrix, with the PRM x/z swap.
                let r = e.rotation;
                let (mut x0, mut x1, mut z0, mut z1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
                for v in &sm.vertices {
                    let q = [v.pos[2], v.pos[1], v.pos[0]];
                    let wx = q[0] * r[0][0] + q[1] * r[1][0] + q[2] * r[2][0] + e.position[0];
                    let wz = q[0] * r[0][2] + q[1] * r[1][2] + q[2] * r[2][2] + e.position[2];
                    x0 = x0.min(wx); x1 = x1.max(wx); z0 = z0.min(wz); z1 = z1.max(wz);
                }
                let wy = ylo + e.position[1];
                flats.push(((wy * 10.0).round() as i64, [x0, x1, z0, z1], i, sm.texture, sm.lod_mask, sm.blended));
            }
        }
        flats.sort_by_key(|f| f.0);
        let mut pairs = std::collections::BTreeMap::new();
        for (a, fa) in flats.iter().enumerate() {
            for fb in flats[a + 1..].iter().take_while(|fb| fb.0 == fa.0) {
                if fa.2 == fb.2 {
                    continue;
                }
                let (p, q) = (fa.1, fb.1);
                let ox = p[1].min(q[1]) - p[0].max(q[0]);
                let oz = p[3].min(q[3]) - p[2].max(q[2]);
                if ox > 50.0 && oz > 50.0 {
                    let key = (strip(&level.entities[fa.2].name), fa.3, fa.5, strip(&level.entities[fb.2].name), fb.3, fb.5);
                    *pairs.entry(key).or_insert(0) += 1;
                }
            }
        }
        let mut v: Vec<_> = pairs.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for ((na, ta, ba, nb, tb, bb), n) in v.iter().take(25) {
            println!("  {n:4}x  {na}[tex {ta}{}]  vs  {nb}[tex {tb}{}]", if *ba { " blend" } else { "" }, if *bb { " blend" } else { "" });
        }
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

    if let Some(i) = args.iter().position(|a| a == "--anim-names") {
        // `--anim-names [FILTER]`: every clip name per set (case-insensitive filter).
        let filter = args.get(i + 1).map(|f| f.to_lowercase()).unwrap_or_default();
        for set in &level.anm.sets {
            let mut names: Vec<_> = set.animations.keys().filter(|n| n.to_lowercase().contains(&filter)).collect();
            names.sort();
            if !names.is_empty() {
                println!("  {}: {}", set.path, names.iter().map(|n| format!("{n} ({:.1}s)", set.animations[*n].duration())).collect::<Vec<_>>().join(", "));
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--skeleton") {
        // `--skeleton ACTOR`: the bone hierarchy of that actor's model (the `Ground` entity
        // that follows it), with bind-pose offsets from the parent.
        let name = args.get(i + 1).map(String::as_str).unwrap_or("");
        if let Some(k) = level.entities.iter().position(|e| e.name == name) {
            let model = level.prm.skinned_model(level.entities[k + 1].primitive)?;
            for (j, b) in model.bones.iter().enumerate() {
                let parent = b.parent.map_or("-".to_owned(), |p| model.bones[p].name.clone());
                println!("  {j:2} {:<16} <- {:<16} t {:?}", b.name, parent, b.translation);
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--clip-bones") {
        // `--clip-bones NAME`: bones a clip animates (first set that has it).
        let name = args.get(i + 1).map(String::as_str).unwrap_or("");
        if let Some(a) = level.anm.sets.iter().find_map(|s| s.animations.get(name)) {
            println!("  {name}: {}", a.tracks.iter().map(|t| t.bone.as_str()).collect::<Vec<_>>().join(", "));
            for t in a.tracks.iter().filter(|t| t.bone == "GROUND" || t.bone == "PELVIS") {
                println!("    {} pos first {:?} last {:?}; rot first {:?} last {:?}", t.bone, t.positions.first(), t.positions.last(), t.rotations.first(), t.rotations.last());
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
        for (k, v) in level.strings.descriptions() {
            if filter.is_empty() || k.to_lowercase().contains(&filter.to_lowercase()) || v.to_lowercase().contains(&filter.to_lowercase()) {
                println!("  {k} (description) = {v}");
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
                if let Some(c) = &e.cloth {
                    println!("         cloth: anchors {:?}, wind {:?}", c.anchors, c.wind);
                }
            }
        }
    }
    if let Some(i) = args.iter().position(|a| a == "--sounds") {
        let pat = args.get(i + 1).map_or("", String::as_str).to_ascii_lowercase();
        let dir = Path::new("data/Scenes").join(id);
        let archive = hitman_clone::glacier::audio::AudioArchive::load(&dir.join(format!("{id}_MAIN.WHD")), &dir.join(format!("{id}_MAIN.WAV")), Path::new("data/streams.wav"))?;
        let mut names: Vec<_> = archive.entries.keys().filter(|n| n.to_ascii_lowercase().contains(&pat)).collect();
        names.sort();
        println!("  {} entries, {} matching", archive.entries.len(), names.len());
        for n in &names {
            println!("  {n}");
        }
        // `--sounds PATTERN --export DIR` writes the matching sounds out, decoded.
        if let Some(j) = args.iter().position(|a| a == "--export") {
            let out = Path::new(&args[j + 1]);
            std::fs::create_dir_all(out)?;
            for n in names {
                if let Some(bytes) = archive.decode(n) {
                    let ext = if bytes.starts_with(b"OggS") { "ogg" } else { "wav" };
                    std::fs::write(out.join(format!("{}.{ext}", n.trim_end_matches(".wav"))), bytes)?;
                }
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

/// Entity name without its instance suffix (`Ground_20x14_03_63` → `Ground_20x14`).
fn strip(n: &str) -> String {
    n.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_' || c == '#').to_owned()
}
