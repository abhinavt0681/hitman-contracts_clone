//! Footsteps, timed by the animation itself: a step sounds when a foot bone comes down to
//! the lowest height that foot reaches (tracked per foot, with hysteresis so each plant counts
//! once), so it lands with the foot at any speed and frame rate, for 47 and everyone else. The sound
//! comes from the surface under the character (`collision::Surface`, from texture names):
//! snow, gravel, steel decks, wooden floors, concrete…
//!
//! The original raises `Msg_Sound_FootStep` from its animation system; the `.ANM` data
//! carries no step events, so contact is detected from the pose.

use crate::{
    audio::SoundBank,
    character::{Gait, Rig},
    collision::WorldCollision,
    player::Player,
};
use bevy::{
    audio::{PlaybackSettings, SpatialScale, Volume},
    prelude::*,
    transform::TransformSystems,
};

/// A foot within this of its lowest height is planted; it must rise `LIFT` above that again
/// before the next step counts.
const PLANT: f32 = 0.025;
const LIFT: f32 = 0.06;
/// Others' steps are only played within this distance of the camera.
const HEARD_WITHIN: f32 = 25.0;

pub struct FootstepsPlugin;

impl Plugin for FootstepsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, track_feet).add_systems(PostUpdate, step.after(TransformSystems::Propagate));
    }
}

#[derive(Component)]
struct Feet {
    joints: [Entity; 2],
    /// Lowest height each foot has been at lately (relaxes upward slowly).
    low: [f32; 2],
    /// Whether each foot is in the air, so its next plant is a step.
    lifted: [bool; 2],
    variant: usize,
}

/// (Re)attaches foot tracking whenever a rig is built (characters, and 47 after a disguise).
fn track_feet(mut commands: Commands, rigs: Query<(Entity, &Rig), Changed<Rig>>) {
    for (e, rig) in &rigs {
        if let (Some(l), Some(r)) = (rig.joint("LEFT_FOOT"), rig.joint("RIGHT_FOOT")) {
            commands.entity(e).insert(Feet { joints: [l, r], low: [1.0; 2], lifted: [false; 2], variant: e.index_u32() as usize });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn step(
    mut commands: Commands,
    time: Res<Time>,
    bank: Option<Res<SoundBank>>,
    world: Res<WorldCollision>,
    player: Query<&Player>,
    camera: Query<&GlobalTransform, With<crate::player::OrbitCamera>>,
    joints: Query<&GlobalTransform>,
    mut walkers: Query<(Entity, &GlobalTransform, &Gait, &mut Feet)>,
) {
    let Some(bank) = bank else { return };
    let me = player.single().ok().and_then(|p| p.model);
    let ear = camera.iter().next().map(|c| c.translation());
    let dt = time.delta_secs();
    for (e, root, gait, mut feet) in &mut walkers {
        let base = root.translation();
        for k in 0..2 {
            let Ok(foot) = joints.get(feet.joints[k]) else { continue };
            let h = foot.translation().y - base.y;
            // The lowest point drifts with the clip (walk, run, sneak): follow it down at once
            // and up slowly.
            feet.low[k] = if h < feet.low[k] { h } else { feet.low[k] + 0.05 * dt };
            if h > feet.low[k] + LIFT {
                feet.lifted[k] = true;
            }
            let landed = feet.lifted[k] && h < feet.low[k] + PLANT;
            if landed {
                feet.lifted[k] = false;
            }
            if !landed || gait.speed < 0.2 {
                continue;
            }
            let surface = world.raycast(base + Vec3::Y * 0.4, Vec3::NEG_Y, 1.2).map(|h| h.surface).unwrap_or_default();
            let Some(sounds) = bank.steps.get(&surface).filter(|v| !v.is_empty()) else { continue };
            feet.variant = (feet.variant * 7 + 3) % 997;
            let sound = sounds[feet.variant % sounds.len()].clone();
            let running = gait.speed > 2.6;
            debug!("step {:?} foot {k} on {surface:?} at speed {:.1}", e, gait.speed);
            if Some(e) == me {
                let volume = if gait.crouching {
                    0.08
                } else if running {
                    0.5
                } else {
                    0.28
                };
                commands.spawn((AudioPlayer::new(sound), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume))));
            } else if ear.is_some_and(|ear| ear.distance(base) < HEARD_WITHIN) {
                let volume = if running { 0.7 } else { 0.4 };
                commands.spawn((
                    AudioPlayer::new(sound),
                    PlaybackSettings::DESPAWN.with_spatial(true).with_spatial_scale(SpatialScale::new(1.0 / 3.0)).with_volume(Volume::Linear(volume)),
                    Transform::from_translation(base),
                ));
            }
        }
    }
}
