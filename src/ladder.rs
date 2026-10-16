//! Ladders: 47 climbs up and down the level's ladders (E at either end).
//!
//! Each ladder comes from one of the level's guide entities (`LadderSub_T1Guide`,
//! `LadderTunnel_T1Guide`, `MountainLadder_T1Guide_01`…): an invisible box from the foot of the
//! ladder to the top. The guide doesn't say which side is climbed, so that comes from the
//! level's collision: the side with ground at the bottom faces the side with a landing at the
//! top.
//!
//! The climb plays 47's original `Climb_Ladder` clip (forwards up, backwards down). Its root
//! motion rises 2.01 m per 1.9 s cycle, so he climbs at that speed and the hands and feet stay
//! on the rungs.

use crate::{
    character::{Animator, Gait},
    collision::WorldCollision,
    game::Mission,
    player::{OrbitCamera, Player},
    view::LadderGuide,
};
use bevy::prelude::*;

/// `Climb_Ladder` is one clip: frames 0–23 are the climbing stroke (frame 23 matches frame 0
/// to 0.002 rad on average, the root 50 cm higher), frames 23–57 the climb over the top
/// (another 150.65 cm). Strokes loop on the way; the climb-over plays once at the top
/// (backwards when getting on to climb down). Found by `examples/climb_cycle.rs`.
const CLIP: &str = "Climb_Ladder";
const STROKE: (f32, f32) = (0.0, 23.0);
const OVER: (f32, f32) = (23.0, 57.0);
const CLIMB_SPEED: f32 = 0.50 / (23.0 / 30.0);
const OVER_RISE: f32 = 1.5065;
const OVER_SECONDS: f32 = 34.0 / 30.0;
/// How far from the ladder 47 hangs, and where he stands at either end (m).
const HANG: f32 = 0.32;
const STAND: f32 = 0.6;
/// Within this of a ladder's end (m), E climbs it.
const REACH: f32 = 0.9;
/// How far a guide's box sits above the floors it joins: measured on the submarine ladder
/// (guide −2.6…0.9 m, ice −3.7, deck −0.2) and the tunnel shaft (−8.0…1.25, floors −9.1, 0.15).
const GUIDE_LIFT: f32 = 1.1;
/// Seconds to step on or off the ladder.
const STEP_ON: f32 = 0.35;
const STEP_OFF: f32 = 0.5;

pub struct LadderPlugin;

impl Plugin for LadderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (offer, climb)
                .chain()
                .after(crate::game::GameplaySet)
                .run_if(resource_exists::<Ladders>)
                .run_if(resource_exists::<Mission>)
                .run_if(in_state(crate::flow::Phase::Playing)),
        );
    }
}

pub struct Ladder {
    name: String,
    /// Horizontal centre of the ladder.
    centre: Vec3,
    /// Horizontal drift of the rungs per metre of height (leaning ladders).
    lean: Vec3,
    /// Horizontal direction 47 faces while on it (towards the ladder, and the top landing).
    face: Vec3,
    /// Floor heights at the foot and at the top landing.
    bottom: f32,
    top: f32,
}

impl Ladder {
    /// Where 47 hangs on the ladder at height `y`.
    fn hang(&self, y: f32) -> Vec3 {
        (self.centre + self.lean * (y - self.bottom) - self.face * HANG).with_y(y)
    }
    fn foot(&self) -> Vec3 {
        (self.centre - self.face * STAND).with_y(self.bottom)
    }
    fn landing(&self) -> Vec3 {
        (self.centre + self.lean * (self.top - self.bottom) + self.face * STAND).with_y(self.top)
    }
}

#[derive(Resource)]
pub struct Ladders(pub Vec<Ladder>);

/// Builds ladders from the level's guides; guides whose sides can't be found are skipped.
pub fn build(guides: &[LadderGuide], world: &WorldCollision) -> Ladders {
    let mut ladders = Vec::new();
    for g in guides.iter().filter(|g| g.name.contains("Guide") && g.max.y - g.min.y > 1.0) {
        // The rungs' line from the ladder mesh when there is one; else the guide's centre.
        let (centre, lean, sides) = match &g.rail {
            Some(r) if r.high.y - r.low.y > 0.3 => {
                let lean = (r.high - r.low).with_y(0.0) / (r.high.y - r.low.y);
                let normal = r.across.cross(Vec3::Y).normalize();
                // Back to the guide's foot height, so `centre` is the rungs' line at the bottom.
                let centre = (r.low - lean * (r.low.y - (g.min.y - GUIDE_LIFT))).with_y(0.0);
                (centre, lean, vec![normal, -normal])
            }
            _ => (
                ((g.min + g.max) * 0.5).with_y(0.0),
                Vec3::ZERO,
                (0..16).map(|k| {
                    let a = k as f32 * std::f32::consts::TAU / 16.0;
                    Vec3::new(a.cos(), 0.0, a.sin())
                }).collect(),
            ),
        };
        let (lo, hi) = (g.min.y - GUIDE_LIFT, g.max.y - GUIDE_LIFT);
        // Floor height near `y` under a horizontal point, if any.
        let floor_near = |p: Vec3, y: f32| {
            world
                .raycast(p.with_y(y + 1.2), Vec3::NEG_Y, 2.4)
                .filter(|h| h.normal.y > 0.6)
                .map(|h| y + 1.2 - h.distance)
                .filter(|f| (f - y).abs() < 0.4)
        };
        let best = sides
            .into_iter()
            .filter_map(|d| {
                let top = floor_near(centre + lean * (hi - lo) + d * (STAND + 0.3), hi)?;
                let bottom = floor_near(centre - d * (STAND + 0.1), lo)?;
                // The climbing side must be open at chest height.
                let open = world.raycast((centre - d * 0.1).with_y(bottom + 1.3), -d, STAND + 0.2).is_none();
                // Ties (a guide with no ladder mesh) go to the axis-aligned direction.
                let skew = 1.0 - d.x.abs().max(d.z.abs());
                open.then_some((d, bottom, top, (top - hi).abs() + (bottom - lo).abs() + 0.05 * skew))
            })
            .min_by(|a, b| a.3.total_cmp(&b.3));
        match best {
            Some((face, bottom, top, _)) => {
                info!("ladder {}: {:.1} m, foot {:.2} top {:.2}, at {:.2?} facing {:.2?}, lean {:.2?}/m", g.name, top - bottom, bottom, top, centre, face, lean);
                ladders.push(Ladder { name: g.name.clone(), centre, lean, face, bottom, top });
            }
            None => warn!("ladder {}: no foot and landing found around {:.1?}; not climbable", g.name, centre),
        }
    }
    Ladders(ladders)
}

/// 47 on a ladder: which one, which way, and how far through the current stage.
#[derive(Component)]
pub struct Climbing {
    ladder: usize,
    up: bool,
    stage: Stage,
    /// Where the current stage started.
    from: Vec3,
    t: f32,
}

/// Up: `On` (step to the foot of the rungs) → `Climb` → `Over` (the top) → `Off`.
/// Down: `On` (step to the top of the rungs) → `Over` (backwards) → `Climb` → `Off`.
#[derive(PartialEq)]
enum Stage {
    On,
    Climb,
    Over,
    Off,
}

/// Offers the ladder when 47 stands at either end of one; E starts the climb.
fn offer(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    ladders: Res<Ladders>,
    mut mission: ResMut<Mission>,
    player: Query<(Entity, &Transform), (With<Player>, Without<Climbing>)>,
) {
    let Ok((e, t)) = player.single() else { return };
    if mission.busy > 0.0 || mission.outcome.is_some() || mission.dragging_body.is_some() || mission.prompt.is_some() {
        return;
    }
    let me = t.translation;
    let near = |p: Vec3| me.with_y(0.0).distance(p.with_y(0.0)) < REACH && (me.y - p.y).abs() < 0.8;
    for (i, l) in ladders.0.iter().enumerate() {
        let up = if near(l.foot()) {
            true
        } else if near(l.landing()) {
            false
        } else {
            continue;
        };
        mission.prompt = Some(if up { "[E] Climb up the ladder" } else { "[E] Climb down the ladder" }.to_owned());
        if keys.just_pressed(KeyCode::KeyE) {
            info!("climbing {} {}", if up { "up" } else { "down" }, l.name);
            commands.entity(e).insert(Climbing { ladder: i, up, stage: Stage::On, from: me, t: 0.0 });
        }
        return;
    }
}

/// Moves and animates 47 through a climb.
#[allow(clippy::too_many_arguments)]
fn climb(
    mut commands: Commands,
    time: Res<Time>,
    ladders: Res<Ladders>,
    mut mission: ResMut<Mission>,
    mut player: Query<(Entity, &mut Transform, &mut Player, &mut Climbing)>,
    mut rigs: Query<(&mut Gait, &mut Animator)>,
    mut camera: Query<&mut OrbitCamera>,
) {
    let Ok((e, mut t, mut p, mut c)) = player.single_mut() else { return };
    let Some(l) = ladders.0.get(c.ladder) else {
        commands.entity(e).remove::<Climbing>();
        return;
    };
    let Some((mut gait, mut anim)) = p.model.and_then(|m| rigs.get_mut(m).ok()) else { return };
    let dt = time.delta_secs();
    // Nothing else while on a ladder: no actions, no weapons, no locomotion.
    mission.busy = mission.busy.max(0.1);
    gait.hold = true;
    p.vertical_speed = 0.0;
    p.speed = 0.0;
    t.rotation = Transform::IDENTITY.looking_to(l.face, Vec3::Y).rotation;
    // The camera swings round behind him, facing the ladder.
    if let Ok(mut orbit) = camera.single_mut() {
        let want = (-l.face.x).atan2(-l.face.z);
        let diff = (want - orbit.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        orbit.yaw += diff * (4.0 * dt).min(1.0);
    }
    // The climb-over spans the top `OVER_RISE` (all of a short ladder).
    let over_low = (l.top - OVER_RISE).max(l.bottom);
    c.t += dt;
    let next = |c: &mut Climbing, stage: Stage, from: Vec3| {
        c.stage = stage;
        c.from = from;
        c.t = 0.0;
    };
    match c.stage {
        Stage::On => {
            let start = if c.up { l.hang(l.bottom) } else { l.hang(l.top) };
            if c.up {
                anim.play_range(CLIP, 0.0, STROKE.0, STROKE.1, true);
            } else {
                // Getting on from the top is the climb-over backwards, held at its last frame.
                anim.play_range(CLIP, -0.0, OVER.0, OVER.1, false);
            }
            let k = (c.t / STEP_ON).min(1.0);
            t.translation = c.from.lerp(start, k);
            if k >= 1.0 {
                if c.up {
                    next(&mut c, Stage::Climb, start);
                } else {
                    next(&mut c, Stage::Over, start);
                }
            }
        }
        Stage::Climb => {
            let rate = if c.up { 1.0 } else { -1.0 };
            anim.play_range(CLIP, rate, STROKE.0, STROKE.1, true);
            let y = t.translation.y + rate * CLIMB_SPEED * dt;
            let (end, done) = if c.up { (over_low, y >= over_low) } else { (l.bottom, y <= l.bottom) };
            t.translation = l.hang(if done { end } else { y });
            if done {
                if c.up {
                    next(&mut c, Stage::Over, t.translation);
                } else {
                    next(&mut c, Stage::Off, t.translation);
                }
            }
        }
        Stage::Over => {
            let k = (c.t / OVER_SECONDS).min(1.0);
            if c.up {
                anim.play_range(CLIP, 1.0, OVER.0, OVER.1, false);
                t.translation = l.hang(over_low.lerp(l.top, k));
            } else {
                anim.play_range(CLIP, -1.0, OVER.0, OVER.1, false);
                t.translation = l.hang(l.top.lerp(over_low, k));
            }
            if k >= 1.0 {
                if c.up {
                    next(&mut c, Stage::Off, t.translation);
                } else if over_low > l.bottom {
                    next(&mut c, Stage::Climb, t.translation);
                } else {
                    next(&mut c, Stage::Off, t.translation);
                }
            }
        }
        Stage::Off => {
            gait.hold = false;
            let end = if c.up { l.landing() } else { l.foot() };
            let k = (c.t / STEP_OFF).min(1.0);
            t.translation = c.from.lerp(end, k);
            if k >= 1.0 {
                p.grounded = true;
                mission.busy = 0.0;
                commands.entity(e).remove::<Climbing>();
            }
        }
    }
}
