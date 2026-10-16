//! Finds the loopable stretch of `Climb_Ladder`: frames a < b whose poses (every bone but the
//! root) match best, and how far the root rises between them.
use bevy::math::Quat;
use hitman_clone::glacier::{anm, level::Level};
fn q(r: [f32; 4]) -> Quat {
    let [x, y, z, w] = r;
    Quat::from_xyzw(-z, -y, -x, w).normalize()
}
fn main() -> anyhow::Result<()> {
    let level = Level::load(std::path::Path::new("data/Scenes"), "C02-1")?;
    let set = level.anm.sets.iter().find(|s| s.path.ends_with("Hero.anm")).unwrap();
    let a = &set.animations["Climb_Ladder"];
    let n = a.frames as usize;
    let pose = |f: usize| -> Vec<Quat> { a.tracks.iter().filter(|t| t.bone != "GROUND" && !t.bone.contains("FINGER")).map(|t| q(anm::sample_rot(&t.rotations, f as f32))).collect() };
    let height = |f: usize| a.tracks.iter().find(|t| t.bone == "GROUND").map_or(0.0, |t| anm::sample_pos(&t.positions, f as f32)[1]);
    let poses: Vec<_> = (0..=n).map(pose).collect();
    let mut best = Vec::new();
    for i in 0..=n {
        for j in i + 8..=n {
            let d: f32 = poses[i].iter().zip(&poses[j]).map(|(x, y)| x.angle_between(*y)).sum::<f32>() / poses[i].len() as f32;
            best.push((d, i, j));
        }
    }
    best.sort_by(|x, y| x.0.total_cmp(&y.0));
    for (d, i, j) in best.iter().take(12) {
        println!("frames {i:2}..{j:2}: mean bone diff {:.3} rad, root rise {:.1} cm", d, height(*j) - height(*i));
    }
    for f in (0..=n).step_by(4) {
        println!("frame {f:2} root height {:.1}", height(f));
    }
    Ok(())
}
