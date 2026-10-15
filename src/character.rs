//! Skinned characters: 47, guards, Bjarkhov and the rest of the cast.
//!
//! The rig lives in the same "P-space" as static meshes (PRM X and Z swapped, see
//! `view.rs`): vertex and bone positions get `swap_xz`, and bone rotations are conjugated by
//! that mirror, which maps quaternion (x, y, z, w) to (−z, −y, −x, w). Joints are entities
//! under the character root, whose transform carries the cm→m scale and placement.
//!
//! Animations come straight from the level's `.ANM` (see `glacier::anm`), converted the same
//! way. The GROUND track carries root motion; gameplay moves characters itself, so looping
//! clips drop it and play in place, while one-off clips (deaths, kills) keep it.

use crate::view::{MaterialKey, build_material, swap_xz};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{
        Indices, PrimitiveTopology, VertexAttributeValues,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
    prelude::*,
};
use hitman_clone::glacier::{
    anm::{self, AnmFile, Animation},
    skin::SkinnedModel,
    tex::TexArchive,
};
use std::collections::HashMap;

/// Cross-fade time between clips.
const BLEND_SECONDS: f32 = 0.25;
/// Below this speed a moving character plays its idle.
const IDLE_BELOW: f32 = 0.15;
/// Between walk and run clips by speed (m/s).
const RUN_ABOVE: f32 = 2.6;

pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, choose_locomotion).add_systems(PostUpdate, animate.before(TransformSystems::Propagate));
    }
}

/// Every animation set in the level.
#[derive(Resource)]
pub struct AnimLibrary(pub AnmFile);

impl AnimLibrary {
    fn clip(&self, set: usize, name: &str) -> Option<&Animation> {
        self.0.sets.get(set)?.animations.get(name)
    }

    /// Index of the set whose file name is `file`, e.g. `"Hero.anm"`.
    pub fn set_index(&self, file: &str) -> Option<usize> {
        self.0.sets.iter().position(|s| s.path.rsplit('\\').next().is_some_and(|f| f.eq_ignore_ascii_case(file)))
    }

    /// First of `names` that exists in `set`.
    pub fn first_of(&self, set: usize, names: &[&str]) -> Option<String> {
        names.iter().find(|n| self.clip(set, n).is_some()).map(|n| (*n).to_owned())
    }
}

/// Skeleton of a spawned character, for animation.
#[derive(Component)]
pub struct Rig {
    joints: Vec<Entity>,
    bind_local: Vec<Transform>,
    by_name: HashMap<String, usize>,
}

struct Playing {
    clip: String,
    time: f32,
    rate: f32,
    looping: bool,
}

/// Which clip a character plays, with a short cross-fade from the previous one.
#[derive(Component)]
pub struct Animator {
    set: usize,
    current: Playing,
    previous: Option<(Playing, f32)>,
    /// Seconds left of a play-once clip; locomotion waits for it.
    busy: f32,
}

impl Animator {
    pub fn new(set: usize, clip: String) -> Self {
        Self { set, current: Playing { clip, time: 0.0, rate: 1.0, looping: true }, previous: None, busy: 0.0 }
    }

    pub fn set(&self) -> usize {
        self.set
    }

    /// Loops `clip`, cross-fading from whatever was playing.
    pub fn play(&mut self, clip: &str, rate: f32) {
        self.start(clip, rate, true);
    }

    /// Plays `clip` once and holds its last frame (deaths, kills).
    pub fn play_once(&mut self, clip: &str) {
        self.start(clip, 1.0, false);
    }

    /// `play_once` at `rate` times normal speed.
    pub fn play_once_at(&mut self, clip: &str, rate: f32) {
        self.start(clip, rate, false);
    }

    fn start(&mut self, clip: &str, rate: f32, looping: bool) {
        if !looping {
            self.busy = 1.0; // refined to the clip length on the next animate pass
        }
        if self.current.clip == clip {
            self.current.rate = rate;
            return;
        }
        let old = std::mem::replace(&mut self.current, Playing { clip: clip.to_owned(), time: 0.0, rate, looping });
        self.previous = Some((old, BLEND_SECONDS));
    }
}

/// Locomotion input: current ground speed in m/s, and the clips to choose from.
#[derive(Component)]
pub struct Gait {
    pub speed: f32,
    pub crouching: bool,
    pub dragging: bool,
    /// Clip played when standing; gameplay swaps it (alert stance, aiming).
    pub idle: String,
    pub crouch_idle: String,
    pub walk: Option<(String, f32)>,
    pub run: Option<(String, f32)>,
    pub crouch_walk: Option<(String, f32)>,
    pub drag_walk: Option<(String, f32)>,
}

impl Gait {
    /// Looks up idle/walk/run clips in `set`; walk and run carry their root speed (m/s) so
    /// playback can match the character's actual speed without foot sliding.
    pub fn new(lib: &AnimLibrary, set: usize, idle: String) -> Self {
        let with_speed = |names: &[&str]| lib.first_of(set, names).and_then(|n| lib.clip(set, &n).map(|c| (n.clone(), root_speed(c))));
        Self {
            speed: 0.0,
            crouching: false,
            dragging: false,
            crouch_idle: lib.first_of(set, &["Sneak_Ambient", "Crouch_Ambient"]).unwrap_or_else(|| idle.clone()),
            idle,
            walk: with_speed(&["Walk_Forward", "Walk"]),
            run: with_speed(&["Run"]),
            crouch_walk: with_speed(&["Sneak_Forward", "Crouch_Forward"]),
            drag_walk: with_speed(&["Drag_Backward", "Drag_Forward"]),
        }
    }
}

/// Average root (GROUND) speed of a clip in m/s.
fn root_speed(clip: &Animation) -> f32 {
    let Some(root) = clip.tracks.iter().find(|t| t.bone == "GROUND") else { return 1.0 };
    let (Some(a), Some(b)) = (root.positions.first(), root.positions.last()) else { return 1.0 };
    let d = Vec3::from(b.1) - Vec3::from(a.1);
    (d.length() * 0.01 / clip.duration().max(1e-3)).max(0.1)
}

pub struct CharacterAssets<'a> {
    pub meshes: &'a mut Assets<Mesh>,
    pub materials: &'a mut Assets<StandardMaterial>,
    pub images: &'a mut Assets<Image>,
    pub bindposes: &'a mut Assets<SkinnedMeshInverseBindposes>,
    pub textures: &'a TexArchive,
}

/// Builds `model` under `root`: joint entities, inverse bind poses and skinned meshes.
pub fn attach(commands: &mut Commands, root: Entity, model: &SkinnedModel, assets: &mut CharacterAssets) {
    let bind_local: Vec<Transform> = model
        .bones
        .iter()
        .map(|b| {
            let [x, y, z, w] = b.rotation;
            Transform { translation: Vec3::from(swap_xz(b.translation)), rotation: Quat::from_xyzw(-z, -y, -x, w).normalize(), scale: Vec3::ONE }
        })
        .collect();

    // Model-space bind pose, composed down the hierarchy (parents always precede children).
    let mut bind_global: Vec<Mat4> = Vec::with_capacity(bind_local.len());
    for (b, local) in model.bones.iter().zip(&bind_local) {
        let m = local.to_matrix();
        bind_global.push(b.parent.map_or(m, |p| bind_global[p] * m));
    }

    let mut joints: Vec<Entity> = Vec::with_capacity(model.bones.len());
    for (b, local) in model.bones.iter().zip(&bind_local) {
        let parent = b.parent.map_or(root, |p| joints[p]);
        let j = commands.spawn((*local, Name::new(b.name.clone()))).id();
        commands.entity(parent).add_child(j);
        joints.push(j);
    }

    let inverse: Vec<Mat4> = bind_global.iter().map(Mat4::inverse).collect();
    let skin = SkinnedMesh { inverse_bindposes: assets.bindposes.add(SkinnedMeshInverseBindposes::from(inverse)), joints: joints.clone() };

    let mut material_cache: HashMap<MaterialKey, Handle<StandardMaterial>> = HashMap::new();
    for sm in &model.submeshes {
        let key = MaterialKey { texture: sm.texture, blended: sm.blended };
        let material = material_cache.entry(key).or_insert_with(|| assets.materials.add(build_material(assets.textures, key, assets.images))).clone();
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, sm.vertices.iter().map(|v| swap_xz(v.pos)).collect::<Vec<_>>())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, sm.vertices.iter().map(|v| swap_xz(v.normal)).collect::<Vec<_>>())
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, sm.vertices.iter().map(|v| v.uv).collect::<Vec<_>>())
            .with_inserted_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(sm.vertices.iter().map(|v| v.bones).collect()))
            .with_inserted_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, sm.vertices.iter().map(|v| v.weights).collect::<Vec<_>>())
            .with_inserted_indices(Indices::U16(sm.triangles.iter().flatten().copied().collect()));
        let part = commands.spawn((Mesh3d(assets.meshes.add(mesh)), MeshMaterial3d(material), skin.clone())).id();
        commands.entity(root).add_child(part);
    }

    let by_name = model.bones.iter().enumerate().map(|(i, b)| (b.name.clone(), i)).collect();
    commands.entity(root).insert(Rig { joints, bind_local, by_name });
}

fn choose_locomotion(mut q: Query<(&Gait, &mut Animator)>) {
    for (gait, mut anim) in &mut q {
        if anim.busy > 0.0 {
            continue;
        }
        let pick = if gait.speed < IDLE_BELOW {
            None
        } else if gait.dragging && gait.drag_walk.is_some() {
            gait.drag_walk.as_ref()
        } else if gait.crouching && gait.crouch_walk.is_some() {
            gait.crouch_walk.as_ref()
        } else if gait.speed > RUN_ABOVE && gait.run.is_some() {
            gait.run.as_ref()
        } else {
            gait.walk.as_ref().or(gait.run.as_ref())
        };
        let idle_clip = if gait.dragging {
            "Drag_Ambient"
        } else if gait.crouching {
            &gait.crouch_idle
        } else {
            &gait.idle
        };
        match pick {
            Some((clip, root)) => anim.play(clip, gait.speed / root),
            None => anim.play(idle_clip, 1.0),
        }
    }
}

/// Local pose for one clip at `time`: per joint, an optional rotation and translation.
fn sample_pose(rig: &Rig, clip: &Animation, time: f32, looping: bool) -> Vec<(Option<Quat>, Option<Vec3>)> {
    let mut pose = vec![(None, None); rig.joints.len()];
    let frames = clip.frames.max(1) as f32;
    let frame = if looping { (time * anm::FPS) % frames } else { (time * anm::FPS).min(frames) };
    for track in &clip.tracks {
        // Looping clips (locomotion, idles) play in place: gameplay moves the character. One-off
        // clips keep their root motion; a death needs it to bring the body to the floor.
        if track.bone == "GROUND" && looping {
            continue;
        }
        let Some(&j) = rig.by_name.get(&track.bone) else { continue };
        let [x, y, z, w] = anm::sample_rot(&track.rotations, frame);
        let rot = Quat::from_xyzw(-z, -y, -x, w).normalize();
        let pos = (!track.positions.is_empty()).then(|| Vec3::from(swap_xz(anm::sample_pos(&track.positions, frame))));
        pose[j] = (Some(rot), pos);
    }
    pose
}

fn animate(
    time: Res<Time>,
    lib: Option<Res<AnimLibrary>>,
    mut missing: Local<std::collections::HashSet<String>>,
    mut rigs: Query<(&Rig, &mut Animator)>,
    mut joints: Query<&mut Transform>,
) {
    let Some(lib) = lib else { return };
    if std::env::var_os("HITMAN_NO_ANIM").is_some() {
        return; // debug: show bind poses
    }
    let dt = time.delta_secs();
    for (rig, mut anim) in &mut rigs {
        let set = anim.set;
        anim.current.time += dt * anim.current.rate;
        let Some(clip) = lib.clip(set, &anim.current.clip) else {
            // A bad clip name must never lock a character: report it once and release.
            if missing.insert(anim.current.clip.clone()) {
                warn!("animation clip {:?} not found in set {set}", anim.current.clip);
            }
            anim.busy = 0.0;
            continue;
        };
        anim.busy = if anim.current.looping { 0.0 } else { (clip.duration() - anim.current.time).max(0.0) / anim.current.rate.max(0.01) };
        let mut pose = sample_pose(rig, clip, anim.current.time, anim.current.looping);

        let mut fade_done = false;
        if let Some((prev, remaining)) = anim.previous.as_mut() {
            prev.time += dt * prev.rate;
            *remaining -= dt;
            let w = (*remaining / BLEND_SECONDS).clamp(0.0, 1.0);
            if let Some(prev_clip) = lib.clip(set, &prev.clip) {
                let old = sample_pose(rig, prev_clip, prev.time, prev.looping);
                for (i, ((r, p), (or, op))) in pose.iter_mut().zip(old).enumerate() {
                    let bind = rig.bind_local[i];
                    let (a, b) = (or.unwrap_or(bind.rotation), r.unwrap_or(bind.rotation));
                    *r = Some(b.slerp(a, w));
                    let (a, b) = (op.unwrap_or(bind.translation), p.unwrap_or(bind.translation));
                    *p = Some(b.lerp(a, w));
                }
            }
            fade_done = *remaining <= 0.0;
        }
        if fade_done {
            anim.previous = None;
        }

        for (i, &j) in rig.joints.iter().enumerate() {
            if let Ok(mut t) = joints.get_mut(j) {
                let bind = rig.bind_local[i];
                t.rotation = pose[i].0.unwrap_or(bind.rotation);
                t.translation = pose[i].1.unwrap_or(bind.translation);
            }
        }
    }
}
