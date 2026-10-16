//! Ragdolls: the dead go limp, as in the original.
//!
//! A character killed by a shot drops at once, pushed by the round. One killed some other way
//! (wire, poison, a cut throat) plays the original death clip first and goes limp when it ends,
//! so the body settles where the clip left it. Either way thirteen capsule bodies (pelvis,
//! torso, head, upper arms, forearms, thighs, calves, feet) are spawned on the skeleton's
//! current pose and jointed, and from then on Rapier poses those bones; the rest (spine,
//! clavicles, hands, face) ride along with their last local pose.
//!
//! Knees and elbows are hinges about the bone's local Y axis, bending only one way: measured
//! from the original `Walk`, `Run` and `Die_*` clips, where they turn purely about that axis,
//! always negatively, from straight in the bind pose (`examples/hinge.rs`). Hips, shoulders,
//! spine, neck and ankles are limited cones around the pose the body died in.
//!
//! The level is one fixed triangle-mesh collider built from `WorldCollision`. Dragging a body
//! (G) pulls it by the wrist or ankle nearest 47's hand.
//!
//! `HITMAN_PHYSICS_DEBUG` draws the colliders. `HITMAN_KILL="Name@secs"` (testing) kills that
//! character at that time with a shot from 47's direction, to watch the fall.

use crate::{
    character::{Animator, Rig},
    collision::WorldCollision,
    game::{Actor, Mission},
    player::Player,
};
use bevy::{prelude::*, transform::TransformSystems};
use bevy_rapier3d::prelude::*;

/// Push (N·s) a killing shot gives the body part it hits.
pub const SHOT_PUSH: f32 = 30.0;
/// Bodies are about as dense as water.
const DENSITY: f32 = 985.0;
/// Friction in the joints (motor damping towards standing still), so bodies don't flop
/// like jelly.
const JOINT_FRICTION: f32 = 0.4;
/// How quickly a grabbed wrist or ankle is pulled to 47's hand (per second).
const GRIP_PULL: f32 = 10.0;
/// Highest a dragged limb is held above 47's feet (m).
const GRIP_HEIGHT: f32 = 0.5;

pub struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default())
            .add_systems(Update, (level_collider, test_kill, go_limp, drag_by_limb))
            .add_systems(PostUpdate, pose_bones.after(PhysicsSet::Writeback).before(TransformSystems::Propagate));
        if std::env::var_os("HITMAN_PHYSICS_DEBUG").is_some() {
            app.add_plugins(RapierDebugRenderPlugin::default());
        }
    }
}

/// A killing shot: where it struck, and the push it gives.
#[derive(Component)]
pub struct Shot {
    pub at: Vec3,
    pub impulse: Vec3,
}

/// A body handed over to physics. Bones are looked up by name, so it survives the body's
/// model being swapped (47 taking the outfit).
#[derive(Component)]
pub struct Ragdoll {
    /// Bone name, its body, and the bone's world scale (the model's 0.01).
    parts: Vec<(&'static str, Entity, f32)>,
    /// While dragged: the kinematic anchor at 47's hand, and the joint holding the limb to it.
    drag: Option<(Entity, Entity)>,
}

/// One rigid body of a ragdoll.
#[derive(Component)]
pub struct RagdollPart;

#[derive(Component)]
struct DragAnchor;

enum Shape {
    /// A capsule from this bone to another (its child).
    Along(&'static str),
    /// A capsule between two other bones.
    Between(&'static str, &'static str),
    /// The chest up to the neck, plus the shoulders.
    Torso,
    /// A ball between the head bone and the forehead.
    Head,
}

enum Limit {
    /// Free to turn up to `swing` about the bone's cross axes and `twist` about its own,
    /// from the pose it died in.
    Cone { swing: f32, twist: f32 },
    /// A hinge about local Y, from straight (the bind pose) to `bend` (negative).
    Hinge { bend: f32 },
}

struct Part {
    bone: &'static str,
    parent: Option<usize>,
    shape: Shape,
    radius: f32,
    limit: Limit,
}

const fn part(bone: &'static str, parent: Option<usize>, shape: Shape, radius: f32, limit: Limit) -> Part {
    Part { bone, parent, shape, radius, limit }
}

const PARTS: [Part; 13] = [
    part("PELVIS", None, Shape::Between("LEFT_THIGH", "RIGHT_THIGH"), 0.11, Limit::Cone { swing: 0.0, twist: 0.0 }),
    part("SPINE_1", Some(0), Shape::Torso, 0.13, Limit::Cone { swing: 0.5, twist: 0.35 }),
    part("HEAD", Some(1), Shape::Head, 0.11, Limit::Cone { swing: 0.6, twist: 0.6 }),
    part("LEFT_UPPER_ARM", Some(1), Shape::Along("LEFT_FOREARM"), 0.055, Limit::Cone { swing: 1.4, twist: 0.7 }),
    part("LEFT_FOREARM", Some(3), Shape::Along("LEFT_HAND"), 0.045, Limit::Hinge { bend: -2.5 }),
    part("RIGHT_UPPER_ARM", Some(1), Shape::Along("RIGHT_FOREARM"), 0.055, Limit::Cone { swing: 1.4, twist: 0.7 }),
    part("RIGHT_FOREARM", Some(5), Shape::Along("RIGHT_HAND"), 0.045, Limit::Hinge { bend: -2.5 }),
    part("LEFT_THIGH", Some(0), Shape::Along("LEFT_CALF"), 0.08, Limit::Cone { swing: 1.1, twist: 0.4 }),
    part("LEFT_CALF", Some(7), Shape::Along("LEFT_FOOT"), 0.06, Limit::Hinge { bend: -2.6 }),
    part("LEFT_FOOT", Some(8), Shape::Along("LEFT_TOE"), 0.045, Limit::Cone { swing: 0.5, twist: 0.3 }),
    part("RIGHT_THIGH", Some(0), Shape::Along("RIGHT_CALF"), 0.08, Limit::Cone { swing: 1.1, twist: 0.4 }),
    part("RIGHT_CALF", Some(10), Shape::Along("RIGHT_FOOT"), 0.06, Limit::Hinge { bend: -2.6 }),
    part("RIGHT_FOOT", Some(11), Shape::Along("RIGHT_TOE"), 0.045, Limit::Cone { swing: 0.5, twist: 0.3 }),
];

/// Limbs a body can be dragged by: the forearm or calf, held at the hand or foot.
const GRIPS: [(&str, &str); 4] = [("LEFT_FOREARM", "LEFT_HAND"), ("RIGHT_FOREARM", "RIGHT_HAND"), ("LEFT_CALF", "LEFT_FOOT"), ("RIGHT_CALF", "RIGHT_FOOT")];

/// The level as one fixed triangle mesh, once its collision is built.
fn level_collider(mut commands: Commands, world: Res<WorldCollision>, mut done: Local<bool>) {
    if *done || world.len() == 0 {
        return;
    }
    *done = true;
    let started = std::time::Instant::now();
    let tris = world.triangles();
    let vertices: Vec<Vec3> = tris.iter().flatten().copied().collect();
    let indices: Vec<[u32; 3]> = (0..tris.len() as u32).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect();
    let flags = TriMeshFlags::MERGE_DUPLICATE_VERTICES | TriMeshFlags::DELETE_DEGENERATE_TRIANGLES | TriMeshFlags::DELETE_DUPLICATE_TRIANGLES;
    match Collider::trimesh_with_flags(vertices, indices, flags) {
        Ok(collider) => {
            commands.spawn((RigidBody::Fixed, collider, Friction::coefficient(0.9), Transform::IDENTITY, Name::new("Level collision")));
            info!("ragdoll: level collider from {} triangles in {:.2}s", tris.len(), started.elapsed().as_secs_f32());
        }
        Err(e) => warn!("ragdoll: level collider failed: {e}"),
    }
}

/// `HITMAN_KILL="Name@secs"`: shoots that character dead at that time (testing).
fn test_kill(
    mut commands: Commands,
    time: Res<Time>,
    mut done: Local<bool>,
    player: Query<&GlobalTransform, With<Player>>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor)>,
) {
    if *done {
        return;
    }
    let Some((name, at)) = std::env::var("HITMAN_KILL").ok().and_then(|v| v.split_once('@').map(|(n, t)| (n.to_owned(), t.parse::<f32>().unwrap_or(1.0)))) else {
        *done = true;
        return;
    };
    if time.elapsed_secs() < at {
        return;
    }
    *done = true;
    let from = player.single().map_or(Vec3::ZERO, GlobalTransform::translation);
    for (e, t, mut actor) in &mut actors {
        if actor.name == name {
            actor.alive = false;
            let hit = t.translation() + Vec3::Y * 1.3;
            commands.entity(e).remove::<crate::character::Gait>().insert(Shot { at: hit, impulse: (hit - from).with_y(0.0).normalize_or_zero() * SHOT_PUSH });
            info!("test kill: {name} at {:.1?}", t.translation());
        }
    }
}

/// Dead characters go limp: at once if shot, else when their death clip ends.
fn go_limp(
    mut commands: Commands,
    mut dead: Query<(Entity, &Actor, &Rig, &mut Animator, Option<&Shot>), Without<Ragdoll>>,
    globals: Query<&GlobalTransform>,
) {
    for (e, actor, rig, mut anim, shot) in &mut dead {
        if actor.alive || (shot.is_none() && anim.busy()) {
            continue;
        }
        let Some(ragdoll) = build(&mut commands, rig, &globals, shot, &actor.name) else { continue };
        anim.go_limp();
        commands.entity(e).insert(ragdoll).remove::<Shot>();
    }
}

fn build(commands: &mut Commands, rig: &Rig, globals: &Query<&GlobalTransform>, shot: Option<&Shot>, name: &str) -> Option<Ragdoll> {
    let world = |bone: &str| rig.joint(bone).and_then(|j| globals.get(j).ok()).map(GlobalTransform::compute_transform);
    let mut parts: Vec<(&'static str, Entity, f32)> = Vec::new();
    let mut poses: Vec<Transform> = Vec::new();
    for p in &PARTS {
        let bone = world(p.bone)?;
        let (pos, rot) = (bone.translation, bone.rotation);
        let local = |w: Vec3| rot.inverse() * (w - pos);
        let capsule = |a: Vec3, b: Vec3, r: f32| {
            // Pull the ends in so the caps end at the joints instead of past them.
            let d = (b - a).normalize_or_zero() * r.min((b - a).length() * 0.45);
            Collider::capsule(a + d, b - d, r)
        };
        let collider = match p.shape {
            Shape::Along(child) => capsule(Vec3::ZERO, local(world(child)?.translation), p.radius),
            Shape::Between(a, b) => capsule(local(world(a)?.translation), local(world(b)?.translation), p.radius),
            Shape::Torso => Collider::compound(vec![
                (Vec3::ZERO, Quat::IDENTITY, capsule(Vec3::ZERO, local(world("NECK")?.translation), p.radius)),
                (Vec3::ZERO, Quat::IDENTITY, capsule(local(world("LEFT_UPPER_ARM")?.translation), local(world("RIGHT_UPPER_ARM")?.translation), 0.07)),
            ]),
            Shape::Head => {
                let centre = (local(world("FOREHEAD")?.translation) + Vec3::ZERO) * 0.5;
                Collider::compound(vec![(centre, Quat::IDENTITY, Collider::ball(p.radius))])
            }
        };
        let body = commands
            .spawn((
                RagdollPart,
                RigidBody::Dynamic,
                Transform::from_translation(pos).with_rotation(rot),
                collider,
                ColliderMassProperties::Density(DENSITY),
                Damping { linear_damping: 0.05, angular_damping: 1.5 },
                Friction::coefficient(0.8),
                Velocity::zero(),
                Ccd::enabled(),
                Name::new(format!("{name} {}", p.bone)),
            ))
            .id();
        if let Some(pi) = p.parent {
            let parent = poses[pi];
            let (axes, basis) = match p.limit {
                // Zero at the pose it died in.
                Limit::Cone { .. } => (JointAxesMask::LIN_AXES, parent.rotation.inverse() * rot),
                // Zero at the bind pose (straight), which is the child's bind rotation in its
                // parent bone's frame.
                Limit::Hinge { .. } => (JointAxesMask::LIN_AXES | JointAxesMask::ANG_X | JointAxesMask::ANG_Z, rig.bind_local(rig.index(p.bone)?).rotation),
            };
            let mut joint = GenericJointBuilder::new(axes)
                .local_anchor1(parent.rotation.inverse() * (pos - parent.translation))
                .local_anchor2(Vec3::ZERO)
                .local_basis1(basis)
                .local_basis2(Quat::IDENTITY)
                .contacts_enabled(false);
            joint = match p.limit {
                Limit::Cone { swing, twist } => joint
                    .limits(JointAxis::AngX, [-swing, swing])
                    .limits(JointAxis::AngY, [-swing, swing])
                    .limits(JointAxis::AngZ, [-twist, twist])
                    .motor_velocity(JointAxis::AngX, 0.0, JOINT_FRICTION)
                    .motor_velocity(JointAxis::AngY, 0.0, JOINT_FRICTION)
                    .motor_velocity(JointAxis::AngZ, 0.0, JOINT_FRICTION),
                Limit::Hinge { bend } => joint.limits(JointAxis::AngY, [bend, 0.05]).motor_velocity(JointAxis::AngY, 0.0, JOINT_FRICTION),
            };
            commands.entity(body).insert(ImpulseJoint::new(parts[pi].1, joint));
        }
        parts.push((p.bone, body, bone.scale.x));
        poses.push(Transform::from_translation(pos).with_rotation(rot));
    }
    if let Some(shot) = shot {
        // The part nearest the hit takes the push.
        if let Some((i, pose)) = poses.iter().enumerate().min_by(|a, b| a.1.translation.distance(shot.at).total_cmp(&b.1.translation.distance(shot.at))) {
            commands.entity(parts[i].1).insert(ExternalImpulse::at_point(shot.impulse, shot.at, pose.translation));
        }
    }
    Some(Ragdoll { parts, drag: None })
}

/// Writes the bodies' poses into the skeleton, and keeps the character's root (what gameplay
/// sees: bodies found, dragged, hidden) under the pelvis.
fn pose_bones(
    mut ragdolls: Query<(&Ragdoll, &Rig, &mut Transform), (Without<RagdollPart>, Without<DragAnchor>)>,
    bodies: Query<&Transform, (With<RagdollPart>, Without<Ragdoll>)>,
    mut joints: Query<&mut Transform, (Without<Ragdoll>, Without<RagdollPart>, Without<DragAnchor>)>,
) {
    for (ragdoll, rig, mut root) in &mut ragdolls {
        let mut simulated: Vec<Option<(Transform, f32)>> = vec![None; rig.joints().len()];
        for &(bone, body, scale) in &ragdoll.parts {
            if let (Some(i), Ok(t)) = (rig.index(bone), bodies.get(body)) {
                simulated[i] = Some((*t, scale));
            }
        }
        let Some(pelvis) = rig.index("PELVIS").and_then(|i| simulated[i]) else { continue };
        root.translation = pelvis.0.translation - Vec3::Y * 0.1;
        let root_m = root.to_matrix();
        let mut world: Vec<Mat4> = Vec::with_capacity(rig.joints().len());
        for (i, &j) in rig.joints().iter().enumerate() {
            let parent = rig.parent(i).map_or(root_m, |p| world[p]);
            let Ok(mut t) = joints.get_mut(j) else {
                world.push(parent);
                continue;
            };
            match simulated[i] {
                Some((body, scale)) => {
                    let w = Mat4::from_scale_rotation_translation(Vec3::splat(scale), body.rotation, body.translation);
                    *t = Transform::from_matrix(parent.inverse() * w);
                    world.push(w);
                }
                None => world.push(parent * t.to_matrix()),
            }
        }
    }
}

/// Dragging (G) a ragdoll: the wrist or ankle nearest 47's hand is jointed to a kinematic
/// anchor that follows the hand.
#[allow(clippy::too_many_arguments)]
fn drag_by_limb(
    mut commands: Commands,
    time: Res<Time>,
    mission: Option<Res<Mission>>,
    player: Query<(&GlobalTransform, &Player)>,
    rigs: Query<&Rig>,
    globals: Query<&GlobalTransform>,
    mut ragdolls: Query<(Entity, &mut Ragdoll)>,
    mut anchors: Query<&mut Transform, With<DragAnchor>>,
) {
    let Some(mission) = mission else { return };
    let Ok((pt, p)) = player.single() else { return };
    // 47's right hand, or just behind him at hand height.
    let hand = p
        .model
        .and_then(|m| rigs.get(m).ok())
        .and_then(|r| r.joint("RIGHT_HAND"))
        .and_then(|j| globals.get(j).ok())
        .map(GlobalTransform::translation)
        .unwrap_or_else(|| pt.translation() + pt.rotation() * Vec3::new(0.0, 0.5, 0.6));
    // Held low, as if bent over pulling: a wrist or ankle hoisted to hand height lifts the
    // whole body off the floor.
    let hand = hand.with_y(hand.y.min(pt.translation().y + GRIP_HEIGHT));
    for (e, mut ragdoll) in &mut ragdolls {
        let dragged = mission.dragging_body == Some(e);
        match (dragged, ragdoll.drag) {
            (true, None) => {
                let Ok(rig) = rigs.get(e) else { continue };
                let grip = GRIPS
                    .iter()
                    .filter_map(|&(limb, end)| {
                        let body = ragdoll.parts.iter().find(|p| p.0 == limb)?.1;
                        let at = globals.get(rig.joint(end)?).ok()?.translation();
                        Some((body, at))
                    })
                    .min_by(|a, b| a.1.distance(hand).total_cmp(&b.1.distance(hand)));
                let Some((body, at)) = grip else { continue };
                let Ok(body_t) = globals.get(body).map(GlobalTransform::compute_transform) else { continue };
                let anchor = commands.spawn((DragAnchor, RigidBody::KinematicPositionBased, Transform::from_translation(at), Name::new("Drag anchor"))).id();
                let joint = SphericalJointBuilder::new().local_anchor1(Vec3::ZERO).local_anchor2(body_t.rotation.inverse() * (at - body_t.translation));
                let holder = commands.spawn((ImpulseJoint::new(anchor, joint), Transform::default())).id();
                commands.entity(body).add_child(holder);
                for &(_, part, _) in &ragdoll.parts {
                    commands.entity(part).insert(Sleeping::disabled());
                }
                ragdoll.drag = Some((anchor, holder));
            }
            (true, Some((anchor, _))) => {
                if let Ok(mut t) = anchors.get_mut(anchor) {
                    let k = 1.0 - (-GRIP_PULL * time.delta_secs()).exp();
                    t.translation = t.translation.lerp(hand, k);
                }
            }
            (false, Some((anchor, holder))) => {
                commands.entity(holder).despawn();
                commands.entity(anchor).despawn();
                for &(_, part, _) in &ragdoll.parts {
                    commands.entity(part).insert(Sleeping::default());
                }
                ragdoll.drag = None;
            }
            (false, None) => {}
        }
    }
}
