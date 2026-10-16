//! Measures how the knees and elbows bend in the original clips: the rotation of each joint
//! relative to its bind pose, as axis × angle in Bevy's frame (the P-conjugated bones).
use bevy::math::{Quat, Vec3};
use hitman_clone::glacier::{anm, level::Level};
fn q(r: [f32; 4]) -> Quat {
    let [x, y, z, w] = r;
    Quat::from_xyzw(-z, -y, -x, w).normalize()
}
fn main() -> anyhow::Result<()> {
    let level = Level::load(std::path::Path::new("data/Scenes"), "C02-1")?;
    let k = level.entities.iter().position(|e| e.name == "Hitman2").unwrap();
    let model = level.prm.skinned_model(level.entities[k + 1].primitive)?;
    let set = level.anm.sets.iter().find(|s| s.path.ends_with("Reg_Male.anm")).unwrap();
    for clip in ["Walk", "Die_Forward", "Die_Backward", "Run"] {
        let Some(a) = set.animations.get(clip) else { continue };
        for bone in ["LEFT_CALF", "RIGHT_CALF", "LEFT_FOREARM", "RIGHT_FOREARM"] {
            let b = model.bones.iter().find(|b| b.name == bone).unwrap();
            let Some(t) = a.tracks.iter().find(|t| t.bone == bone) else { continue };
            let (mut sum, mut lo, mut hi) = (Vec3::ZERO, f32::MAX, f32::MIN);
            for f in 0..a.frames {
                let rel = q(b.rotation).inverse() * q(anm::sample_rot(&t.rotations, f as f32));
                let (axis, angle) = rel.to_axis_angle();
                let v = axis * if angle > std::f32::consts::PI { angle - std::f32::consts::TAU } else { angle };
                sum += v;
                lo = lo.min(v.x);
                hi = hi.max(v.x);
            }
            println!("{clip:<13} {bone:<14} mean {:.2?}  x range {lo:.2}..{hi:.2}", sum / a.frames as f32);
        }
    }
    Ok(())
}
