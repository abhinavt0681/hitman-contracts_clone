//! Scripted screenshots for checking renders without a human at the keyboard.
//!
//! `HITMAN_SCREENSHOT=out.png` saves one frame once the level is loaded, then exits.
//! `HITMAN_CAM="x,y,z,tx,ty,tz"` places the camera at x,y,z looking at tx,ty,tz (meters)
//! and turns off fog.
//! `HITMAN_START="x,y,z"` / `HITMAN_YAW=radians` override 47's spawn and facing.
//! `HITMAN_SHOT_FRAME=n` captures at frame n instead of the default.
//! `HITMAN_AUTOWALK=secs[@start]` holds W (plus Shift if `HITMAN_AUTORUN` is set) for that long,
//! from `start` seconds in (default 0),
//! to exercise 47's movement and collision. `HITMAN_AUTOKEY=D` presses D instead, which
//! strafes him across the camera's view (handy for checking the gait side-on).
//! `HITMAN_KEYS="E@0.2,E@2.6,RMB@3-5,LMB@4"` taps keys or mouse buttons at given times
//! (seconds after startup), or holds them over a range.

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
        if let Some((secs, start)) = std::env::var("HITMAN_AUTOWALK").ok().and_then(|s| {
            let (secs, start) = s.split_once('@').unwrap_or((&s, "0"));
            Some((secs.parse::<f32>().ok()?, start.parse::<f32>().ok()?))
        }) {
            app.insert_resource(AutoWalk(secs, start)).add_systems(PreUpdate, auto_walk.after(bevy::input::InputSystems));
        }
        if let Ok(spec) = std::env::var("HITMAN_KEYS") {
            let taps = spec
                .split(',')
                .filter_map(|t| {
                    let (k, at) = t.split_once('@')?;
                    let press = match k {
                        "E" => Press::Key(KeyCode::KeyE),
                        "W" => Press::Key(KeyCode::KeyW),
                        "G" => Press::Key(KeyCode::KeyG),
                        "C" => Press::Key(KeyCode::KeyC),
                        "F" => Press::Key(KeyCode::KeyF),
                        "R" => Press::Key(KeyCode::KeyR),
                        "Ctrl" => Press::Key(KeyCode::ControlLeft),
                        "Alt" => Press::Key(KeyCode::AltLeft),
                        "Enter" => Press::Key(KeyCode::Enter),
                        "1" => Press::Key(KeyCode::Digit1),
                        "2" => Press::Key(KeyCode::Digit2),
                        "3" => Press::Key(KeyCode::Digit3),
                        "4" => Press::Key(KeyCode::Digit4),
                        "5" => Press::Key(KeyCode::Digit5),
                        "6" => Press::Key(KeyCode::Digit6),
                        "LMB" => Press::Mouse(MouseButton::Left),
                        "RMB" => Press::Mouse(MouseButton::Right),
                        _ => return None,
                    };
                    // `K@t` taps at t; `K@a-b` holds from a to b.
                    let (start, end) = match at.split_once('-') {
                        Some((a, b)) => (a.parse::<f32>().ok()?, b.parse::<f32>().ok()?),
                        None => (at.parse::<f32>().ok()?, 0.0),
                    };
                    Some((press, start, end))
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

fn place_camera(mut commands: Commands, mut cam: Query<(Entity, &mut Transform), With<crate::player::OrbitCamera>>) {
    let v: Vec<f32> = std::env::var("HITMAN_CAM").unwrap_or_default().split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let Ok((id, mut t)) = cam.single_mut() else { return };
    // Inspection shots want to see everything, not atmosphere; flying detaches from 47.
    // `HITMAN_CAM_FOG=1` keeps the fog (for comparing looks from a fixed viewpoint).
    if std::env::var_os("HITMAN_CAM_FOG").is_none() {
        commands.entity(id).remove::<bevy::pbr::DistanceFog>();
    }
    commands.entity(id).insert(bevy::camera_controller::free_camera::FreeCamera::default());
    if let [x, y, z, tx, ty, tz] = v[..] {
        *t = Transform::from_xyz(x, y, z).looking_at(Vec3::new(tx, ty, tz), Vec3::Y);
    } else {
        warn!("HITMAN_CAM wants 6 comma-separated numbers");
    }
}

#[derive(Resource)]
struct AutoWalk(f32, f32);

#[derive(Resource)]
/// Scripted input: a key or mouse button, pressed at `start` and released at `end` (one
/// frame for a tap).
struct KeyTaps(Vec<(Press, f32, f32)>);

#[derive(Clone, Copy)]
enum Press {
    Key(KeyCode),
    Mouse(MouseButton),
}

fn tap_keys(time: Res<Time<Real>>, mut taps: ResMut<KeyTaps>, mut keys: ResMut<ButtonInput<KeyCode>>, mut mouse: ResMut<ButtonInput<MouseButton>>) {
    let now = time.elapsed_secs();
    for (press, start, end) in taps.0.iter_mut() {
        let down = now >= *start && (now < *end || *start >= 0.0);
        match (*press, down) {
            (Press::Key(k), true) => keys.press(k),
            (Press::Key(k), false) => keys.release(k),
            (Press::Mouse(b), true) => mouse.press(b),
            (Press::Mouse(b), false) => mouse.release(b),
        }
        // A tap lasts one frame.
        if down && *end <= *start {
            *start = -1.0;
            *end = -1.0;
        }
    }
}

fn auto_walk(time: Res<Time>, mut walk: ResMut<AutoWalk>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    if walk.1 > 0.0 {
        walk.1 -= time.delta_secs();
        return;
    }
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
