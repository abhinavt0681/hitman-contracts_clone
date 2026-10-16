//! Agent 47: third-person movement over the static level collision.
//!
//! Controls: WASD move (walk), Shift run, mouse orbit (click to capture, Esc to release),
//! F1 toggles between 47 and the free-fly debug camera.

use crate::collision::WorldCollision;
use bevy::{
    camera_controller::free_camera::{FreeCamera, FreeCameraState},
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

const WALK_SPEED: f32 = 1.6;
const RUN_SPEED: f32 = 4.2;
const SNEAK_SPEED: f32 = 0.95;
const DRAG_SPEED: f32 = 1.05;
const TURN_RATE: f32 = 12.0;
const GRAVITY: f32 = 18.0;
const RADIUS: f32 = 0.3;
/// Highest ledge 47 walks up without jumping (stairs, curbs).
const STEP_HEIGHT: f32 = 0.45;
/// Below this vertical normal component, a surface is too steep to stand on.
const MIN_FLOOR_NY: f32 = 0.55;
const EYE_HEIGHT: f32 = 1.65;
const CROUCH_EYE_HEIGHT: f32 = 1.15;
const CAMERA_DISTANCE: f32 = 3.2;
const CAMERA_SHOULDER: f32 = 0.45;
const AIM_SHOULDER: f32 = 0.35;
const AIM_DISTANCE: f32 = 1.6;
const FOV: f32 = std::f32::consts::FRAC_PI_4;
const AIM_FOV: f32 = 0.55;
const MOUSE_SENSITIVITY: f32 = 0.0025;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (toggle_mode, grab_cursor, (orbit_input, move_player, follow_camera).chain().run_if(playing))
                .run_if(in_state(crate::flow::Phase::Playing)),
        );
    }
}

#[derive(Component)]
pub struct Player {
    pub vertical_speed: f32,
    pub grounded: bool,
    /// Root of the skinned model, whose `Gait` follows 47's speed.
    pub model: Option<Entity>,
    pub speed: f32,
    pub crouching: bool,
    pub crouch_toggle: bool,
}

impl Player {
    /// Current ground speed in m/s.
    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Root entity of 47's skinned model (where his `Animator` lives).
    pub fn model(&self) -> Entity {
        self.model.unwrap_or(Entity::PLACEHOLDER)
    }
}

/// Orbit state for the third-person camera.
#[derive(Component)]
pub struct OrbitCamera {
    pub yaw: f32,
    pitch: f32,
    /// 0 = normal, 1 = aiming over the shoulder (set by the weapons code).
    pub aim: f32,
}

fn playing(cam: Query<(), (With<OrbitCamera>, Without<FreeCamera>)>) -> bool {
    !cam.is_empty()
}

/// Spawns 47 at `feet`. His skinned model is attached by the caller under the returned
/// `model` entity; without one he falls back to a stand-in figure.
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, feet: Vec3, model: Option<Transform>) -> Entity {
    let player = commands
        .spawn((
            Transform::from_translation(feet),
            Visibility::default(),
            Player {
                vertical_speed: 0.0,
                grounded: false,
                model: None,
                speed: 0.0,
                crouching: false,
                crouch_toggle: false,
            },
            Name::new("47"),
        ))
        .id();
    if let Some(model_transform) = model {
        let m = commands.spawn((model_transform, Visibility::default(), Name::new("47 model"))).id();
        commands.entity(player).add_child(m);
        commands.entity(player).insert(Player {
            vertical_speed: 0.0,
            grounded: false,
            model: Some(m),
            speed: 0.0,
            crouching: false,
            crouch_toggle: false,
        });
        return m;
    }
    let suit = materials.add(StandardMaterial { base_color: Color::srgb(0.05, 0.05, 0.06), perceptual_roughness: 0.6, ..default() });
    let skin = materials.add(Color::srgb(0.85, 0.7, 0.6));
    commands.entity(player).with_children(|c| {
        c.spawn((Mesh3d(meshes.add(Capsule3d::new(0.24, 1.1))), MeshMaterial3d(suit), Transform::from_xyz(0.0, 0.85, 0.0)));
        c.spawn((Mesh3d(meshes.add(Sphere::new(0.12))), MeshMaterial3d(skin), Transform::from_xyz(0.0, 1.62, 0.0)));
    });
    player
}

impl OrbitCamera {
    /// Turns the camera to look along `fwd` (horizontal).
    pub fn look_along(&mut self, fwd: Vec3) {
        self.yaw = (-fwd.x).atan2(-fwd.z);
    }
}

pub fn orbit_camera() -> OrbitCamera {
    let yaw = std::env::var("HITMAN_YAW").ok().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    OrbitCamera { yaw, pitch: -0.25, aim: 0.0 }
}

fn toggle_mode(mut commands: Commands, keys: Res<ButtonInput<KeyCode>>, cam: Query<(Entity, Has<FreeCamera>), With<OrbitCamera>>) {
    if !keys.just_pressed(KeyCode::F1) {
        return;
    }
    let Ok((id, flying)) = cam.single() else { return };
    if flying {
        commands.entity(id).remove::<(FreeCamera, FreeCameraState)>();
    } else {
        commands.entity(id).insert(FreeCamera { walk_speed: 8.0, run_speed: 40.0, ..default() });
    }
}

fn grab_cursor(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    flying: Query<(), With<FreeCamera>>,
) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    if !flying.is_empty() {
        return; // FreeCamera manages the cursor itself
    }
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn orbit_input(motion: Res<AccumulatedMouseMotion>, cursor: Query<&CursorOptions, With<PrimaryWindow>>, mut cam: Query<&mut OrbitCamera>) {
    let Ok(mut orbit) = cam.single_mut() else { return };
    if cursor.single().is_ok_and(|c| c.grab_mode == CursorGrabMode::None) {
        return;
    }
    orbit.yaw -= motion.delta.x * MOUSE_SENSITIVITY;
    orbit.pitch = (orbit.pitch - motion.delta.y * MOUSE_SENSITIVITY).clamp(-1.3, 0.9);
}

fn move_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    world: Res<WorldCollision>,
    cam: Query<&OrbitCamera>,
    mut player: Query<(&mut Transform, &mut Player)>,
    mut gaits: Query<&mut crate::character::Gait>,
    mission: Option<Res<crate::game::Mission>>,
) {
    let (Ok(orbit), Ok((mut t, mut p))) = (cam.single(), player.single_mut()) else { return };
    let dt = time.delta_secs().min(0.05);

    // Busy (strangling, changing clothes) or the mission is over: no walking.
    let frozen = mission.as_ref().is_some_and(|m| m.busy > 0.0 || m.outcome.is_some());
    let is_dragging = mission.as_ref().is_some_and(|m| m.dragging_body.is_some());
    let mut input = Vec2::ZERO;
    for (key, dir) in [(KeyCode::KeyW, Vec2::Y), (KeyCode::KeyS, -Vec2::Y), (KeyCode::KeyA, -Vec2::X), (KeyCode::KeyD, Vec2::X)] {
        if keys.pressed(key) && !frozen {
            input += dir;
        }
    }
    let forward = Vec3::new(-orbit.yaw.sin(), 0.0, -orbit.yaw.cos());
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let wish = (forward * input.y + right * input.x).normalize_or_zero();

    if keys.just_pressed(KeyCode::KeyC) {
        p.crouch_toggle = !p.crouch_toggle;
    }
    let hold_sneak = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight) || keys.pressed(KeyCode::AltLeft);
    let sneaking = (p.crouch_toggle || hold_sneak) && !is_dragging;
    p.crouching = sneaking;

    let aiming = orbit.aim > 0.5;
    let speed = if is_dragging {
        DRAG_SPEED
    } else if aiming {
        WALK_SPEED * 0.6
    } else if sneaking {
        SNEAK_SPEED
    } else if keys.pressed(KeyCode::ShiftLeft) {
        RUN_SPEED
    } else {
        WALK_SPEED
    };

    // Horizontal move, then push out of walls at knee, waist and chest height.
    let mut pos = t.translation + wish * speed * dt;
    for _ in 0..4 {
        let mut max_push = Vec3::ZERO;
        // Knee, waist, chest and head: low overhangs (the plane's belly) block him too.
        for h in [STEP_HEIGHT + RADIUS, 1.0, 1.5, 1.8] {
            let push = world.push_out_horizontal(pos + Vec3::Y * h, RADIUS);
            if push.length_squared() > max_push.length_squared() {
                max_push = push;
            }
        }
        if max_push == Vec3::ZERO {
            break;
        }
        pos += max_push;
    }

    // Vertical: gravity, then snap to the floor found by a ray from above the step height.
    p.vertical_speed -= GRAVITY * dt;
    let probe_from = pos + Vec3::Y * (STEP_HEIGHT + 0.05);
    let fall = (-p.vertical_speed * dt).max(0.0);
    let reach = STEP_HEIGHT + 0.05 + if p.grounded { STEP_HEIGHT } else { fall.max(0.25) };

    // Multi-point probe: check center, then 4 foot perimeter offsets to prevent slipping through triangle seams
    let mut ground_hit = world.raycast(probe_from, Vec3::NEG_Y, reach);
    if ground_hit.as_ref().is_none_or(|h| h.normal.y < MIN_FLOOR_NY) {
        for offset in [
            Vec3::new(0.08, 0.0, 0.0),
            Vec3::new(-0.08, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.08),
            Vec3::new(0.0, 0.0, -0.08),
        ] {
            if let Some(h) = world.raycast(probe_from + offset, Vec3::NEG_Y, reach) {
                if h.normal.y >= MIN_FLOOR_NY {
                    ground_hit = Some(h);
                    break;
                }
            }
        }
    }

    match ground_hit {
        Some(hit) if hit.normal.y >= MIN_FLOOR_NY && p.vertical_speed <= 0.0 => {
            pos.y = probe_from.y - hit.distance;
            p.vertical_speed = 0.0;
            p.grounded = true;
        }
        _ => {
            pos.y += p.vertical_speed * dt;
            p.grounded = false;
        }
    }
    // Fell out of the world: put him back where he was.
    if pos.y < -200.0 {
        pos = t.translation;
        p.vertical_speed = 0.0;
    }
    let moved = (pos - t.translation).with_y(0.0).length() / dt.max(1e-4);
    if let Some(mut gait) = p.model.and_then(|m| gaits.get_mut(m).ok()) {
        gait.speed = if p.grounded { moved } else { 0.0 };
        gait.crouching = sneaking;
        gait.dragging = is_dragging;
    }
    p.speed = moved;
    t.translation = pos;

    if aiming {
        // Aiming: face where the camera looks and strafe.
        let target = Transform::IDENTITY.looking_to(forward, Vec3::Y).rotation;
        t.rotation = t.rotation.slerp(target, (TURN_RATE * 2.0 * dt).min(1.0));
    } else if wish != Vec3::ZERO {
        let target = Transform::IDENTITY.looking_to(wish, Vec3::Y).rotation;
        t.rotation = t.rotation.slerp(target, (TURN_RATE * dt).min(1.0));
    }

}

fn follow_camera(
    world: Res<WorldCollision>,
    player: Query<(&Transform, &Player), Without<OrbitCamera>>,
    mut cam: Query<(&mut Transform, &OrbitCamera, &mut Projection)>,
) {
    let (Ok((pt, p)), Ok((mut ct, orbit, mut projection))) = (player.single(), cam.single_mut()) else { return };
    let rot = Quat::from_euler(EulerRot::YXZ, orbit.yaw, orbit.pitch, 0.0);
    let eye_h = if p.crouching { CROUCH_EYE_HEIGHT } else { EYE_HEIGHT };
    // Aiming pulls the camera in over the right shoulder and narrows the view.
    let a = orbit.aim * orbit.aim * (3.0 - 2.0 * orbit.aim);
    let shoulder = CAMERA_SHOULDER.lerp(AIM_SHOULDER, a);
    let distance = CAMERA_DISTANCE.lerp(AIM_DISTANCE, a);
    if let Projection::Perspective(persp) = &mut *projection {
        persp.fov = FOV.lerp(AIM_FOV, a);
    }
    let pivot = pt.translation + Vec3::Y * eye_h + rot * Vec3::X * shoulder;
    let back = rot * Vec3::Z;
    // Spring arm: pull in when level geometry is between 47 and the camera.
    let dist = world.raycast(pivot, back, distance).map_or(distance, |h| (h.distance - 0.2).max(0.3));
    ct.translation = pivot + back * dist;
    ct.rotation = rot;
}
