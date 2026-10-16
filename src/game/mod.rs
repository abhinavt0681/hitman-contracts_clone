//! Mission logic for C02-1: actors, perception, combat, disguises, objectives.
//!
//! Everything named here comes from the level data: objective and message text from `.LOC`,
//! item and objective locations from GMS entities (`TNT_*`, `Bomb_places`, `Exitbox`), outfits
//! from the `_HMas`/`_NUDE` character templates. What the original scripts (`.sdl`, not
//! decoded) decide is approximated: who is suspicious of what, and how guards fight.
use crate::{
    collision::WorldCollision,
    player::{OrbitCamera, Player},
};
use bevy::prelude::*;
use hitman_clone::glacier::{loc::Strings, skin::SkinnedModel, tex::TexArchive};
use std::collections::HashMap;

mod actions;
mod ai;
mod doors;
mod mission;
pub mod weapons;

pub use ai::Steering;
pub use doors::{Door, DoorLeaf};
use actions::*;
use ai::*;
use doors::*;
use mission::*;
const EYE: f32 = 1.6;
pub struct GamePlugin;
impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<weapons::Gunplay>();
        app.add_systems(PostStartup, debug_setup.run_if(resource_exists::<Mission>));
        app.add_systems(
            Update,
            (
                actor_patrol,
                perceive,
                guard_combat,
                weapons::guard_fire,
                player_actions,
                weapons::gun_input,
                weapons::arm_pose,
                shoot,
            )
                .chain()
                .run_if(resource_exists::<Mission>)
                .run_if(in_state(crate::flow::Phase::Playing)),
        );
        app.add_systems(
            Update,
            (
                update_doors,
                soup_poison_timer,
                bjarkhov_meeting,
                update_dragged_body,
                hazards,
                messages_tick,
                restart,
                log_minds,
                weapons::fade_flashes,
                weapons::hold_models,
            )
                .run_if(resource_exists::<Mission>)
                .run_if(in_state(crate::flow::Phase::Playing)),
        );
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
    /// Hostile: hunting 47 (shooting on sight).
    pub alerted: bool,
    /// What an unalarmed guard is doing about something odd.
    pub mind: Mind,
    /// Where he last saw 47 (or the trouble) while hostile, and for how long he has lost him.
    pub last_seen: Option<Vec3>,
    pub lost_for: f32,
    /// The idle clip to go back to after aiming.
    calm_idle: Option<String>,
    fire_cooldown: f32,
    /// Rounds left in the current burst.
    burst: u8,
    /// Seconds spent sighting 47 down the rifle before the first burst (reaction time).
    sighting: f32,
    /// `m_iHitpoints`: 20 for everyone in the original.
    pub health: f32,
    /// A soldier whose AK-47 47 has taken.
    pub disarmed: bool,
}
/// A guard's state of mind short of open hostility.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Mind {
    #[default]
    Calm,
    /// Something is off about 47: stops and watches him.
    Wary,
    /// Goes to look at `spot`, then lingers there.
    Investigate { spot: Vec3, linger: f32 },
    /// Checks `left` more spots around `centre` before going back to his post.
    Search { centre: Vec3, spot: Vec3, left: u8, linger: f32 },
}
/// The unit a soldier belongs to, from his name: men of one unit know each other's faces.
pub fn unit(name: &str) -> Option<&'static str> {
    const UNITS: [(&str, &str); 10] = [
        ("Bodyguard", "Fuchs's detail"),
        ("Boat", "boat crew"),
        ("Submarine", "submarine guard"),
        ("Tunnel", "tunnel guard"),
        ("Sniper", "snipers"),
        ("Patrol", "patrol"),
        ("_s1", "first squad"),
        ("_s2", "second squad"),
        ("_s3", "third squad"),
        ("_p", "perimeter"),
    ];
    UNITS.iter().find(|(k, _)| name.contains(k)).map(|&(_, u)| u)
}
impl Actor {
    pub fn new(name: &str) -> Self {
        let outfit = Outfit::from_actor(name);
        let kind = match outfit {
            Outfit::Fuchs | Outfit::Bjarkhov => Kind::Target,
            Outfit::Soldier | Outfit::Bodyguard => Kind::Guard,
            _ => Kind::Civilian,
        };
        Self { name: name.to_owned(), kind, outfit, alive: true, stripped: false, found: false, suspicion: 0.0, alerted: false, mind: Mind::Calm, last_seen: None, lost_for: 0.0, calm_idle: None, fire_cooldown: 0.0, burst: 0, sighting: 0.0, health: 20.0, disarmed: false }
    }
}
/// A patrol route from the level's path data: nodes with optional pauses. Routes whose ends
/// meet are walked as loops; open ones back and forth.
#[derive(Component, Clone, Default, Debug)]
pub struct Patrol {
    pub nodes: Vec<(Vec3, f32)>,
    pub current: usize,
    pub wait_timer: f32,
    pub forward: bool,
    looped: bool,
    /// Where a guard without a route stands, and which way he faces (taken on the first frame).
    post: Option<(Vec3, Quat)>,
}
impl Patrol {
    pub fn new(nodes: Vec<(Vec3, f32)>) -> Self {
        let looped = nodes.len() > 2 && nodes.first().zip(nodes.last()).is_some_and(|(a, b)| a.0.distance(b.0) < 6.0);
        Self { nodes, current: 0, wait_timer: 0.0, forward: true, looped, post: None }
    }

    fn advance(&mut self) {
        let n = self.nodes.len();
        if self.looped {
            self.current = (self.current + 1) % n;
        } else if self.forward && self.current + 1 < n {
            self.current += 1;
        } else if !self.forward && self.current > 0 {
            self.current -= 1;
        } else {
            self.forward = !self.forward;
            self.advance();
        }
    }
}
/// Character models by actor name: the `_HMas`/`_NUDE` templates (outfits and stripped
/// bodies) and every placed actor's own model.
#[derive(Resource)]
pub struct Wardrobe(pub HashMap<String, SkinnedModel>);
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
    /// A guard's AK-47, once taken.
    AssaultRifle,
}
impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Item::Holstered => "Holstered",
            Item::FiberWire => "Fiber Wire",
            Item::SilencedPistol => "Silenced Silverballer",
            Item::PoisonVial => "Cyanide Poison",
            Item::BombRemote => "Bomb Remote Detonator",
            Item::AssaultRifle => "AK-47",
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
    /// The unit of the man whose uniform 47 wears.
    pub disguise_unit: Option<&'static str>,
    /// Sounds guards may come to check this frame: where, and how far they carry.
    pub noises: Vec<(Vec3, f32)>,
    pub active_item: Item,
    /// Silverballer rounds: in the gun and spare magazines.
    pub pistol: weapons::Ammo,
    /// The AK-47, if 47 has taken one from a dead soldier.
    pub rifle: Option<weapons::Ammo>,
    pub tnt: u32,
    pub sub_bombs_placed: u32,
    pub soup_poisoned: bool,
    pub soup_timer: Option<f32>,
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
            disguise_unit: None,
            noises: Vec::new(),
            active_item: Item::Holstered,
            pistol: weapons::Ammo { loaded: weapons::SILVERBALLER.magazine, spare: weapons::SILVERBALLER.magazine * 3 },
            rifle: None,
            tnt: 0,
            sub_bombs_placed: 0,
            soup_poisoned: false,
            soup_timer: None,
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
fn messages_tick(time: Res<Time>, mut mission: ResMut<Mission>) {
    let dt = time.delta_secs();
    for m in &mut mission.messages {
        m.1 -= dt;
    }
    mission.messages.retain(|m| m.1 > 0.0);
}
/// Enter after the mission ends restarts the game process (simplest full reset).
fn restart(keys: Res<ButtonInput<KeyCode>>, mission: Res<Mission>, mut exit: MessageWriter<AppExit>) {
    if mission.outcome.is_some() && keys.just_pressed(KeyCode::Enter) {
        crate::flow::restart_process(&mut exit);
    }
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
            "AssaultRifle" => {
                mission.rifle = Some(weapons::Ammo { loaded: 30, spare: 60 });
                Item::AssaultRifle
            }
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
