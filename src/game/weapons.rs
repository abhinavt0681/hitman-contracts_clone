//! Guns and the fiber wire, on the original's numbers.
//!
//! The level's weapon templates (`Gun_Hardballer_Silenced_01`, `Rifle_AK47_01`, …, GMS class
//! 0x100429) carry the engine's weapon schema: `Operation` (manual / semi / full auto),
//! `ShotsPerMagazine`, `Precision(deg)`, `NearRange(m)` / `FarRange(m)`, `DamageMul`,
//! `ShotsPerMinute`; their ammo templates (`Ammo45ACP`, `Ammo556`, …) carry `NearDamage` /
//! `FarDamage`. Characters have `m_iHitpoints = 20` (the `Being` script subtracts the damage
//! the engine computes). The engine's damage code isn't decoded yet, so ammo damage is scaled
//! by `DAMAGE_SCALE` with per-zone multipliers chosen to play like the original (a Silverballer
//! round to the chest at close range kills, a head shot always does).

use super::*;
use crate::{
    audio::SoundBank,
    character::{AnimLibrary, Animator, Gait, Rig},
    collision::WorldCollision,
    player::{OrbitCamera, Player},
};
use bevy::{
    audio::{PlaybackSettings, SpatialScale, Volume},
    input::mouse::MouseButton,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

/// One gun's stats, from its GMS weapon and ammo templates.
pub struct WeaponDef {
    pub automatic: bool,
    pub magazine: u32,
    /// Cone half-angle of a perfect shot (degrees).
    pub precision: f32,
    pub near_range: f32,
    pub far_range: f32,
    pub damage_mul: f32,
    pub near_damage: f32,
    pub far_damage: f32,
    pub shots_per_minute: f32,
    pub silenced: bool,
    /// Upper-body clips: held, aimed, reloading (and how long the reload takes).
    pub hold: &'static str,
    pub aim: &'static str,
    /// Where in the aim clip's sweep of directions the gun points straight ahead.
    pub aim_frame: f32,
    pub reload: &'static str,
    pub reload_seconds: f32,
}

/// `Gun_Hardballer_Silenced_01` + `Ammo45ACP`: semi-auto, 14 rounds, 0.35°, 30–80 m,
/// 2.1 → 0.7 damage, 300 rounds/min.
pub const SILVERBALLER: WeaponDef = WeaponDef {
    automatic: false,
    magazine: 14,
    precision: 0.35,
    near_range: 30.0,
    far_range: 80.0,
    damage_mul: 1.0,
    near_damage: 2.1,
    far_damage: 0.7,
    shots_per_minute: 300.0,
    silenced: true,
    hold: "UB_Hold_Gun_RightHand",
    aim: "UB_Aim_Gun",
    aim_frame: 0.0,
    reload: "UB_Reload_Hardballer",
    reload_seconds: 2.2,
};

/// `Rifle_AK47_01` + `Ammo556`: full auto, 30 rounds, 0.6°, 150–200 m, 1.8 → 1.2 damage,
/// 600 rounds/min. The guards carry these.
pub const AK47: WeaponDef = WeaponDef {
    automatic: true,
    magazine: 30,
    precision: 0.6,
    near_range: 150.0,
    far_range: 200.0,
    damage_mul: 1.0,
    near_damage: 1.8,
    far_damage: 1.2,
    shots_per_minute: 600.0,
    silenced: false,
    hold: "UB_Hold_Rifle",
    aim: "UB_Aim_Rifle",
    // The clip opens on a one-armed reach; by 0.6 s the rifle is shouldered.
    aim_frame: 0.6,
    reload: "UB_Reload_Rifle",
    reload_seconds: 1.8,
};

/// Ammo damage → hitpoints.
const DAMAGE_SCALE: f32 = 10.0;
/// Guards within this of a silenced shot know it for what it is; further off it is a noise.
const SILENCED_ALARM: f32 = 5.0;
const SILENCED_HEARD: f32 = 15.0;
/// An unsilenced shot is heard, and recognised, this far.
const LOUD_HEARD: f32 = 60.0;
/// A guard's delay between taking his stance and the first burst (s).
const REACTION_MIN: f32 = 0.8;
const REACTION_MAX: f32 = 1.4;
/// How long a fiber-wire kill holds both men (the `Die_Carrot` clip).
const STRANGLE_SECONDS: f32 = 4.7;

/// Where a bullet lands on a character.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Zone {
    Head,
    Torso,
    Limb,
}

impl Zone {
    fn multiplier(self) -> f32 {
        match self {
            Zone::Head => 5.0,
            Zone::Torso => 1.0,
            Zone::Limb => 0.5,
        }
    }

    /// From the hit's height above the feet (m).
    fn at(height: f32) -> Zone {
        if height > 1.5 {
            Zone::Head
        } else if height > 0.95 {
            Zone::Torso
        } else {
            Zone::Limb
        }
    }
}

/// Damage (hitpoints) of one round from `w` hitting `zone` at `dist` metres.
pub fn damage(w: &WeaponDef, dist: f32, zone: Zone) -> f32 {
    let t = ((dist - w.near_range) / (w.far_range - w.near_range).max(1.0)).clamp(0.0, 1.0);
    w.near_damage.lerp(w.far_damage, t) * w.damage_mul * DAMAGE_SCALE * zone.multiplier()
}

/// Rounds in the gun and carried.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ammo {
    pub loaded: u32,
    pub spare: u32,
}

/// 47's handling of the gun in hand.
#[derive(Resource, Default)]
pub struct Gunplay {
    cooldown: f32,
    /// Seconds left of a reload, and which sounds of it have played.
    reloading: f32,
    reload_step: u8,
    /// Extra spread from shots fired in quick succession (degrees), decaying.
    bloom: f32,
    /// The item the pose was last set for, to play draw / tighten animations on change.
    posed: Option<Item>,
}

impl Gunplay {
    /// Seconds left of a reload in progress.
    pub fn reloading(&self) -> f32 {
        self.reloading
    }

    /// The spread (degrees) of the next shot: the weapon's precision, times how steady 47 is
    /// (aimed or from the hip; standing, walking or running; crouched), plus recoil bloom.
    pub fn cone(&self, w: &WeaponDef, aim: f32, speed: f32, crouching: bool) -> f32 {
        let steadiness = if aim > 0.5 { 1.0 } else { 4.0 } * if speed > 2.5 { 4.0 } else if speed > 0.3 { 2.0 } else { 1.0 } * if crouching { 0.7 } else { 1.0 };
        w.precision * steadiness + self.bloom
    }
}

/// A short muzzle flash.
#[derive(Component)]
pub(super) struct Flash(f32);

/// The gun 47 holds, if any.
pub fn weapon_of(item: Item) -> Option<&'static WeaponDef> {
    match item {
        Item::SilencedPistol => Some(&SILVERBALLER),
        Item::AssaultRifle => Some(&AK47),
        _ => None,
    }
}

fn ammo_mut(mission: &mut Mission, item: Item) -> Option<&mut Ammo> {
    match item {
        Item::SilencedPistol => Some(&mut mission.pistol),
        Item::AssaultRifle => mission.rifle.as_mut(),
        _ => None,
    }
}

/// Random number in 0..1 (xorshift; shooting needs no better).
fn rand01(state: &mut u32) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as f32 / (1u32 << 24) as f32
}

/// `dir` deflected by up to `degrees` in a random direction.
fn spread(dir: Vec3, degrees: f32, rng: &mut u32) -> Vec3 {
    let side = dir.any_orthonormal_vector();
    let up = dir.cross(side);
    let angle = degrees.to_radians() * rand01(rng).sqrt();
    let around = rand01(rng) * std::f32::consts::TAU;
    (dir * angle.cos() + (side * around.cos() + up * around.sin()) * angle.sin()).normalize()
}

/// Nearest point along a ray where it passes through a character (a stack of spheres from
/// shins to head), with that point's height above the feet.
fn ray_hits_body(origin: Vec3, dir: Vec3, feet: Vec3, max: f32) -> Option<(f32, f32)> {
    const PARTS: [(f32, f32); 6] = [(0.3, 0.17), (0.65, 0.17), (1.0, 0.22), (1.25, 0.24), (1.45, 0.22), (1.66, 0.13)];
    PARTS
        .iter()
        .filter_map(|&(h, r)| {
            let c = feet + Vec3::Y * h;
            let along = (c - origin).dot(dir);
            (along > 0.0 && along < max && (origin + dir * along).distance(c) < r).then_some((along, h))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

fn pick<'a>(sounds: &'a [Handle<AudioSource>], rng: &mut u32) -> Option<&'a Handle<AudioSource>> {
    (!sounds.is_empty()).then(|| &sounds[(rand01(rng) * sounds.len() as f32) as usize % sounds.len()])
}

/// A sound placed in the world.
fn play_at(commands: &mut Commands, sound: &Handle<AudioSource>, at: Vec3, volume: f32, reference: f32) {
    commands.spawn((
        AudioPlayer::new(sound.clone()),
        PlaybackSettings::DESPAWN.with_spatial(true).with_spatial_scale(SpatialScale::new(1.0 / reference)).with_volume(Volume::Linear(volume)),
        Transform::from_translation(at),
    ));
}

fn flash(commands: &mut Commands, at: Vec3, strength: f32) {
    commands.spawn((PointLight { color: Color::srgb(1.0, 0.8, 0.5), intensity: 60_000.0 * strength, range: 6.0, ..default() }, Transform::from_translation(at), Flash(0.05)));
}

pub(super) fn fade_flashes(mut commands: Commands, time: Res<Time>, mut flashes: Query<(Entity, &mut Flash)>) {
    for (e, mut f) in &mut flashes {
        f.0 -= time.delta_secs();
        if f.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// What a hit does to a character: damage, a flinch, the death, and who notices.
#[allow(clippy::too_many_arguments)]
pub(super) fn wound(
    commands: &mut Commands,
    lib: &AnimLibrary,
    bank: Option<&SoundBank>,
    mission: &mut Mission,
    text: &Text,
    target: Entity,
    actor: &mut Actor,
    anim: &mut Animator,
    at: Vec3,
    points: f32,
    shooter: Vec3,
    rng: &mut u32,
) {
    actor.health -= points;
    if let Some(b) = bank {
        if let Some(s) = pick(&b.impact_flesh, rng) {
            play_at(commands, s, at, 0.7, 3.0);
        }
    }
    if actor.health <= 0.0 {
        actor.alive = false;
        let clip = lib.first_of(anim.set(), &["Die_Backward", "Die_Forward"]).unwrap_or_default();
        anim.play_once(&clip);
        // The round knocks him down: he goes limp at once (`ragdoll`).
        commands.entity(target).remove::<Gait>().insert(crate::ragdoll::Shot { at, impulse: (at - shooter).normalize_or_zero() * crate::ragdoll::SHOT_PUSH });
        if let Some(s) = bank.and_then(|b| pick(&b.death_cries, rng)) {
            play_at(commands, s, at, 0.8, 4.0);
        }
        let (kind, name) = (actor.kind, actor.name.clone());
        on_kill(mission, text, kind, &name);
    } else {
        // He knows he's been shot, and by whom.
        if actor.kind == Kind::Guard {
            actor.alerted = true;
            actor.last_seen = Some(shooter);
            actor.lost_for = 0.0;
        }
        let clip = lib.first_of(anim.set(), &["HitTorsoFront", "HitTorsoFront01", "HitShoulderLeftFront"]).unwrap_or_default();
        anim.play_once(&clip);
        if let Some(s) = bank.and_then(|b| pick(&b.pain_cries, rng)) {
            play_at(commands, s, at, 0.8, 4.0);
        }
    }
}

/// Aiming (right mouse), firing (left), reloading (R) the gun in hand.
#[allow(clippy::too_many_arguments)]
pub(super) fn gun_input(
    mut commands: Commands,
    time: Res<Time>,
    (mouse, keys): (Res<ButtonInput<MouseButton>>, Res<ButtonInput<KeyCode>>),
    world: Res<WorldCollision>,
    (text, lib): (Res<Text>, Res<AnimLibrary>),
    bank: Option<Res<SoundBank>>,
    mut mission: ResMut<Mission>,
    mut gun: ResMut<Gunplay>,
    mut rng: Local<u32>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut camera: Query<(&GlobalTransform, &mut OrbitCamera)>,
    player: Query<(&GlobalTransform, &Player)>,
    (rigs, joints): (Query<&Rig>, Query<&GlobalTransform, Without<Player>>),
    mut animators: Query<&mut Animator, Without<Actor>>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator), Without<Player>>,
) {
    if *rng == 0 {
        *rng = 0x9e37_79b9;
    }
    let dt = time.delta_secs();
    gun.cooldown = (gun.cooldown - dt).max(0.0);
    gun.bloom = (gun.bloom - 3.0 * dt).max(0.0);
    let (Ok((cam, mut orbit)), Ok((pt, p))) = (camera.single_mut(), player.single()) else { return };
    let weapon = weapon_of(mission.active_item);
    let free = mission.outcome.is_none() && mission.busy <= 0.0 && mission.dragging_body.is_none();
    let aiming = weapon.is_some() && free && mouse.pressed(MouseButton::Right);
    orbit.aim = (orbit.aim + if aiming { 6.0 } else { -6.0 } * dt).clamp(0.0, 1.0);
    let Some(w) = weapon else {
        gun.reloading = 0.0;
        return;
    };
    let item = mission.active_item;
    let bank = bank.as_deref();
    let me = pt.translation();
    let mut anim = p.model.and_then(|m| animators.get_mut(m).ok());

    // Reloading: the magazine comes out, goes in, the slide is racked.
    if gun.reloading > 0.0 {
        gun.reloading -= dt;
        let done = 1.0 - gun.reloading / w.reload_seconds;
        let rifle = item == Item::AssaultRifle;
        let steps = [(0.1, if rifle { bank.map(|b| &b.rifle_clip_out) } else { bank.map(|b| &b.pistol_clip_out) }),
            (0.55, if rifle { bank.map(|b| &b.rifle_clip_in) } else { bank.map(|b| &b.pistol_clip_in) }),
            (0.85, if rifle { bank.map(|b| &b.rifle_chamber) } else { bank.map(|b| &b.pistol_chamber) })];
        for (k, (at, sound)) in steps.iter().enumerate() {
            if done >= *at && gun.reload_step as usize == k {
                gun.reload_step += 1;
                if let Some(s) = sound {
                    crate::audio::play_sound(&mut commands, s, 0.6);
                }
            }
        }
        if gun.reloading <= 0.0 {
            if let Some(ammo) = ammo_mut(&mut mission, item) {
                let take = (w.magazine - ammo.loaded).min(ammo.spare);
                ammo.loaded += take;
                ammo.spare -= take;
            }
        }
        return;
    }
    let ammo = ammo_mut(&mut mission, item).copied().unwrap_or_default();
    let wants_reload = keys.just_pressed(KeyCode::KeyR) || (ammo.loaded == 0 && mouse.just_pressed(MouseButton::Left));
    if wants_reload && free && ammo.loaded < w.magazine && ammo.spare > 0 {
        gun.reloading = w.reload_seconds;
        gun.reload_step = 0;
        if let Some(a) = anim.as_mut() {
            a.play_upper(w.reload, false);
        }
        return;
    }

    let triggered = if w.automatic { mouse.pressed(MouseButton::Left) } else { mouse.just_pressed(MouseButton::Left) };
    if !triggered || !free || gun.cooldown > 0.0 {
        return;
    }
    if cursor.single().is_ok_and(|c| c.grab_mode == CursorGrabMode::None) {
        return; // this click captures the mouse
    }
    if ammo.loaded == 0 {
        if mouse.just_pressed(MouseButton::Left) {
            if let Some(b) = bank {
                crate::audio::play_sound(&mut commands, &b.empty_click, 0.6);
            }
        }
        return;
    }
    if let Some(a) = ammo_mut(&mut mission, item) {
        a.loaded -= 1;
    }
    gun.cooldown = 60.0 / w.shots_per_minute;

    // Where the round goes: the crosshair, spread by the weapon's precision and how steady
    // 47 is (aimed or from the hip, standing, walking or running, crouched).
    let cone = gun.cone(w, orbit.aim, p.speed(), p.crouching);
    gun.bloom += w.precision * 2.0;
    let fwd = cam.forward().as_vec3();
    let dir = spread(fwd, cone, &mut rng);
    // Start past 47 so the third-person camera never shoots him in the back.
    let origin = cam.translation() + fwd * (me - cam.translation()).dot(fwd).max(0.0);
    let muzzle = p
        .model
        .and_then(|m| rigs.get(m).ok())
        .and_then(|r| r.joint("RIGHT_HAND"))
        .and_then(|j| joints.get(j).ok())
        .map_or(me + Vec3::Y * 1.4, |g| g.translation());

    if let Some(b) = bank {
        let shot = if item == Item::AssaultRifle { &b.rifle_shot } else { &b.shot_silenced };
        crate::audio::play_sound(&mut commands, shot, if w.silenced { 0.7 } else { 1.0 });
    }
    flash(&mut commands, muzzle + dir * 0.3, if w.silenced { 0.25 } else { 1.0 });

    let wall = world.raycast(origin, dir, 300.0);
    let wall_at = wall.as_ref().map_or(300.0, |h| h.distance);
    let target = actors
        .iter()
        .filter(|(_, _, a, _)| a.alive)
        .filter_map(|(e, t, _, _)| ray_hits_body(origin, dir, t.translation(), wall_at).map(|(d, h)| (e, d, h)))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match target {
        Some((e, d, h)) => {
            if let Ok((_, _, mut a, mut victim_anim)) = actors.get_mut(e) {
                let points = damage(w, me.distance(origin + dir * d), Zone::at(h));
                wound(&mut commands, &lib, bank, &mut mission, &text, e, &mut a, &mut victim_anim, origin + dir * d, points, me, &mut rng);
            }
        }
        None => {
            if let (Some(hit), Some(b)) = (wall, bank) {
                if let Some(s) = b.impacts.get(&hit.surface).and_then(|v| pick(v, &mut rng)) {
                    play_at(&mut commands, s, origin + dir * hit.distance, 0.6, 3.0);
                }
            }
        }
    }

    // Who hears it: a silenced round only up close; an AK across the base.
    let (alarm, heard) = if w.silenced { (SILENCED_ALARM, SILENCED_HEARD) } else { (LOUD_HEARD, LOUD_HEARD) };
    let mut alarmed = false;
    for (_, t, mut a, _) in &mut actors {
        if a.alive && a.kind == Kind::Guard && t.translation().distance(me) < alarm {
            a.alerted = true;
            a.last_seen = Some(me);
            a.lost_for = 0.0;
            alarmed = true;
        }
    }
    mission.noises.push((me, heard));
    if alarmed {
        raise_alarm(Some(&mut commands), bank, &mut mission, &text, None);
    }
}

/// 47's arms: holding, aiming or reloading the gun, or the fiber wire pulled taut.
pub(super) fn arm_pose(
    mut commands: Commands,
    mission: Res<Mission>,
    mut gun: ResMut<Gunplay>,
    bank: Option<Res<SoundBank>>,
    camera: Query<&OrbitCamera>,
    player: Query<&Player>,
    mut animators: Query<&mut Animator>,
) {
    let (Ok(p), Ok(orbit)) = (player.single(), camera.single()) else { return };
    let Some(mut anim) = p.model.and_then(|m| animators.get_mut(m).ok()) else { return };
    let item = mission.active_item;
    let changed = gun.posed != Some(item);
    gun.posed = Some(item);
    if mission.busy > 0.0 || mission.dragging_body.is_some() {
        anim.stop_upper();
        return;
    }
    if gun.reloading > 0.0 {
        return; // the reload clip is playing
    }
    match (item, weapon_of(item)) {
        (_, Some(w)) => {
            if changed {
                anim.play_upper("UB_Pull_Gun_Right", false);
            } else if anim.upper().is_none_or(|(c, t)| c != "UB_Pull_Gun_Right" || t > 0.4) {
                anim.play_upper(if orbit.aim > 0.3 { w.aim } else { w.hold }, true);
                if orbit.aim > 0.3 {
                    // The original's aim clips are a sweep of directions (the engine picks
                    // the frame for where the gun points); hold the straight-ahead frame, and
                    // 47 turns to face the crosshair.
                    anim.pose_upper(w.aim_frame);
                }
            }
        }
        (Item::FiberWire, None) => {
            // Pull the wire taut, then hold it ready.
            if changed {
                anim.play_upper("UB_Tighten_Carrot01", false);
                if let Some(b) = &bank {
                    crate::audio::play_sound(&mut commands, &b.fiber_wire, 0.5);
                }
            } else if anim.upper().is_none_or(|(c, t)| c != "UB_Tighten_Carrot01" || t > 2.0) {
                anim.play_upper("UB_Hold_Carrot", true);
            }
        }
        _ => anim.stop_upper(),
    }
}

/// A fiber-wire kill from behind: 47 plays `Carrot`, the victim `Die_Carrot`, standing just in
/// front of him and facing the same way, for the 4.7 s the clips last.
#[allow(clippy::too_many_arguments)]
pub(super) fn strangle(
    commands: &mut Commands,
    lib: &AnimLibrary,
    bank: Option<&SoundBank>,
    mission: &mut Mission,
    text: &Text,
    me: &GlobalTransform,
    hitman: Option<&mut Animator>,
    victim: Entity,
    actor: &mut Actor,
    victim_anim: &mut Animator,
    victim_transform: &mut Transform,
) {
    let fwd = (me.rotation() * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
    victim_transform.translation = me.translation() + fwd * 0.45;
    victim_transform.rotation = facing(fwd);
    actor.alive = false;
    let clip = lib.first_of(victim_anim.set(), &["Die_Carrot", "Die_Throat_Cut_Alt_01", "Die_Forward"]).unwrap_or_default();
    victim_anim.play_once(&clip);
    commands.entity(victim).remove::<Gait>();
    if let Some(h) = hitman {
        h.stop_upper();
        let clip = lib.first_of(h.set(), &["Carrot", "Cut_Throat"]).unwrap_or_default();
        h.play_once(&clip);
    }
    if let Some(b) = bank {
        crate::audio::play_sound(commands, &b.fiber_wire, 0.8);
    }
    mission.busy = STRANGLE_SECONDS;
    let (kind, name) = (actor.kind, actor.name.clone());
    on_kill(mission, text, kind, &name);
}

/// The guards' AK-47s: bursts of 3–6 rounds at 600 a minute, each a real shot whose spread
/// grows with range, with 47 moving, in the dark and in a squall.
#[allow(clippy::too_many_arguments)]
pub(super) fn guard_fire(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<WorldCollision>,
    exposure: Res<crate::atmosphere::Exposure>,
    bank: Option<Res<SoundBank>>,
    mut mission: ResMut<Mission>,
    mut rng: Local<u32>,
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(&GlobalTransform, &mut Actor, &Gait)>,
) {
    if *rng == 0 {
        *rng = 0x2545_f491;
    }
    if mission.outcome.is_some() {
        return;
    }
    let dt = time.delta_secs();
    let Ok((pt, p)) = player.single() else { return };
    let chest = pt.translation() + Vec3::Y * 1.2;
    for (gt, mut a, gait) in &mut actors {
        // Standing to shoot, with 47 in sight a moment ago (`guard_combat` has already added
        // this frame's time to `lost_for`).
        if !a.alive || !a.alerted || a.kind != Kind::Guard || gait.speed > 0.1 || a.lost_for > 0.3 {
            a.burst = 0;
            a.sighting = 0.0;
            continue;
        }
        // Raising the rifle and taking aim (`m_fReactionTime`; its value isn't decoded yet).
        if a.sighting == 0.0 {
            a.fire_cooldown = REACTION_MIN + (REACTION_MAX - REACTION_MIN) * rand01(&mut rng);
        }
        a.sighting += dt;
        let (eye, _) = npc_view(gt);
        let dist = eye.distance(chest);
        if dist > ai::RIFLE_RANGE {
            continue;
        }
        a.fire_cooldown -= dt;
        if a.fire_cooldown > 0.0 {
            continue;
        }
        if a.burst == 0 {
            a.burst = 3 + (rand01(&mut rng) * 4.0) as u8;
        }
        a.burst -= 1;
        a.fire_cooldown = if a.burst == 0 { 0.7 + 0.7 * rand01(&mut rng) } else { 60.0 / AK47.shots_per_minute };

        // Aim error on top of the rifle's own 0.6°.
        let moving = if p.speed() > 2.5 { 3.0 } else if p.speed() > 0.3 { 1.5 } else { 0.0 };
        let error = AK47.precision + 1.2 + 0.06 * dist + moving + 1.5 * (1.0 - exposure.light) + 2.0 * exposure.gust;
        let aim = (chest - eye).normalize();
        let dir = spread(aim, error, &mut rng);
        let muzzle = eye + aim * 0.6;
        if let Some(b) = &bank {
            play_at(&mut commands, &b.rifle_shot, muzzle, 0.9, 6.0);
        }
        flash(&mut commands, muzzle, 1.0);
        let wall = world.raycast(eye, dir, dist + 1.0).map_or(f32::MAX, |h| h.distance);
        if let Some((d, h)) = ray_hits_body(eye, dir, pt.translation(), wall) {
            mission.health -= damage(&AK47, d, Zone::at(h));
            if let Some(s) = bank.as_ref().and_then(|b| pick(&b.impact_flesh, &mut rng)) {
                crate::audio::play_sound(&mut commands, s, 0.6);
            }
        }
    }
    if mission.health <= 0.0 {
        mission.health = 0.0;
        mission.outcome = Some(Outcome::Failed);
    }
}

/// The weapon models in 47's right hand (one per kind, shown when that item is out).
#[derive(Component)]
pub(super) struct HeldWeapons {
    hand: Entity,
    models: Vec<(Item, Entity)>,
}

/// Static meshes keep X and Z swapped relative to skeleton bones (see `view.rs` and
/// `character.rs`): a weapon's grip frame needs that swap to sit in the hand bone's frame.
const SWAP_XZ: Mat4 = Mat4::from_cols(Vec4::Z, Vec4::Y, Vec4::X, Vec4::W);

/// Template whose model stands for `item` in the hand.
pub fn model_for(item: Item) -> Option<&'static str> {
    match item {
        Item::SilencedPistol => Some("Gun_Hardballer_Silenced_01"),
        Item::AssaultRifle => Some("Rifle_AK47_01"),
        Item::FiberWire => Some("CC_FiberWire_01"),
        _ => None,
    }
}

/// Puts the guns and the wire in 47's right hand (again after a change of clothes rebuilds
/// his rig), placed so the template's `PosBox_Hand` sits in the palm, and shows the one in use.
pub(super) fn hold_models(
    mut commands: Commands,
    mission: Res<Mission>,
    models: Option<Res<crate::view::WeaponModels>>,
    player: Query<&Player>,
    rigs: Query<(&Rig, Option<&HeldWeapons>)>,
    mut visibility: Query<&mut Visibility>,
) {
    let (Some(models), Ok(p)) = (models, player.single()) else { return };
    let Some(root) = p.model else { return };
    let Ok((rig, held)) = rigs.get(root) else { return };
    let Some(hand) = rig.joint("RIGHT_HAND") else { return };
    let held = match held {
        Some(h) if h.hand == hand => h,
        _ => {
            let mut spawned = Vec::new();
            for item in [Item::SilencedPistol, Item::AssaultRifle, Item::FiberWire] {
                let Some(model) = model_for(item).and_then(|n| models.0.get(n)) else { continue };
                let place = Transform::from_matrix(SWAP_XZ * model.grip.to_matrix().inverse());
                let holder = commands.spawn((place, Visibility::Hidden)).id();
                for (mesh, material, t) in &model.parts {
                    let part = commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), *t)).id();
                    commands.entity(holder).add_child(part);
                }
                // Joints carry no visibility of their own; give the hand one so what it holds draws.
                commands.entity(hand).insert_if_new(Visibility::Inherited).add_child(holder);
                spawned.push((item, holder));
            }
            commands.entity(root).insert(HeldWeapons { hand, models: spawned });
            return;
        }
    };
    for (item, e) in &held.models {
        if let Ok(mut v) = visibility.get_mut(*e) {
            let want = if *item == mission.active_item && mission.busy <= 0.0 { Visibility::Inherited } else { Visibility::Hidden };
            if *v != want {
                *v = want;
            }
        }
    }
}


/// A gun 47 put down (X), where he left it, with its rounds.
#[derive(Component)]
pub(super) struct DroppedGun {
    item: Item,
    ammo: Ammo,
}

/// X puts the gun in hand on the ground (before a frisk, say); E picks it up again.
#[allow(clippy::too_many_arguments)]
pub(super) fn drop_and_pick_up(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    models: Option<Res<crate::view::WeaponModels>>,
    meshes: Res<Assets<Mesh>>,
    bank: Option<Res<SoundBank>>,
    text: Res<Text>,
    mut mission: ResMut<Mission>,
    player: Query<&GlobalTransform, With<Player>>,
    guns: Query<(Entity, &GlobalTransform, &DroppedGun)>,
) {
    if mission.busy > 0.0 || mission.outcome.is_some() {
        return;
    }
    let Ok(pt) = player.single() else { return };
    let me = pt.translation();
    if keys.just_pressed(KeyCode::KeyX) {
        let item = mission.active_item;
        let ammo = match item {
            Item::SilencedPistol if mission.carrying_pistol => {
                mission.carrying_pistol = false;
                Some(mission.pistol)
            }
            Item::AssaultRifle => mission.rifle.take(),
            _ => None,
        };
        let Some(ammo) = ammo else { return };
        mission.active_item = Item::Holstered;
        let fwd = (pt.rotation() * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
        let at = me + fwd * 0.5;
        let holder = commands.spawn((DroppedGun { item, ammo }, Transform::from_translation(at), Visibility::Inherited, Name::new(format!("Dropped {item:?}")))).id();
        if let Some(model) = model_for(item).and_then(|n| models.as_ref()?.0.get(n)) {
            let (rotation, lift) = lay_flat(model, &meshes);
            commands.entity(holder).insert(Transform::from_translation(at + Vec3::Y * lift).with_rotation(Quat::from_rotation_y(pt.rotation().to_euler(EulerRot::YXZ).0) * rotation));
            for (mesh, material, t) in &model.parts {
                let part = commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), *t)).id();
                commands.entity(holder).add_child(part);
            }
        }
        if let Some(b) = &bank {
            crate::audio::play_sound(&mut commands, if item == Item::AssaultRifle { &b.drop_rifle } else { &b.drop_gun }, 0.6);
        }
        return;
    }
    if mission.prompt.is_some() {
        return;
    }
    let Some((e, gun)) = guns
        .iter()
        .map(|(e, t, g)| (e, g, t.translation().with_y(0.0).distance(me.with_y(0.0))))
        .filter(|(_, _, d)| *d < 1.3)
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(e, g, _)| (e, g))
    else {
        return;
    };
    let name = if gun.item == Item::AssaultRifle { "AK-47" } else { "Silverballer" };
    mission.prompt = Some(format!("[E] Pick up the {name}"));
    if keys.just_pressed(KeyCode::KeyE) {
        match gun.item {
            Item::SilencedPistol => {
                mission.carrying_pistol = true;
                mission.pistol = gun.ammo;
            }
            _ => match mission.rifle.as_mut() {
                Some(r) => r.spare += gun.ammo.loaded + gun.ammo.spare,
                None => mission.rifle = Some(gun.ammo),
            },
        }
        mission.active_item = gun.item;
        mission.picked_up(&text, if gun.item == Item::AssaultRifle { "Rifle_AK47_01" } else { "Gun_Hardballer_Silenced_01" }, name);
        commands.entity(e).despawn();
        if let Some(b) = &bank {
            crate::audio::play_sound(&mut commands, if gun.item == Item::AssaultRifle { &b.pickup_rifle } else { &b.pickup_gun }, 0.6);
        }
    }
}

/// Turns a weapon model to lie flat (its thinnest axis up, longest along X) and how high to
/// lift it to rest on the floor.
fn lay_flat(model: &crate::view::WeaponModel, meshes: &Assets<Mesh>) -> (Quat, f32) {
    let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
    for (mesh, _, t) in &model.parts {
        let Some(positions) = meshes.get(mesh).and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION)).and_then(|a| a.as_float3()) else { continue };
        for p in positions {
            let p = t.transform_point(Vec3::from(*p));
            (lo, hi) = (lo.min(p), hi.max(p));
        }
    }
    if lo.x > hi.x {
        return (Quat::IDENTITY, 0.02);
    }
    let size = hi - lo;
    let mut axes = [(size.x, Vec3::X), (size.y, Vec3::Y), (size.z, Vec3::Z)];
    axes.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (long, mid, thin) = (axes[0].1, axes[1].1, axes[2].1);
    // Model axes → world: long → X, thin → Y (up), mid → Z; keep it a proper rotation.
    let m = Mat3::from_cols(long, thin, if long.cross(thin).dot(mid) > 0.0 { mid } else { -mid });
    (Quat::from_mat3(&m.transpose()), axes[2].0 * 0.5 + 0.01)
}
