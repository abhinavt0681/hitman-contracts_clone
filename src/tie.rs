//! 47's tie. The original keeps it out of the suit's mesh: it's its own object (`Tie`, engine
//! class 0x2000FF, in the `Hitman2_Tie` group with three `CollisionPlane`s that keep it off his
//! chest), hung from the knot and swinging as he moves. Here it hangs from his chest bone at
//! the template's own offset and swings on a damped spring driven by his acceleration.
//! It comes off with the suit.

use crate::{character::Rig, game::{Mission, Outfit}, player::Player};
use bevy::prelude::*;

/// Stiffness and damping of the swing, and how far acceleration tips it (rad per m/s²).
const STIFFNESS: f32 = 60.0;
const DAMPING: f32 = 7.0;
const TIP: f32 = 0.05;
const MAX_SWING: f32 = 0.6;

pub struct TiePlugin;

impl Plugin for TiePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (attach, swing));
    }
}

/// The tie's mesh and material, where it sits in 47's model space, and its knot (the top of
/// the mesh, in its own space).
#[derive(Resource)]
pub struct TieModel {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    pub in_model: Mat4,
    pub knot: Vec3,
}

/// The pivot at the knot, with the swing's state.
#[derive(Component, Default)]
struct Tie {
    angle: Vec2,
    rate: Vec2,
    last_velocity: Vec3,
    last_position: Option<Vec3>,
}

/// Puts the tie on whenever 47's skeleton is (re)built in his suit.
fn attach(mut commands: Commands, tie: Option<Res<TieModel>>, mission: Option<Res<Mission>>, player: Query<&Player>, rigs: Query<&Rig, Changed<Rig>>) {
    let (Some(tie), Ok(p)) = (tie, player.single()) else { return };
    let Some(model) = p.model else { return };
    let Ok(rig) = rigs.get(model) else { return };
    if mission.is_some_and(|m| m.outfit != Outfit::Suit) {
        return;
    }
    let (Some(chest), Some(i)) = (rig.joint("SPINE_1"), rig.index("SPINE_1")) else { return };
    // Model space → chest-bone space, then the knot as the pivot.
    let in_chest = rig.bind_model(i).inverse() * tie.in_model;
    let knot = in_chest.transform_point3(tie.knot);
    let pivot = commands.spawn((Tie::default(), Transform::from_translation(knot), Visibility::Inherited, Name::new("Tie"))).id();
    let mesh = commands
        .spawn((Mesh3d(tie.mesh.clone()), MeshMaterial3d(tie.material.clone()), Transform::from_matrix(Mat4::from_translation(-knot) * in_chest)))
        .id();
    commands.entity(pivot).add_child(mesh);
    commands.entity(chest).insert_if_new(Visibility::Inherited).add_child(pivot);
}

/// A damped spring tips the tie against 47's acceleration (it trails when he sets off, swings
/// forward when he stops or turns).
fn swing(time: Res<Time>, player: Query<&GlobalTransform, With<Player>>, mut ties: Query<(&mut Tie, &mut Transform, &ChildOf)>, joints: Query<&GlobalTransform>) {
    let dt = time.delta_secs();
    let Ok(pt) = player.single() else { return };
    if dt <= 0.0 {
        return;
    }
    for (mut tie, mut t, parent) in &mut ties {
        let pos = pt.translation();
        let velocity = tie.last_position.map_or(Vec3::ZERO, |last| (pos - last) / dt);
        let accel = ((velocity - tie.last_velocity) / dt).clamp_length_max(30.0);
        tie.last_position = Some(pos);
        tie.last_velocity = velocity;
        // Acceleration in 47's own frame: forward (−Z) and right (+X).
        let (fwd, right) = (pt.forward().as_vec3().with_y(0.0).normalize_or_zero(), pt.right().as_vec3().with_y(0.0).normalize_or_zero());
        let target = Vec2::new(accel.dot(fwd), accel.dot(right)) * TIP;
        let force = (target - tie.angle) * STIFFNESS - tie.rate * DAMPING;
        tie.rate += force * dt;
        let angle = (tie.angle + tie.rate * dt).clamp(Vec2::splat(-MAX_SWING), Vec2::splat(MAX_SWING));
        tie.angle = angle;
        // The swing in the world, brought into the chest bone's frame.
        let world_swing = Quat::from_axis_angle(right, angle.x) * Quat::from_axis_angle(fwd, -angle.y);
        let chest = joints.get(parent.parent()).map_or(Quat::IDENTITY, |g| g.compute_transform().rotation);
        t.rotation = chest.inverse() * world_swing * chest;
    }
}
