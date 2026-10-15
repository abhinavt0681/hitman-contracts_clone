//! Mission logic for C02-1: actors, perception, combat, disguises, objectives.
//!
//! Everything named here comes from the level data: objective and message text from `.LOC`,
//! item and objective locations from GMS entities (`TNT_*`, `Bomb_places`, `Exitbox`), outfits
//! from the `_HMas`/`_NUDE` character templates. What the original scripts (`.sdl`, not
//! decoded) decide is approximated: who is suspicious of what, and how guards fight.

use crate::{
    character::{self, AnimLibrary, Animator, CharacterAssets, Gait},
    collision::WorldCollision,
    player::{OrbitCamera, Player},
};
use bevy::{
    input::mouse::MouseButton,
    mesh::skinning::SkinnedMeshInverseBindposes,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use hitman_clone::glacier::{loc::Strings, skin::SkinnedModel, tex::TexArchive};
use std::collections::HashMap;

/// Guards notice 47 within this distance (m); `RelaxedVisRange=3000` in their properties.
const VIEW_RANGE: f32 = 30.0;
/// Half-angle of a guard's vision cone.
const VIEW_HALF_ANGLE: f32 = 1.05;
const BODY_SPOT_RANGE: f32 = 18.0;
const GUNSHOT_HEARING: f32 = 30.0;
const ALARM_SPREAD: f32 = 35.0;
const RIFLE_RANGE: f32 = 22.0;
const RIFLE_DPS: f32 = 14.0;
const GUARD_RUN_SPEED: f32 = 4.0;
const REACH: f32 = 1.6;
const EYE: f32 = 1.6;
const RADIATION_DPS: f32 = 6.0;
#[allow(dead_code)]
const BOMB_FUSE: f32 = 6.0;

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, debug_setup.run_if(resource_exists::<Mission>));
        app.add_systems(
            Update,
            (
                actor_patrol,
                perceive,
                guard_combat,
                player_actions,
                shoot,
            )
                .chain()
                .run_if(resource_exists::<Mission>),
        );
        app.add_systems(
            Update,
            (
                update_doors,
                soup_poison_timer,
                bjarkhov_meeting,
                update_dragged_body,
                hazards,
                bomb_timer,
                messages_tick,
                restart,
            )
                .run_if(resource_exists::<Mission>),
        );
    }
}

#[derive(Component, Clone, Debug)]
#[allow(dead_code)]
pub struct DoorLeaf {
    pub source_id: u32,
    pub closed_rotation: Quat,
    pub open_rotation: Quat,
}

#[derive(Component, Clone, Debug)]
#[allow(dead_code)]
pub struct Door {
    pub name: String,
    pub interaction_point: Vec3,
    pub is_open: bool,
    pub progress: f32,
    pub target: f32,
    pub auto_close_timer: f32,
    pub leaves: Vec<Entity>,
    pub collision_sources: Vec<u32>,
}

pub fn update_doors(
    mut commands: Commands,
    time: Res<Time>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut collision: ResMut<WorldCollision>,
    mut doors: Query<&mut Door>,
    mut leaves: Query<(&DoorLeaf, &mut Transform)>,
    actors: Query<(&GlobalTransform, &Actor)>,
    player: Query<&GlobalTransform, With<Player>>,
) {
    let dt = time.delta_secs();
    let player_pos = player.iter().next().map(|gt| gt.translation());

    for mut door in &mut doors {
        // NPCs approaching closed doors automatically open them
        if !door.is_open {
            let door_pos = door.interaction_point;
            for (gt, actor) in &actors {
                if actor.alive && gt.translation().distance(door_pos) < 2.2 {
                    door.is_open = true;
                    door.target = 1.0;
                    door.auto_close_timer = 5.0;
                    if let Some(sb) = &sound_bank {
                        if let Some(ppos) = player_pos {
                            let dist = ppos.distance(door_pos);
                            const MAX_DOOR_HEAR_DIST: f32 = 18.0;
                            if dist < MAX_DOOR_HEAR_DIST {
                                let falloff = (1.0 - dist / MAX_DOOR_HEAR_DIST).powi(2);
                                let vol = (0.70 * falloff).clamp(0.04, 0.70);
                                let is_metal = door.name.contains("Metal") || door.name.contains("DDoor") || door.name.contains("Fence");
                                let snd = if is_metal { &sb.door_metal_open } else { &sb.door_wood_open };
                                crate::audio::play_sound(&mut commands, snd, vol);
                            }
                        }
                    }
                    break;
                }
            }
        }

        // Auto close timer
        if door.is_open && door.auto_close_timer > 0.0 {
            door.auto_close_timer -= dt;
            if door.auto_close_timer <= 0.0 {
                door.target = 0.0;
                door.is_open = false;
                if let Some(sb) = &sound_bank {
                    if let Some(ppos) = player_pos {
                        let door_pos = door.interaction_point;
                        let dist = ppos.distance(door_pos);
                        const MAX_DOOR_HEAR_DIST: f32 = 18.0;
                        if dist < MAX_DOOR_HEAR_DIST {
                            let falloff = (1.0 - dist / MAX_DOOR_HEAR_DIST).powi(2);
                            let vol = (0.70 * falloff).clamp(0.04, 0.70);
                            let is_metal = door.name.contains("Metal") || door.name.contains("DDoor") || door.name.contains("Fence");
                            let snd = if is_metal { &sb.door_metal_close } else { &sb.door_wood_close };
                            crate::audio::play_sound(&mut commands, snd, vol);
                        }
                    }
                }
            }
        }

        // Animate door progress towards target
        if (door.progress - door.target).abs() > 1e-4 {
            let dir = (door.target - door.progress).signum();
            door.progress = (door.progress + dir * 2.5 * dt).clamp(0.0, 1.0);

            if door.progress > 0.05 {
                for &src in &door.collision_sources {
                    collision.disable_source(src);
                }
            } else {
                for &src in &door.collision_sources {
                    collision.enable_source(src);
                }
            }

            let t = door.progress * door.progress * (3.0 - 2.0 * door.progress);
            for &leaf_entity in &door.leaves {
                if let Ok((leaf, mut xform)) = leaves.get_mut(leaf_entity) {
                    xform.rotation = leaf.closed_rotation.slerp(leaf.open_rotation, t);
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Guard,
    Civilian,
    Target,
}

/// What someone is wearing. 47 starts in his suit and can take any other.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Outfit {
    Suit,
    Soldier,
    Bodyguard,
    BombWorker,
    Civilian,
    Fuchs,
    Bjarkhov,
}

impl Outfit {
    pub fn from_actor(name: &str) -> Self {
        match name {
            n if n.contains("Bjarkhov") => Outfit::Bjarkhov,
            n if n.contains("FabianFuchs") => Outfit::Fuchs,
            n if n.contains("Bodyguard") => Outfit::Bodyguard,
            n if n.contains("BombWorker") => Outfit::BombWorker,
            n if n.contains("Soldier") || n.contains("Patrol") || n.contains("Guard") => Outfit::Soldier,
            _ => Outfit::Civilian,
        }
    }

    /// Template actor whose model 47 wears in this outfit, and the stripped body left behind.
    fn templates(self) -> (Option<&'static str>, Option<&'static str>) {
        match self {
            Outfit::Soldier => (Some("RussianSoldierArctic_01_HMas"), Some("RussianSoldierArctic_01_NUDE")),
            Outfit::Bodyguard => (Some("FuchsBodyguard_01_HMas"), None),
            Outfit::BombWorker => (Some("BombWorker_01_HMas"), Some("CivilianRussian_01_NUDE")),
            Outfit::Civilian => (Some("CivilianRussian_01_HMas"), Some("CivilianRussian_01_NUDE")),
            Outfit::Fuchs => (Some("FabianFuchs_01_HMas"), Some("FabianFuchs_01_NUDE")),
            Outfit::Suit | Outfit::Bjarkhov => (None, None),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Outfit::Suit => "Suit",
            Outfit::Soldier => "Russian soldier",
            Outfit::Bodyguard => "Fuchs's bodyguard",
            Outfit::BombWorker => "Bomb worker (radiation suit)",
            Outfit::Civilian => "Civilian",
            Outfit::Fuchs => "Fabian Fuchs",
            Outfit::Bjarkhov => "Commander Bjarkhov",
        }
    }
}

#[derive(Component)]
pub struct Actor {
    pub name: String,
    pub kind: Kind,
    pub outfit: Outfit,
    pub alive: bool,
    /// Outfit already taken by 47.
    pub stripped: bool,
    /// Body already reported, so it doesn't re-trigger alarms.
    pub found: bool,
    pub suspicion: f32,
    pub alerted: bool,
    fire_cooldown: f32,
}

impl Actor {
    pub fn new(name: &str) -> Self {
        let outfit = Outfit::from_actor(name);
        let kind = match outfit {
            Outfit::Fuchs | Outfit::Bjarkhov => Kind::Target,
            Outfit::Soldier | Outfit::Bodyguard => Kind::Guard,
            _ => Kind::Civilian,
        };
        Self { name: name.to_owned(), kind, outfit, alive: true, stripped: false, found: false, suspicion: 0.0, alerted: false, fire_cooldown: 0.0 }
    }
}

#[derive(Component, Clone, Default, Debug)]
pub struct Patrol {
    pub waypoints: Vec<Vec3>,
    pub current: usize,
    pub wait_timer: f32,
    pub forward: bool,
}

/// Character models by actor name: the `_HMas`/`_NUDE` templates (outfits and stripped
/// bodies) and every placed actor's own model.
#[derive(Resource)]
pub struct Wardrobe(pub HashMap<String, SkinnedModel>);

/// Head parts sit entirely above this height (cm) in the bind pose.
const HEAD_MIN_HEIGHT: f32 = 140.0;

#[derive(Resource)]
pub struct Textures(pub TexArchive);

#[derive(Resource)]
pub struct Text(pub Strings);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Item {
    #[default]
    Holstered,
    FiberWire,
    SilencedPistol,
    PoisonVial,
    BombRemote,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Holstered => "Holstered",
            Item::FiberWire => "Fiber Wire",
            Item::SilencedPistol => "Silenced Silverballer",
            Item::PoisonVial => "Cyanide Poison",
            Item::BombRemote => "Bomb Remote Detonator",
        }
    }
}

/// World positions (m) of mission locations, from GMS entities.
#[derive(Resource)]
pub struct Anchors {
    pub tnt: Vec<(Vec3, bool)>,
    pub bomb_sites: Vec<(Vec3, bool)>,
    pub soup_pot: Vec3,
    pub radiation_suit: Vec3,
    pub exit: Vec3,
    /// The submarine's irradiated interior: centre and radius.
    pub sub: (Vec3, f32),
    pub bjarkhov_office: Vec3,
    pub bjarkhov_desk: Vec3,
    pub ship_gangway: Vec3,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    Failed,
}

pub struct Objective {
    pub text: String,
    pub done: bool,
}

#[derive(Resource)]
pub struct Mission {
    pub objectives: Vec<Objective>,
    pub health: f32,
    pub outfit: Outfit,
    pub active_item: Item,
    pub tnt: u32,
    pub sub_bombs_placed: u32,
    pub soup_poisoned: bool,
    pub soup_timer: Option<f32>,
    pub bomb: Option<f32>,
    pub alarm: bool,
    pub collateral: u32,
    pub messages: Vec<(String, f32)>,
    pub prompt: Option<String>,
    pub outcome: Option<Outcome>,
    /// Seconds 47 is busy with an action and can't move.
    pub busy: f32,
    pub elapsed: f32,
    pub dragging_body: Option<Entity>,
    pub drag_timer: f32,
    pub bjarkhov_meeting_state: u8,
    pub bjarkhov_vodka_poisoned: bool,
}

impl Mission {
    pub fn new(text: &Strings) -> Self {
        let objectives = (1..=4)
            .map(|i| Objective { text: text.get(&format!("Objective{i}")).unwrap_or("?").to_owned(), done: false })
            .collect();
        Self {
            objectives,
            health: 100.0,
            outfit: Outfit::Suit,
            active_item: Item::Holstered,
            tnt: 0,
            sub_bombs_placed: 0,
            soup_poisoned: false,
            soup_timer: None,
            bomb: None,
            alarm: false,
            collateral: 0,
            messages: Vec::new(),
            prompt: None,
            outcome: None,
            busy: 0.0,
            elapsed: 0.0,
            dragging_body: None,
            drag_timer: 0.0,
            bjarkhov_meeting_state: 0,
            bjarkhov_vodka_poisoned: false,
        }
    }

    pub fn say(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        if !self.messages.iter().any(|(m, _)| *m == msg) {
            self.messages.push((msg, 4.0));
        }
    }

    pub fn rating(&self) -> &'static str {
        match (self.alarm, self.collateral) {
            (false, 0) => "Silent Assassin",
            (false, _) | (true, 0..=2) => "Professional",
            (true, 3..=9) => "Hitman",
            _ => "Mass Murderer",
        }
    }
}

/// World-space eye position and forward direction of an NPC root (P-space forward is −X).
fn npc_view(t: &GlobalTransform) -> (Vec3, Vec3) {
    let (_, rot, pos) = t.to_scale_rotation_translation();
    let fwd = (rot * Vec3::NEG_X).with_y(0.0).normalize_or_zero();
    (pos + Vec3::Y * EYE, fwd)
}

/// Rotation that makes a character root face `dir` (horizontal).
pub fn facing(dir: Vec3) -> Quat {
    Quat::from_rotation_y(dir.z.atan2(-dir.x))
}

fn line_of_sight(world: &WorldCollision, from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    world.raycast(from, d, d.length()).is_none_or(|h| h.distance > d.length() - 0.3)
}

fn raise_alarm(
    commands: Option<&mut Commands>,
    sound_bank: Option<&crate::audio::SoundBank>,
    mission: &mut Mission,
    text: &Text,
    why: Option<&str>,
) {
    if !mission.alarm {
        mission.alarm = true;
        mission.say(text.0.get("NearByGuardsAlarmed").unwrap_or("Nearby guards alarmed"));
        if let (Some(cmds), Some(sb)) = (commands, sound_bank) {
            crate::audio::play_sound(cmds, &sb.base_alarm, 0.65);
            crate::audio::play_sound(cmds, &sb.osd_warning, 0.7);
        }
    }
    if let Some(w) = why {
        mission.say(w.to_owned());
    }
}

/// Who is suspicious of 47, how fast, given his outfit and behaviour.
fn suspicion_rate(watcher: &Actor, outfit: Outfit, dist: f32, running: bool, drawn_weapon: bool) -> f32 {
    if watcher.kind != Kind::Guard {
        return 0.0;
    }
    // Drawing a weapon in sight of a guard immediately triggers intense suspicion
    if drawn_weapon && dist < VIEW_RANGE {
        return 1.4;
    }
    let close = (1.0 - dist / VIEW_RANGE).clamp(0.0, 1.0);
    let run = if running { 2.0 } else { 1.0 };
    match (watcher.outfit, outfit) {
        // A stranger in a suit at a military base.
        (Outfit::Soldier, Outfit::Suit | Outfit::Civilian) => 0.25 + 0.9 * close * run,
        // Fuchs's men are busy with the deal; they only mind a stranger right next to them.
        (Outfit::Bodyguard, Outfit::Suit | Outfit::Civilian) if dist < 6.0 => 0.6 * run,
        // Same uniform: only odd behaviour up close gives 47 away.
        (a, b) if a == b => if dist < 2.5 { 0.12 } else if running && dist < 8.0 { 0.35 } else { 0.0 },
        (_, Outfit::Fuchs) if watcher.outfit == Outfit::Bodyguard && dist < 3.0 => 0.5,
        _ if running && dist < 6.0 => 0.2,
        _ => 0.0,
    }
}

#[allow(clippy::too_many_arguments)]
fn perceive(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<WorldCollision>,
    text: Res<Text>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut mission: ResMut<Mission>,
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor)>,
) {
    if mission.outcome.is_some() {
        return;
    }
    let dt = time.delta_secs();
    mission.elapsed += dt;
    let Ok((pt, player)) = player.single() else { return };
    let target = if player.crouching {
        pt.translation() + Vec3::Y * 0.75
    } else {
        pt.translation() + Vec3::Y * 1.2
    };
    let running = player.speed() > 2.5;
    let drawn_weapon = mission.active_item != Item::Holstered && mission.active_item != Item::PoisonVial;
    let is_dragging = mission.dragging_body.is_some();

    let bodies: Vec<(Entity, Vec3, String)> = actors
        .iter()
        .filter(|(e, _, a)| !a.alive && !a.found && Some(*e) != mission.dragging_body)
        .map(|(e, t, a)| (e, t.translation(), a.name.clone()))
        .collect();
    let mut found_bodies = Vec::new();
    let mut new_alarm = false;
    let outfit = mission.outfit;

    for (_, t, mut a) in &mut actors {
        if !a.alive || a.kind != Kind::Guard {
            continue;
        }
        let (eye, fwd) = npc_view(t);
        // Bodies in view.
        for (body, pos, name) in &bodies {
            let to = *pos + Vec3::Y * 0.4 - eye;
            if to.length() < BODY_SPOT_RANGE && fwd.angle_between(to.with_y(0.0)) < VIEW_HALF_ANGLE && line_of_sight(&world, eye, *pos + Vec3::Y * 0.4) {
                found_bodies.push((*body, name.clone()));
                a.alerted = true;
            }
        }
        // 47 in view.
        let to = target - eye;
        let dist = to.length();
        let in_cone = fwd.angle_between(to.with_y(0.0)) < VIEW_HALF_ANGLE;
        // Sneaking makes footsteps completely silent; guards cannot detect someone behind them unless physically bumped (<0.5m)
        let heard_behind = if player.crouching {
            dist < 0.5
        } else if running {
            dist < 10.0
        } else if is_dragging {
            dist < 3.2
        } else {
            dist < 1.8
        };
        let visible = dist < VIEW_RANGE && (in_cone || heard_behind) && line_of_sight(&world, eye, target);
        let mut rate = if visible {
            if is_dragging {
                1.5 // dragging a corpse right in front of a guard triggers instant alarm!
            } else {
                // Guards only notice drawn weapons if 47 is in their vision cone (can't see behind their back)
                suspicion_rate(&a, outfit, dist, running, drawn_weapon && in_cone)
            }
        } else {
            0.0
        };
        if player.crouching {
            rate *= 0.35; // sneaking reduces suspicion accumulation rate in view
        }
        a.suspicion = if rate > 0.0 { a.suspicion + rate * dt } else { (a.suspicion - 0.15 * dt).max(0.0) };
        if a.suspicion >= 1.0 && !a.alerted {
            a.alerted = true;
            new_alarm = true;
        }
    }

    let sb_ref = sound_bank.as_deref();
    for (body, name) in found_bodies {
        if let Ok((_, _, mut a)) = actors.get_mut(body) {
            a.found = true;
        }
        let msg = text.0.get(&name).or_else(|| text.0.get(name.split('_').next().unwrap_or(""))).map(str::to_owned);
        raise_alarm(Some(&mut commands), sb_ref, &mut mission, &text, msg.as_deref());
        new_alarm = true;
    }
    if new_alarm {
        raise_alarm(Some(&mut commands), sb_ref, &mut mission, &text, None);
        // Word spreads to guards nearby.
        let alerted: Vec<Vec3> = actors.iter().filter(|(_, _, a)| a.alerted && a.alive).map(|(_, t, _)| t.translation()).collect();
        for (_, t, mut a) in &mut actors {
            if a.alive && a.kind == Kind::Guard && alerted.iter().any(|p| p.distance(t.translation()) < ALARM_SPREAD) {
                a.alerted = true;
            }
        }
    }
}

fn guard_combat(
    time: Res<Time>,
    world: Res<WorldCollision>,
    mut mission: ResMut<Mission>,
    player: Query<&GlobalTransform, With<Player>>,
    mut actors: Query<(&mut Transform, &GlobalTransform, &mut Actor, Option<&mut Gait>)>,
) {
    if mission.outcome.is_some() {
        return;
    }
    let dt = time.delta_secs();
    let Ok(pt) = player.single() else { return };
    let target = pt.translation();
    let mut damage = 0.0;
    for (mut t, gt, mut a, gait) in &mut actors {
        if !a.alive || !a.alerted || a.kind != Kind::Guard {
            continue;
        }
        let Some(mut gait) = gait else { continue };
        let to = (target - t.translation).with_y(0.0);
        let dist = to.length();
        if dist > 0.1 {
            t.rotation = facing(to / dist);
        }
        let (eye, _) = npc_view(gt);
        let sees = line_of_sight(&world, eye, target + Vec3::Y * 1.2);
        if dist > RIFLE_RANGE * 0.7 || !sees {
            // Close in, sliding along walls like 47 does.
            let mut pos = t.translation + to.normalize_or_zero() * GUARD_RUN_SPEED * dt;
            pos += world.push_out_horizontal(pos + Vec3::Y * 1.0, 0.3);
            if let Some(h) = world.raycast(pos + Vec3::Y * 0.8, Vec3::NEG_Y, 2.0) {
                pos.y = pos.y + 0.8 - h.distance;
            }
            t.translation = pos;
            gait.speed = GUARD_RUN_SPEED;
        } else {
            gait.speed = 0.0;
            gait.idle = "Shoot_Pose_Rifle01".to_owned();
            a.fire_cooldown -= dt;
            if a.fire_cooldown <= 0.0 {
                a.fire_cooldown = 0.25;
            }
            // Accuracy falls off with range.
            damage += RIFLE_DPS * (1.0 - dist / RIFLE_RANGE).clamp(0.2, 1.0) * dt;
        }
    }
    mission.health -= damage;
}

fn actor_patrol(
    time: Res<Time>,
    world: Res<WorldCollision>,
    mission: Option<Res<Mission>>,
    mut actors: Query<(&mut Transform, &Actor, Option<&mut Gait>, &mut Patrol)>,
) {
    if mission.is_some_and(|m| m.outcome.is_some()) {
        return;
    }
    let dt = time.delta_secs();
    for (mut t, a, gait, mut patrol) in &mut actors {
        if !a.alive || a.alerted || patrol.waypoints.is_empty() {
            continue;
        }
        let Some(mut gait) = gait else { continue };

        if patrol.wait_timer > 0.0 {
            patrol.wait_timer -= dt;
            gait.speed = 0.0;
            continue;
        }

        let target = patrol.waypoints[patrol.current];
        let to = (target - t.translation).with_y(0.0);
        let dist = to.length();

        if dist < 0.8 {
            if patrol.forward {
                if patrol.current + 1 < patrol.waypoints.len() {
                    patrol.current += 1;
                } else {
                    patrol.forward = false;
                    patrol.wait_timer = 3.0;
                }
            } else if patrol.current > 0 {
                patrol.current -= 1;
            } else {
                patrol.forward = true;
                patrol.wait_timer = 3.0;
            }
            continue;
        }

        let dir = to / dist;
        t.rotation = t.rotation.slerp(facing(dir), (4.0 * dt).min(1.0));
        let walk_speed = 1.3;
        let mut pos = t.translation + dir * walk_speed * dt;
        pos += world.push_out_horizontal(pos + Vec3::Y * 1.0, 0.3);
        if let Some(h) = world.raycast(pos + Vec3::Y * 0.8, Vec3::NEG_Y, 2.0) {
            pos.y = pos.y + 0.8 - h.distance;
        }
        t.translation = pos;
        gait.speed = walk_speed;
    }
}

#[allow(clippy::too_many_arguments)]
fn player_actions(
    mut commands: Commands,
    inputs: (Res<ButtonInput<KeyCode>>, Option<Res<bevy::input::mouse::AccumulatedMouseScroll>>),
    time: Res<Time>,
    world: Res<WorldCollision>,
    text: Res<Text>,
    lib: Res<AnimLibrary>,
    wardrobe: Res<Wardrobe>,
    textures: Res<Textures>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut anchors: ResMut<Anchors>,
    mut mission: ResMut<Mission>,
    mut assets: (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>, ResMut<Assets<SkinnedMeshInverseBindposes>>),
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
    mut player_anim: Query<&mut Animator, Without<Actor>>,
    mut doors: Query<&mut Door>,
) {
    let (keys, mouse_scroll) = (&inputs.0, &inputs.1);
    // Weapon / Item switching with keys 1..=5 and mouse wheel
    let items = [
        Item::Holstered,
        Item::FiberWire,
        Item::SilencedPistol,
        Item::PoisonVial,
        Item::BombRemote,
    ];
    let mut switch = if keys.just_pressed(KeyCode::Digit1) {
        Some(Item::Holstered)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(Item::FiberWire)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(Item::SilencedPistol)
    } else if keys.just_pressed(KeyCode::Digit4) {
        Some(Item::PoisonVial)
    } else if keys.just_pressed(KeyCode::Digit5) {
        Some(Item::BombRemote)
    } else {
        None
    };

    if switch.is_none() {
        if let Some(scroll) = &mouse_scroll {
            if scroll.delta.y.abs() > 0.1 {
                let current_idx = items.iter().position(|&i| i == mission.active_item).unwrap_or(0);
                let next_idx = if scroll.delta.y > 0.0 {
                    (current_idx + items.len() - 1) % items.len()
                } else {
                    (current_idx + 1) % items.len()
                };
                switch = Some(items[next_idx]);
            }
        }
    }

    if let Some(item) = switch {
        if item != mission.active_item {
            mission.active_item = item;
            let label = item.label();
            mission.say(format!("Equipped: {label}"));
            if let Some(sb) = &sound_bank {
                if item == Item::Holstered {
                    crate::audio::play_sound(&mut commands, &sb.gun_holster, 0.5);
                } else {
                    crate::audio::play_sound(&mut commands, &sb.gun_pull, 0.5);
                }
            }
        }
    }

    mission.busy = (mission.busy - time.delta_secs()).max(0.0);
    mission.prompt = None;
    if mission.outcome.is_some() || mission.busy > 0.0 {
        return;
    }
    let Ok((pt, player)) = player.single() else { return };
    let me = pt.translation();
    let use_key = keys.just_pressed(KeyCode::KeyE);

    // Drop currently dragged body
    if mission.dragging_body.is_some() {
        mission.prompt = Some("[G] Drop body".to_owned());
        if keys.just_pressed(KeyCode::KeyG) {
            mission.dragging_body = None;
            mission.say("Body dropped");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.body_drop, 0.6);
            }
            return;
        }
    }

    // Closest actor in reach.
    let near = actors
        .iter()
        .map(|(e, t, a, _)| (e, t.translation().distance(me), t, a))
        .filter(|(_, d, _, _)| *d < REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _, t, a)| (e, *t, a.alive, a.stripped, a.outfit, a.kind, a.name.clone()));

    if let Some((e, t, alive, stripped, outfit, kind, name)) = near {
        let (_, fwd) = npc_view(&t);
        let behind = fwd.dot((me - t.translation()).with_y(0.0).normalize_or_zero()) < -0.2;
        if alive && behind {
            let is_poison = mission.active_item == Item::PoisonVial;
            if is_poison {
                mission.prompt = Some("[E] Inject Cyanide Poison".to_owned());
            } else {
                mission.prompt = Some("[E] Strangle with Fiber Wire".to_owned());
            }
            if use_key {
                if let Some(sb) = &sound_bank {
                    if is_poison {
                        crate::audio::play_sound(&mut commands, &sb.clothes_touch, 0.6);
                    } else {
                        crate::audio::play_sound(&mut commands, &sb.fiber_wire, 0.7);
                    }
                }
                if let Ok((_, gt, mut a, mut anim)) = actors.get_mut(e) {
                    a.alive = false;
                    let clip = lib.first_of(anim.set(), &["Die_Throat_Cut_Alt_01", "Die_Forward"]).unwrap_or_default();
                    anim.play_once_at(&clip, 2.5);
                    commands.entity(e).remove::<Gait>();
                    // Anyone watching sees the kill.
                    let spot = gt.translation();
                    if actors.iter().any(|(o, ot, oa, _)| {
                        o != e && oa.alive && oa.kind == Kind::Guard && {
                            let (eye, f) = npc_view(ot);
                            let to = spot + Vec3::Y - eye;
                            to.length() < 20.0 && f.angle_between(to.with_y(0.0)) < VIEW_HALF_ANGLE && line_of_sight(&world, eye, spot + Vec3::Y)
                        }
                    }) {
                        raise_alarm(Some(&mut commands), sound_bank.as_deref(), &mut mission, &text, None);
                    }
                }
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Cut_Throat");
                }
                mission.busy = 1.6;
                on_kill(&mut mission, &text, kind, &name);
            }
            return;
        }

        if !alive && mission.dragging_body.is_none() {
            let can_take = !stripped && outfit.templates().0.is_some_and(|t| wardrobe.0.contains_key(t)) && outfit != mission.outfit;
            if can_take {
                mission.prompt = Some(format!("[E] Take outfit: {}   |   [G] Drag body", outfit.label()));
            } else {
                mission.prompt = Some("[G] Drag body".to_owned());
            }

            if keys.just_pressed(KeyCode::KeyG) {
                mission.dragging_body = Some(e);
                mission.say("Dragging body (press G to drop)");
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.body_drag, 0.6);
                }
                return;
            }

            if can_take && use_key {
                let (wear, nude) = outfit.templates();
                let mut cast = CharacterAssets {
                    meshes: &mut assets.0,
                    materials: &mut assets.1,
                    images: &mut assets.2,
                    bindposes: &mut assets.3,
                    textures: &textures.0,
                };
                if let Some(model) = wear.and_then(|w| wardrobe.0.get(w)) {
                    let root = player.model();
                    commands.entity(root).despawn_children().remove::<character::Rig>();
                    character::attach(&mut commands, root, model, &mut cast);
                }
                if let Some(body) = nude.and_then(|n| wardrobe.0.get(n)) {
                    // Stripped bodies have no head of their own: keep the victim's.
                    let model = match wardrobe.0.get(&name) {
                        Some(own) => body.with_parts_from(own, |p| p.min_height() > HEAD_MIN_HEIGHT),
                        None => body.clone(),
                    };
                    commands.entity(e).despawn_children().remove::<character::Rig>();
                    character::attach(&mut commands, e, &model, &mut cast);
                }
                if let Ok((_, _, mut a, _)) = actors.get_mut(e) {
                    a.stripped = true;
                }
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Change_Clothes");
                }
                mission.busy = 2.0;
                mission.outfit = outfit;
                mission.say(format!("Disguised as {}", outfit.label()));
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.clothes_touch, 0.7);
                    crate::audio::play_sound(&mut commands, &sb.osd_hint, 0.6);
                }
                return;
            }
        }
    }

    // Door interaction
    let mut closest_door: Option<(Mut<Door>, f32)> = None;
    for door in &mut doors {
        let d = door.interaction_point.distance(me);
        if d < 2.5 {
            if closest_door.as_ref().map_or(true, |(_, best)| d < *best) {
                closest_door = Some((door, d));
            }
        }
    }
    if let Some((mut door, _)) = closest_door {
        if mission.prompt.is_none() {
            if !door.is_open {
                mission.prompt = Some("[E] Open door".to_owned());
            } else {
                mission.prompt = Some("[E] Close door".to_owned());
            }
            if use_key {
                door.is_open = !door.is_open;
                door.target = if door.is_open { 1.0 } else { 0.0 };
                if door.is_open {
                    door.auto_close_timer = 7.0;
                }
                if let Some(sb) = &sound_bank {
                    let is_metal = door.name.contains("Metal") || door.name.contains("DDoor") || door.name.contains("Fence");
                    let snd = if door.is_open {
                        if is_metal { &sb.door_metal_open } else { &sb.door_wood_open }
                    } else {
                        if is_metal { &sb.door_metal_close } else { &sb.door_wood_close }
                    };
                    crate::audio::play_sound(&mut commands, snd, 0.65);
                }
                mission.say(if door.is_open { "Door opened" } else { "Door closed" });
                return;
            }
        }
    }

    // Radiation suit pickup in northwest warehouse
    if me.distance(anchors.radiation_suit) < 2.5 && mission.outfit != Outfit::BombWorker {
        mission.prompt = Some("[E] Change into Radiation Suit".to_owned());
        if use_key {
            let (wear, _) = Outfit::BombWorker.templates();
            let mut cast = CharacterAssets {
                meshes: &mut assets.0,
                materials: &mut assets.1,
                images: &mut assets.2,
                bindposes: &mut assets.3,
                textures: &textures.0,
            };
            if let Some(model) = wear.and_then(|w| wardrobe.0.get(w)) {
                let root = player.model();
                commands.entity(root).despawn_children().remove::<character::Rig>();
                character::attach(&mut commands, root, model, &mut cast);
            }
            if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                anim.play_once("Change_Clothes");
            }
            mission.busy = 2.0;
            mission.outfit = Outfit::BombWorker;
            mission.say("Disguised as Bomb Worker (Radiation Suit)");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.clothes_touch, 0.7);
                crate::audio::play_sound(&mut commands, &sb.osd_hint, 0.6);
            }
            return;
        }
    }

    // Soup poisoning in mess hall kitchen
    if me.distance(anchors.soup_pot) < 2.5 && !mission.soup_poisoned {
        if mission.active_item == Item::PoisonVial {
            mission.prompt = Some("[E] Poison soup with cyanide".to_owned());
            if use_key {
                mission.soup_poisoned = true;
                mission.soup_timer = Some(12.0);
                mission.busy = 2.0;
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Pick_Up_Medium");
                }
                mission.say("Poisoned the soup with cyanide");
                return;
            }
        } else {
            mission.prompt = Some("Equip Cyanide Poison [4] to poison soup".to_owned());
        }
    }

    // Bjarkhov office vodka poisoning
    if me.distance(anchors.bjarkhov_desk) < 3.5 && mission.bjarkhov_meeting_state >= 2 && !mission.bjarkhov_vodka_poisoned {
        if mission.active_item == Item::PoisonVial {
            mission.prompt = Some("[E] Poison Bjarkhov's vodka with cyanide".to_owned());
            if use_key {
                mission.bjarkhov_vodka_poisoned = true;
                mission.busy = 1.5;
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Poison_Container");
                }
                mission.say("Poisoned Commander Bjarkhov's vodka with cyanide");
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.gun_pull, 0.5);
                }
                return;
            }
        } else {
            mission.prompt = Some("Equip Cyanide Poison [4] to poison vodka".to_owned());
        }
    }

    // Items and places.
    if let Some(i) = anchors.tnt.iter().position(|(p, taken)| !taken && p.distance(me) < REACH) {
        mission.prompt = Some("[E] Pick up dynamite".to_owned());
        if use_key {
            anchors.tnt[i].1 = true;
            mission.tnt += 1;
            mission.say("Picked up dynamite");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.gun_pull, 0.4);
            }
        }
        return;
    }
    if let Some(i) = anchors.bomb_sites.iter().position(|(p, placed)| !placed && p.distance(me) < 3.0) {
        if mission.tnt == 0 {
            mission.prompt = Some("You need dynamite (there is some on board)".to_owned());
        } else {
            mission.prompt = Some(format!("[E] {}", text.0.get("PlaceBomb").unwrap_or("Place bomb")));
            if use_key {
                mission.tnt -= 1;
                anchors.bomb_sites[i].1 = true;
                mission.sub_bombs_placed += 1;
                mission.busy = 1.5;
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Place_Bomb_Under");
                }
                match mission.sub_bombs_placed {
                    1 => mission.say(text.0.get("1SubBombPlaced").unwrap_or("1st of 3 bombs has been placed on submarine.")),
                    2 => mission.say(text.0.get("2SubBombPlaced").unwrap_or("2nd of 3 bombs placed on submarine.")),
                    _ => {
                        mission.say(text.0.get("AllSubBombsPlaced").unwrap_or("All bombs placed on submarine."));
                        mission.say("Equip Bomb Remote [5] and detonate from safe distance");
                    }
                }
            }
        }
        return;
    }
    let exit_to = anchors.exit - me;
    if exit_to.with_y(0.0).length() < 4.5 && exit_to.y.abs() < 4.0 {
        if mission.objectives[..3].iter().all(|o| o.done) {
            mission.prompt = Some("[E] Board the cargo plane".to_owned());
            if use_key {
                mission.objectives[3].done = true;
                mission.outcome = Some(Outcome::Complete);
            }
        } else {
            mission.prompt = Some(text.0.get("ObjectiveNotComplete").unwrap_or("Objectives not completed").to_owned());
        }
    }
}

fn bjarkhov_meeting(
    mut commands: Commands,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    anchors: Res<Anchors>,
    text: Res<Text>,
    mut mission: ResMut<Mission>,
    player: Query<&GlobalTransform, With<Player>>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
) {
    if mission.outcome.is_some() {
        return;
    }
    let Ok(pt) = player.single() else { return };
    let me = pt.translation();

    // Gangway greeting if disguised as Fuchs
    if mission.outfit == Outfit::Fuchs && mission.bjarkhov_meeting_state == 0 {
        if me.distance(anchors.ship_gangway) < 7.0 {
            mission.bjarkhov_meeting_state = 1;
            mission.say("Icebreaker Guard: 'Welcome aboard Mr. Fuchs. Commander Bjarkhov is waiting for you in his office.'");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.osd_hint, 0.6);
            }
        }
    }

    // Entering Bjarkhov's office
    if me.distance(anchors.bjarkhov_office) < 5.0 && mission.bjarkhov_meeting_state < 2 {
        if mission.outfit == Outfit::Fuchs {
            mission.bjarkhov_meeting_state = 2;
            mission.say("Commander Bjarkhov: 'Ah, Fabian! Good to see you... Come in, let us discuss business over some vodka.'");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.bjarkhov_welcome, 0.9);
            }
            for (_, _, a, mut anim) in &mut actors {
                if a.name.contains("Bjarkhov") && a.alive {
                    anim.play("Bjarkov_Turn_toPour", 1.0);
                }
            }
        }
    }

    // Bjarkhov drinks poisoned vodka
    if mission.bjarkhov_vodka_poisoned && mission.bjarkhov_meeting_state == 2 {
        mission.bjarkhov_meeting_state = 3;
        for (e, _, mut a, mut anim) in &mut actors {
            if a.name.contains("Bjarkhov") && a.alive {
                a.alive = false;
                anim.play_once("Die_Throat_Cut_Alt_01");
                commands.entity(e).remove::<Gait>();
                let name = a.name.clone();
                let kind = a.kind;
                on_kill(&mut mission, &text, kind, &name);
                mission.say("Commander Bjarkhov succumbed to cyanide poisoning");
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.bjarkhov_drink_vodka, 0.85);
                }
                break;
            }
        }
    }
}

fn update_dragged_body(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<WorldCollision>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut mission: ResMut<Mission>,
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(&mut Transform, &Actor)>,
) {
    let Some(drag_entity) = mission.dragging_body else { return };
    let dt = time.delta_secs();
    let Ok((pt, player)) = player.single() else { return };
    let fwd = (pt.rotation() * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
    let drag_pos = pt.translation() - fwd * 0.95;

    if let Ok((mut body_t, _)) = actors.get_mut(drag_entity) {
        let mut target = drag_pos;
        if let Some(h) = world.raycast(target + Vec3::Y * 0.8, Vec3::NEG_Y, 2.0) {
            target.y = target.y + 0.8 - h.distance;
        }
        body_t.translation = target;
        body_t.rotation = pt.rotation();
    }

    if player.speed() > 0.2 {
        mission.drag_timer += dt;
        if mission.drag_timer >= 1.1 {
            mission.drag_timer = 0.0;
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.body_drag, 0.45);
            }
        }
    }
}

fn on_kill(mission: &mut Mission, text: &Text, kind: Kind, name: &str) {
    let objective = match name {
        n if n.contains("FabianFuchs") => Some(0),
        n if n.contains("Bjarkhov") => Some(1),
        _ => None,
    };
    match objective {
        Some(i) if !mission.objectives[i].done => {
            mission.objectives[i].done = true;
            mission.say(text.0.get(&format!("ObjOk{}", i + 1)).unwrap_or("Objective completed").to_owned());
            if mission.objectives[..3].iter().all(|o| o.done) {
                mission.say(text.0.get("HeadForExit").unwrap_or("Head for your EXIT point").to_owned());
            }
        }
        _ if kind != Kind::Target => mission.collateral += 1,
        _ => {}
    }
}

/// Silenced pistol or Bomb Remote: left click once the mouse is captured (or [E] for detonator).
#[allow(clippy::too_many_arguments)]
fn shoot(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    world: Res<WorldCollision>,
    text: Res<Text>,
    lib: Res<AnimLibrary>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    anchors: Res<Anchors>,
    mut mission: ResMut<Mission>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    camera: Query<&GlobalTransform, With<OrbitCamera>>,
    player: Query<&GlobalTransform, With<Player>>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
) {
    let triggered = mouse.just_pressed(MouseButton::Left)
        || (mission.active_item == Item::BombRemote && keys.just_pressed(KeyCode::KeyE));
    if mission.outcome.is_some() || mission.busy > 0.0 || !triggered {
        return;
    }
    if cursor.single().is_ok_and(|c| c.grab_mode == CursorGrabMode::None) && !keys.just_pressed(KeyCode::KeyE) {
        return; // this click captures the mouse
    }

    if mission.active_item == Item::BombRemote {
        if mission.sub_bombs_placed < 3 {
            mission.say("Need to place all 3 bombs on submarine before detonation");
        } else if !mission.objectives[2].done {
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.sub_explode, 1.0);
            }
            mission.objectives[2].done = true;
            mission.alarm = true;
            mission.say("Submarine destroyed! Objective completed.");
            if let Ok(pt) = player.single() {
                if pt.translation().distance(anchors.sub.0) < 22.0 {
                    mission.health -= 60.0;
                    mission.say("You were caught in the explosion blast!");
                }
            }
            for (e, gt, mut a, mut anim) in &mut actors {
                if a.alive && anchors.bomb_sites.iter().any(|(p, _)| gt.translation().distance(*p) < 20.0) {
                    a.alive = false;
                    let clip = lib.first_of(anim.set(), &["Die_Backward", "Die_Forward"]).unwrap_or_default();
                    anim.play_once(&clip);
                    commands.entity(e).remove::<Gait>();
                }
            }
            if mission.objectives[..3].iter().all(|o| o.done) {
                mission.say(text.0.get("HeadForExit").unwrap_or("Head for your EXIT point").to_owned());
            }
        }
        return;
    }

    if mission.active_item != Item::SilencedPistol {
        return;
    }

    let Ok(cam) = camera.single() else { return };
    let (origin, dir) = (cam.translation(), cam.forward().as_vec3());

    if let Some(sb) = &sound_bank {
        crate::audio::play_sound(&mut commands, &sb.shot_silenced, 0.75);
    }

    let wall = world.raycast(origin, dir, 200.0).map_or(200.0, |h| h.distance);
    // Ray against a capsule-ish target: closest approach to the actor's spine.
    let hit = actors
        .iter()
        .filter(|(_, _, a, _)| a.alive)
        .filter_map(|(e, t, _, _)| {
            let base = t.translation();
            (0..4)
                .filter_map(|k| {
                    let c = base + Vec3::Y * (0.4 + 0.4 * k as f32);
                    let along = (c - origin).dot(dir);
                    (along > 0.0 && along < wall && (origin + dir * along).distance(c) < 0.3).then_some(along)
                })
                .reduce(f32::min)
                .map(|d| (e, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let shot_from = origin;
    if let Some((e, _)) = hit {
        if let Ok((_, _, mut a, mut anim)) = actors.get_mut(e) {
            a.alive = false;
            let clip = lib.first_of(anim.set(), &["Die_Backward", "Die_Forward"]).unwrap_or_default();
            anim.play_once(&clip);
            commands.entity(e).remove::<Gait>();
            let (kind, name) = (a.kind, a.name.clone());
            on_kill(&mut mission, &text, kind, &name);
        }
    }
    // Silenced, but not silent: guards close by hear it.
    let mut heard = false;
    for (_, t, mut a, _) in &mut actors {
        if a.alive && a.kind == Kind::Guard && t.translation().distance(shot_from) < GUNSHOT_HEARING * 0.5 {
            a.alerted = true;
            heard = true;
        }
    }
    if heard {
        raise_alarm(Some(&mut commands), sound_bank.as_deref(), &mut mission, &text, None);
    }
}

fn hazards(time: Res<Time>, anchors: Res<Anchors>, mut mission: ResMut<Mission>, player: Query<&GlobalTransform, With<Player>>) {
    if mission.outcome.is_some() {
        return;
    }
    let Ok(pt) = player.single() else { return };
    let (centre, radius) = anchors.sub;
    if mission.outfit != Outfit::BombWorker && pt.translation().distance(centre) < radius {
        mission.health -= RADIATION_DPS * time.delta_secs();
        mission.say("Radiation! You need a radiation suit to be on the submarine");
    }
    if mission.health <= 0.0 {
        mission.health = 0.0;
        mission.outcome = Some(Outcome::Failed);
    }
}

fn soup_poison_timer(
    mut commands: Commands,
    time: Res<Time>,
    lib: Res<AnimLibrary>,
    text: Res<Text>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut mission: ResMut<Mission>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
) {
    let Some(t) = mission.soup_timer.as_mut() else { return };
    let prev = *t;
    *t -= time.delta_secs();
    if prev > 8.0 && *t <= 8.0 {
        if let Some(sb) = &sound_bank {
            crate::audio::play_sound(&mut commands, &sb.fuchs_want_soup, 0.9);
        }
        mission.say(text.0.get("02-1_FOS_01Fuchs").unwrap_or("Can I have some soup down here?!! Russisches Schwein."));
    } else if prev > 4.0 && *t <= 4.0 {
        mission.say("Fabian Fuchs eats the poisoned soup...");
    } else if *t <= 0.0 {
        mission.soup_timer = None;
        if let Some(sb) = &sound_bank {
            crate::audio::play_sound(&mut commands, &sb.fuchs_poisoned, 0.9);
        }
        mission.say("Fabian Fuchs succumbed to cyanide poison");
        for (e, _, mut a, mut anim) in &mut actors {
            if a.name.contains("FabianFuchs") && a.alive {
                a.alive = false;
                let clip = lib.first_of(anim.set(), &["Die_Forward", "Die_Backward"]).unwrap_or_default();
                anim.play_once(&clip);
                commands.entity(e).remove::<Gait>();
                let (kind, name) = (a.kind, a.name.clone());
                on_kill(&mut mission, &text, kind, &name);
            }
        }
    }
}

fn bomb_timer(
    mut commands: Commands,
    time: Res<Time>,
    lib: Res<AnimLibrary>,
    mut mission: ResMut<Mission>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
    anchors: Res<Anchors>,
) {
    let Some(t) = mission.bomb.as_mut() else { return };
    *t -= time.delta_secs();
    if *t > 0.0 {
        return;
    }
    mission.bomb = None;
    mission.objectives[2].done = true;
    mission.alarm = true;
    mission.say("The submarine lab has been destroyed");
    for (e, gt, mut a, mut anim) in &mut actors {
        if a.alive && anchors.bomb_sites.iter().any(|(p, _)| gt.translation().distance(*p) < 14.0) {
            a.alive = false;
            let clip = lib.first_of(anim.set(), &["Die_Backward", "Die_Forward"]).unwrap_or_default();
            anim.play_once(&clip);
            commands.entity(e).remove::<Gait>();
        }
    }
    if mission.objectives[..3].iter().all(|o| o.done) {
        mission.say("Head for your EXIT point");
    }
}

fn messages_tick(time: Res<Time>, mut mission: ResMut<Mission>) {
    let dt = time.delta_secs();
    for m in &mut mission.messages {
        m.1 -= dt;
    }
    mission.messages.retain(|m| m.1 > 0.0);
}

/// Enter after the mission ends restarts the game process (simplest full reset).
fn restart(keys: Res<ButtonInput<KeyCode>>, mission: Res<Mission>, mut exit: MessageWriter<AppExit>) {
    if mission.outcome.is_none() || !keys.just_pressed(KeyCode::Enter) {
        return;
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(std::env::args().skip(1)).spawn();
    }
    exit.write(AppExit::Success);
}

pub fn outfit_label(o: Outfit) -> &'static str {
    o.label()
}

/// Test hooks: `HITMAN_BEHIND=<actor name>` starts 47 a metre behind that actor;
/// `HITMAN_OUTFIT=Soldier|BombWorker|…` starts him in that outfit (no model swap).
fn debug_setup(
    mut mission: ResMut<Mission>,
    actors: Query<(&Transform, &Actor), Without<Player>>,
    mut player: Query<&mut Transform, With<Player>>,
    mut cam: Query<&mut OrbitCamera>,
) {
    if let Ok(name) = std::env::var("HITMAN_BEHIND") {
        if let (Some((t, _)), Ok(mut p)) = (actors.iter().find(|(_, a)| a.name == name), player.single_mut()) {
            let fwd = (t.rotation * Vec3::NEG_X).with_y(0.0).normalize_or_zero();
            p.translation = t.translation - fwd * 1.0 + Vec3::Y * 0.3;
            p.rotation = Transform::IDENTITY.looking_to(fwd, Vec3::Y).rotation;
            if let Ok(mut c) = cam.single_mut() {
                // `HITMAN_LOOK_OFFSET` (radians) swings the camera round for a side view.
                let off = std::env::var("HITMAN_LOOK_OFFSET").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
                c.look_along(Quat::from_rotation_y(off) * fwd);
            }
        }
    }
    if let Some(n) = std::env::var("HITMAN_TNT").ok().and_then(|v| v.parse().ok()) {
        mission.tnt = n;
    }
    if let Some(n) = std::env::var("HITMAN_BOMBS_PLACED").ok().and_then(|v| v.parse().ok()) {
        mission.sub_bombs_placed = n;
    }
    if let Ok(item) = std::env::var("HITMAN_ITEM") {
        mission.active_item = match item.as_str() {
            "FiberWire" => Item::FiberWire,
            "SilencedPistol" => Item::SilencedPistol,
            "PoisonVial" => Item::PoisonVial,
            "BombRemote" => Item::BombRemote,
            _ => Item::Holstered,
        };
    }
    if let Ok(o) = std::env::var("HITMAN_OUTFIT") {
        mission.outfit = match o.as_str() {
            "Soldier" => Outfit::Soldier,
            "BombWorker" => Outfit::BombWorker,
            "Bodyguard" => Outfit::Bodyguard,
            "Civilian" => Outfit::Civilian,
            _ => Outfit::Suit,
        };
    }
    if std::env::var_os("HITMAN_ALL_OBJECTIVES").is_some() {
        mission.objectives[0].done = true;
        mission.objectives[1].done = true;
        mission.objectives[2].done = true;
    }
}
