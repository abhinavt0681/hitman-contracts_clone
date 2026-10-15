//! Scripted screenshots for checking renders without a human at the keyboard.
//!
//! `HITMAN_SCREENSHOT=out.png` saves one frame once the level is loaded, then exits.
//! `HITMAN_CAM="x,y,z,tx,ty,tz"` places the camera at x,y,z looking at tx,ty,tz (meters)
//! and turns off fog.
//! `HITMAN_START="x,y,z"` / `HITMAN_YAW=radians` override 47's spawn and facing.
//! `HITMAN_SHOT_FRAME=n` captures at frame n instead of the default.
//! `HITMAN_AUTOWALK=secs` holds W (plus Shift if `HITMAN_AUTORUN` is set) for that long,
//! to exercise 47's movement and collision. `HITMAN_AUTOKEY=D` presses D instead, which
//! strafes him across the camera's view (handy for checking the gait side-on).
//! `HITMAN_KEYS="E@0.2,E@2.6"` taps keys at given times (seconds after startup).

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};

/// Frames to wait so textures and pipelines finish uploading before capturing.
const SETTLE_FRAMES: u32 = 90;

pub struct ShotPlugin;

impl Plugin for ShotPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("HITMAN_CAM").is_some() {
            app.add_systems(PostStartup, place_camera);
        }
        if let Some(secs) = std::env::var("HITMAN_AUTOWALK").ok().and_then(|s| s.parse::<f32>().ok()) {
            app.insert_resource(AutoWalk(secs)).add_systems(PreUpdate, auto_walk.after(bevy::input::InputSystems));
        }
        if let Ok(spec) = std::env::var("HITMAN_KEYS") {
            let taps = spec
                .split(',')
                .filter_map(|t| {
                    let (k, at) = t.split_once('@')?;
                    let key = match k {
                        "E" => KeyCode::KeyE,
                        "W" => KeyCode::KeyW,
                        "G" => KeyCode::KeyG,
                        "C" => KeyCode::KeyC,
                        "Ctrl" => KeyCode::ControlLeft,
                        "Alt" => KeyCode::AltLeft,
                        "Enter" => KeyCode::Enter,
                        "1" => KeyCode::Digit1,
                        "2" => KeyCode::Digit2,
                        "3" => KeyCode::Digit3,
                        "4" => KeyCode::Digit4,
                        "5" => KeyCode::Digit5,
                        _ => return None,
                    };
                    Some((key, at.parse::<f32>().ok()?))
                })
                .collect();
            app.insert_resource(KeyTaps(taps)).add_systems(PreUpdate, tap_keys.after(bevy::input::InputSystems));
        }
        if let Some(path) = std::env::var_os("HITMAN_SCREENSHOT") {
            app.insert_resource(ShotPath(path.into(), 0)).add_systems(Update, capture);
        }
    }
}

#[derive(Resource)]
struct ShotPath(std::path::PathBuf, u32);

pub fn env_vec3(var: &str) -> Option<Vec3> {
    let v: Vec<f32> = std::env::var(var).ok()?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    if let [x, y, z] = v[..] { Some(Vec3::new(x, y, z)) } else { None }
}

fn place_camera(mut commands: Commands, mut cam: Query<(Entity, &mut Transform), With<Camera3d>>) {
    let v: Vec<f32> = std::env::var("HITMAN_CAM").unwrap_or_default().split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let Ok((id, mut t)) = cam.single_mut() else { return };
    // Inspection shots want to see everything, not atmosphere; flying detaches from 47.
    commands.entity(id).remove::<bevy::pbr::DistanceFog>().insert(bevy::camera_controller::free_camera::FreeCamera::default());
    if let [x, y, z, tx, ty, tz] = v[..] {
        *t = Transform::from_xyz(x, y, z).looking_at(Vec3::new(tx, ty, tz), Vec3::Y);
    } else {
        warn!("HITMAN_CAM wants 6 comma-separated numbers");
    }
}

#[derive(Resource)]
struct AutoWalk(f32);

#[derive(Resource)]
struct KeyTaps(Vec<(KeyCode, f32)>);

fn tap_keys(time: Res<Time>, mut taps: ResMut<KeyTaps>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    let now = time.elapsed_secs();
    for (key, at) in taps.0.iter_mut() {
        if *at >= 0.0 && now >= *at {
            keys.press(*key);
            *at = -1.0;
        } else if *at < 0.0 {
            keys.release(*key);
        }
    }
}

fn auto_walk(time: Res<Time>, mut walk: ResMut<AutoWalk>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    walk.0 -= time.delta_secs();
    let key = match std::env::var("HITMAN_AUTOKEY").as_deref() {
        Ok("D") => KeyCode::KeyD,
        Ok("A") => KeyCode::KeyA,
        Ok("S") => KeyCode::KeyS,
        Ok("E") => KeyCode::KeyE,
        _ => KeyCode::KeyW,
    };
    if walk.0 > 0.0 {
        keys.press(key);
        if std::env::var_os("HITMAN_AUTORUN").is_some() {
            keys.press(KeyCode::ShiftLeft);
        }
    } else {
        keys.release(key);
        keys.release(KeyCode::ShiftLeft);
    }
}

fn capture(
    mut commands: Commands,
    mut shot: ResMut<ShotPath>,
    mut exit: MessageWriter<AppExit>,
    player: Query<&Transform, With<crate::player::Player>>,
    mission: Option<Res<crate::game::Mission>>,
    world: Res<crate::collision::WorldCollision>,
    names: Res<crate::view::EntityNames>,
) {
    shot.1 += 1;
    let at = std::env::var("HITMAN_SHOT_FRAME").ok().and_then(|s| s.parse().ok()).unwrap_or(SETTLE_FRAMES);
    if shot.1 == at {
        if let Ok(t) = player.single() {
            info!("47 at {:?}", t.translation);
            if let Some(m) = &mission {
                info!(
                    "mission: prompt {:?}, messages {:?}, health {:.0}, outfit {:?}, alarm {}, objectives {:?}",
                    m.prompt,
                    m.messages.iter().map(|x| &x.0).collect::<Vec<_>>(),
                    m.health,
                    m.outfit,
                    m.alarm,
                    m.objectives.iter().map(|o| o.done).collect::<Vec<_>>()
                );
            }
            if let Some(path) = std::env::var_os("HITMAN_SLICE") {
                write_slice(&world, t.translation, path.as_ref());
            }
            for h in [0.75, 1.0, 1.5] {
                let near: Vec<&str> = world.walls_near(t.translation + Vec3::Y * h, 0.35).iter().map(|&i| names.0[i as usize].as_str()).collect();
                info!("  walls touching at {h}m: {near:?}");
            }
        }
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(shot.0.clone()));
    } else if shot.1 == at + 15 {
        exit.write(AppExit::Success);
    }
}

/// Top-down map of wall triangles around `center` at knee-to-chest height, 10 cm per pixel,
/// +X right and +Z down; 47 is the grey dot. Plain PGM so no encoder is needed.
fn write_slice(world: &crate::collision::WorldCollision, center: Vec3, path: &std::path::Path) {
    const HALF: i32 = 300;
    let mut px = vec![255u8; (HALF * 2 * HALF * 2) as usize];
    for y in -HALF..HALF {
        for x in -HALF..HALF {
            let p = center + Vec3::new(x as f32 * 0.1, 0.0, y as f32 * 0.1);
            let wall = [0.75, 1.5].iter().any(|h| !world.walls_near(p + Vec3::Y * h, 0.07).is_empty());
            let dot = x * x + y * y < 9;
            px[((y + HALF) * HALF * 2 + x + HALF) as usize] = if dot { 128 } else if wall { 0 } else { 255 };
        }
    }
    let mut out = format!("P5\n{} {}\n255\n", HALF * 2, HALF * 2).into_bytes();
    out.extend_from_slice(&px);
    if let Err(e) = std::fs::write(path, out) {
        warn!("slice: {e}");
    }
}
