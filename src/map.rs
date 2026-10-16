//! The map screen (M / Tab), drawn from the original's own map floor plans: the
//! `C02-1__MAP_*` meshes the level places over the real buildings, seen from straight above
//! by an orthographic camera that only renders the map layer. Markers show 47, the targets
//! and the guards (coloured by how alarmed they are).

use crate::{
    flow::Phase,
    game::{Actor, Kind, Mind},
    player::{OrbitCamera, Player},
    view::MAP_LAYER,
};
use bevy::{
    camera::{visibility::RenderLayers, ScalingMode},
    input::mouse::AccumulatedMouseScroll,
    prelude::*,
};

/// Height the map camera looks down from, and where markers float (above every floor plan).
const CAMERA_HEIGHT: f32 = 400.0;
const MARKER_HEIGHT: f32 = 300.0;

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_map_camera)
            .add_systems(OnEnter(Phase::Map), open_map)
            .add_systems(OnExit(Phase::Map), close_map)
            .add_systems(Update, (pan_and_zoom, place_markers).run_if(in_state(Phase::Map)));
    }
}

#[derive(Component)]
struct MapCamera;

#[derive(Component)]
struct MapUi;

/// A map marker following an actor (or 47, with `None`).
#[derive(Component)]
struct Marker(Option<Entity>);

/// Shared marker meshes and materials.
#[derive(Resource)]
struct MarkerAssets {
    arrow: Handle<Mesh>,
    dot: Handle<Mesh>,
    hitman: Handle<StandardMaterial>,
    target: Handle<StandardMaterial>,
    calm: Handle<StandardMaterial>,
    wary: Handle<StandardMaterial>,
    hostile: Handle<StandardMaterial>,
    dead: Handle<StandardMaterial>,
}

fn spawn_map_camera(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.spawn((
        MapCamera,
        Camera3d::default(),
        Camera { order: 1, is_active: false, clear_color: ClearColorConfig::Custom(Color::srgb(0.02, 0.03, 0.035)), ..default() },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical { viewport_height: 90.0 },
            near: 0.0,
            far: 2000.0,
            ..OrthographicProjection::default_3d()
        }),
        // North (−Z) up.
        Transform::from_xyz(0.0, CAMERA_HEIGHT, 0.0).looking_at(Vec3::ZERO, Vec3::NEG_Z),
        RenderLayers::layer(MAP_LAYER),
        Name::new("Map camera"),
    ));
    // An arrow pointing along −Z, lying flat.
    let arrow = Mesh::from(Triangle3d::new(Vec3::new(0.0, 0.0, -1.4), Vec3::new(-0.8, 0.0, 0.9), Vec3::new(0.8, 0.0, 0.9)));
    let flat = |c: Color| StandardMaterial { base_color: c, unlit: true, cull_mode: None, fog_enabled: false, ..default() };
    commands.insert_resource(MarkerAssets {
        arrow: meshes.add(arrow),
        dot: meshes.add(Circle::new(0.8).mesh().build().rotated_by(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2))),
        hitman: materials.add(flat(Color::WHITE)),
        target: materials.add(flat(Color::srgb(1.0, 0.15, 0.1))),
        calm: materials.add(flat(Color::srgb(0.35, 0.75, 0.4))),
        wary: materials.add(flat(Color::srgb(1.0, 0.75, 0.2))),
        hostile: materials.add(flat(Color::srgb(1.0, 0.3, 0.1))),
        dead: materials.add(flat(Color::srgb(0.35, 0.35, 0.35))),
    });
}

#[allow(clippy::too_many_arguments)]
fn open_map(
    mut commands: Commands,
    assets: Res<MarkerAssets>,
    mut map_cam: Query<(Entity, &mut Camera, &mut Transform), With<MapCamera>>,
    mut main_cam: Query<&mut Camera, (With<OrbitCamera>, Without<MapCamera>)>,
    player: Query<&GlobalTransform, With<Player>>,
    actors: Query<(Entity, &Actor)>,
    mission: Option<Res<crate::game::Mission>>,
) {
    let Ok((cam_id, mut cam, mut ct)) = map_cam.single_mut() else { return };
    cam.is_active = true;
    for mut c in &mut main_cam {
        c.is_active = false;
    }
    if let Ok(p) = player.single() {
        ct.translation = Vec3::new(p.translation().x, CAMERA_HEIGHT, p.translation().z);
    }
    let layer = RenderLayers::layer(MAP_LAYER);
    commands.spawn((Marker(None), Mesh3d(assets.arrow.clone()), MeshMaterial3d(assets.hitman.clone()), Transform::default(), layer.clone()));
    for (e, a) in &actors {
        if a.kind == Kind::Civilian {
            continue;
        }
        commands.spawn((Marker(Some(e)), Mesh3d(assets.dot.clone()), MeshMaterial3d(assets.calm.clone()), Transform::default(), layer.clone()));
    }
    commands
        .spawn((
            MapUi,
            UiTargetCamera(cam_id),
            Node { position_type: PositionType::Absolute, left: px(20.0), bottom: px(16.0), ..default() },
            Text::new(mission.as_deref().map(|m| crate::hud::objectives_text(m) + "\n\n").unwrap_or_default() + "MAP     White: 47   Red: targets   Green/amber/orange: guards (calm / suspicious / hunting)\nWheel: zoom   WASD: pan   M / Tab / Esc: back"),
            TextFont { font_size: FontSize::Px(15.0), ..default() },
            TextColor(Color::srgb(0.75, 0.8, 0.78)),
        ));
}

fn close_map(
    mut commands: Commands,
    mut map_cam: Query<&mut Camera, With<MapCamera>>,
    mut main_cam: Query<&mut Camera, (With<OrbitCamera>, Without<MapCamera>)>,
    stale: Query<Entity, Or<(With<Marker>, With<MapUi>)>>,
) {
    for mut c in &mut map_cam {
        c.is_active = false;
    }
    for mut c in &mut main_cam {
        c.is_active = true;
    }
    for e in &stale {
        commands.entity(e).despawn();
    }
}

fn pan_and_zoom(
    time: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cam: Query<(&mut Transform, &mut Projection), With<MapCamera>>,
) {
    let Ok((mut t, mut projection)) = cam.single_mut() else { return };
    let Projection::Orthographic(ortho) = &mut *projection else { return };
    let ScalingMode::FixedVertical { viewport_height } = &mut ortho.scaling_mode else { return };
    *viewport_height = (*viewport_height * (1.0 - scroll.delta.y * 0.1)).clamp(20.0, 600.0);
    let mut pan = Vec3::ZERO;
    for (key, dir) in [(KeyCode::KeyW, Vec3::NEG_Z), (KeyCode::KeyS, Vec3::Z), (KeyCode::KeyA, Vec3::NEG_X), (KeyCode::KeyD, Vec3::X)] {
        if keys.pressed(key) {
            pan += dir;
        }
    }
    t.translation += pan * *viewport_height * 0.6 * time.delta_secs();
}

fn place_markers(
    assets: Res<MarkerAssets>,
    cam: Query<&Projection, With<MapCamera>>,
    player: Query<&GlobalTransform, With<Player>>,
    orbit: Query<&OrbitCamera>,
    actors: Query<(&GlobalTransform, &Actor)>,
    mut markers: Query<(&Marker, &mut Transform, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    // Markers keep the same size on screen at any zoom.
    let size = match cam.single() {
        Ok(Projection::Orthographic(OrthographicProjection { scaling_mode: ScalingMode::FixedVertical { viewport_height }, .. })) => viewport_height / 60.0,
        _ => 1.5,
    };
    for (marker, mut t, mut material) in &mut markers {
        let (at, facing) = match marker.0 {
            None => {
                let Ok(p) = player.single() else { continue };
                // The arrow points where 47 looks (the camera's heading).
                (p.translation(), Quat::from_rotation_y(orbit.single().map_or(0.0, |o| o.yaw)))
            }
            Some(e) => {
                let Ok((gt, actor)) = actors.get(e) else { continue };
                material.0 = if !actor.alive {
                    assets.dead.clone()
                } else if actor.kind == Kind::Target {
                    assets.target.clone()
                } else if actor.alerted {
                    assets.hostile.clone()
                } else if actor.mind != Mind::Calm || actor.suspicion > 0.3 {
                    assets.wary.clone()
                } else {
                    assets.calm.clone()
                };
                (gt.translation(), Quat::IDENTITY)
            }
        };
        let (yaw, _, _) = facing.to_euler(EulerRot::YXZ);
        t.translation = Vec3::new(at.x, MARKER_HEIGHT + if marker.0.is_none() { 1.0 } else { 0.0 }, at.z);
        t.rotation = Quat::from_rotation_y(yaw);
        t.scale = Vec3::splat(size);
    }
}
