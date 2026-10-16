//! Dialogue scenes, as the original plays them in-game: the screen letterboxes, the camera
//! cuts to an over-the-shoulder shot of whoever is speaking, the line plays in its original
//! voice with its subtitle in the lower bar, and 47 waits until it's over.
//!
//! Gameplay asks for a scene with `Cutscene::play` (lines, speakers, listeners) and checks
//! `Cutscene::finished` to carry on. Lines are the level's own voice files, keyed as in its
//! string table (`C02-1_GIHMI_01G` = "Go right in, Mr. Fuchs, Commander is expecting you.").

use crate::{
    audio::SoundBank,
    collision::WorldCollision,
    game::{Actor, Mission, Text as Strings},
    hud::{HudFont, hud_text},
    player::Player,
};
use bevy::{audio::Volume, prelude::*};
use std::collections::VecDeque;

/// Every line a scene can play; `audio` loads these with their lengths.
pub const LINES: &[&str] = &[
    // The gate: "Go right in, Mr. Fuchs…" / "Thanks, soldier." / "Second door on right."
    "C02-1_GIHMI_01G",
    "C02-1_GIHMI_02H",
    "C02-1_GIHMI_03G",
    // The frisk on deck: "Standard procedure sir, stand still." / "I understand."
    "C02-1_GPHMD_01Guard",
    "C02-1_GPHMD_01HM",
    // A Russian soldier frisking: "Please stand still, sir." / "Thanks, sir, you're fine.
    // Move along." / "What the .. Alarm !"
    "All_R_FS_Frisk01M1",
    "All_R_FS_FriskOKM1",
    "All_R_FS_FriskBADM1",
    // Bjarkhov: "So good to meet you at last Fabian…" / "None for me, thanks." / "No? Hm.
    // Very well Fabian...Very well"
    "C02-1_BGHM_01CB",
    "C02-1_BGHM_02HM",
    "C02-1_BGHM_03CB",
];

/// A pause after each line before the next (s).
const BEAT: f32 = 0.35;
/// Each line's shot: this far in front of the speaker (towards the listener), off to the side
/// and above eye level (m).
const SHOT_DISTANCE: f32 = 1.9;
const SHOT_SIDE: f32 = 0.45;
const SHOT_UP: f32 = 0.05;
/// Letterbox bars, each this share of the screen height.
const BAR: f32 = 12.0;

pub struct CutscenePlugin;

impl Plugin for CutscenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Cutscene>()
            .init_resource::<SceneCamera>()
            .add_systems(Startup, spawn_letterbox)
            .add_systems(Update, (run, show).chain().run_if(resource_exists::<Mission>).run_if(in_state(crate::flow::Phase::Playing)));
    }
}

/// One line of a scene: who says it (`speaker` faces `listener`), and an optional clip the
/// speaker plays meanwhile (a frisk, a gesture).
pub struct Line {
    pub key: &'static str,
    pub speaker: Entity,
    pub listener: Entity,
    pub clip: Option<&'static str>,
}

/// On characters taking part in a scene: their AI leaves them standing where they are.
#[derive(Component)]
pub struct InScene;

#[derive(Resource, Default)]
pub struct Cutscene {
    id: Option<&'static str>,
    participants: Vec<Entity>,
    queue: VecDeque<Line>,
    current: Option<(Line, f32)>,
    subtitle: String,
    finished: Option<&'static str>,
}

impl Cutscene {
    pub fn play(&mut self, id: &'static str, lines: Vec<Line>) {
        self.id = Some(id);
        self.queue = lines.into();
        self.current = None;
        self.finished = None;
    }

    pub fn playing(&self) -> bool {
        self.current.is_some() || !self.queue.is_empty()
    }

    /// True once, when scene `id` has just ended.
    pub fn finished(&mut self, id: &str) -> bool {
        if self.finished == Some(id) {
            self.finished = None;
            true
        } else {
            false
        }
    }
}

/// The camera shot of the line playing, for `player::follow_camera` to use instead of its own.
#[derive(Resource, Default)]
pub struct SceneCamera(pub Option<Transform>);

#[derive(Component)]
struct Letterbox;
#[derive(Component)]
struct SceneSubtitle;

fn spawn_letterbox(mut commands: Commands, font: Res<HudFont>) {
    let bar = |top: bool| Node {
        position_type: PositionType::Absolute,
        top: if top { percent(0.0) } else { Val::Auto },
        bottom: if top { Val::Auto } else { percent(0.0) },
        width: percent(100.0),
        height: percent(BAR),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        display: Display::None,
        ..default()
    };
    commands.spawn((Letterbox, bar(true), BackgroundColor(Color::BLACK), GlobalZIndex(5)));
    commands.spawn((Letterbox, bar(false), BackgroundColor(Color::BLACK), GlobalZIndex(5))).with_children(|b| {
        b.spawn((SceneSubtitle, Text::new(""), hud_text(&font, 14.0, Color::srgb(0.88, 0.9, 0.88)), TextLayout::justify(Justify::Center), Node { max_width: percent(80.0), ..default() }));
    });
}

#[allow(clippy::too_many_arguments)]
fn run(
    mut commands: Commands,
    time: Res<Time>,
    strings: Res<Strings>,
    world: Res<WorldCollision>,
    bank: Option<Res<SoundBank>>,
    mut scene: ResMut<Cutscene>,
    mut camera: ResMut<SceneCamera>,
    mut mission: ResMut<Mission>,
    mut bodies: Query<(&mut Transform, Has<Player>), Or<(With<Actor>, With<Player>)>>,
    mut animators: Query<&mut crate::character::Animator>,
    mut gaits: Query<&mut crate::character::Gait>,
) {
    if !scene.playing() {
        camera.0 = None;
        return;
    }
    // 47 stays put while people talk.
    mission.busy = mission.busy.max(0.2);
    if let Some((line, left)) = scene.current.as_mut() {
        // Every frame: nothing else may turn them away mid-line.
        face_each_other(line, &mut bodies);
        *left -= time.delta_secs();
        if *left > 0.0 {
            return;
        }
        scene.current = None;
    }
    let Some(line) = scene.queue.pop_front() else {
        for e in std::mem::take(&mut scene.participants) {
            if let Ok(mut ec) = commands.get_entity(e) {
                ec.remove::<InScene>();
            }
        }
        scene.finished = scene.id.take();
        scene.subtitle.clear();
        camera.0 = None;
        return;
    };
    let (Ok((s, _)), Ok((l, _))) = (bodies.get(line.speaker), bodies.get(line.listener)) else {
        return; // someone's gone: skip the line
    };
    let (s, l) = (s.translation, l.translation);
    for e in [line.speaker, line.listener] {
        if !scene.participants.contains(&e) {
            scene.participants.push(e);
            commands.entity(e).insert(InScene);
        }
        if let Ok(mut gait) = gaits.get_mut(e) {
            gait.speed = 0.0;
        }
    }
    face_each_other(&line, &mut bodies);
    // The speaker, framed at their own eye level from the listener's side: over the
    // listener's shoulder when they stand close, a level medium shot when they don't.
    let (speaker_head, listener_head) = (s + Vec3::Y * 1.62, l + Vec3::Y * 1.62);
    let to_listener = (listener_head - speaker_head).with_y(0.0).normalize_or_zero();
    let side = to_listener.cross(Vec3::Y);
    let wanted = speaker_head + to_listener * SHOT_DISTANCE - side * SHOT_SIDE + Vec3::Y * SHOT_UP;
    let to_cam = wanted - speaker_head;
    let at = world.raycast(speaker_head, to_cam, to_cam.length()).map_or(wanted, |h| speaker_head + to_cam.normalize() * (h.distance - 0.15).max(0.4));
    camera.0 = Some(Transform::from_translation(at).looking_at(speaker_head - Vec3::Y * 0.12, Vec3::Y));

    let length = bank.as_ref().and_then(|b| b.lines.get(line.key)).map_or(3.0, |l| l.1);
    if let Some((voice, _)) = bank.as_ref().and_then(|b| b.lines.get(line.key)) {
        commands.spawn((AudioPlayer::new(voice.clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(0.95))));
    }
    if let (Some(clip), Ok(mut anim)) = (line.clip, animators.get_mut(line.speaker)) {
        anim.play_once(clip);
    }
    scene.subtitle = crate::flow::plain(strings.0.get(line.key).unwrap_or(""));
    debug!("scene line {} ({length:.1}s): {}", line.key, scene.subtitle);
    scene.current = Some((line, length + BEAT));
}

/// Speaker and listener turn to each other.
fn face_each_other(line: &Line, bodies: &mut Query<(&mut Transform, Has<Player>), Or<(With<Actor>, With<Player>)>>) {
    let (Ok((s, _)), Ok((l, _))) = (bodies.get(line.speaker), bodies.get(line.listener)) else { return };
    let (s, l) = (s.translation, l.translation);
    for (who, to) in [(line.speaker, l - s), (line.listener, s - l)] {
        if let Ok((mut t, is_player)) = bodies.get_mut(who) {
            let to = to.with_y(0.0).normalize_or_zero();
            if to != Vec3::ZERO {
                t.rotation = if is_player { Transform::IDENTITY.looking_to(to, Vec3::Y).rotation } else { crate::game::facing(to) };
            }
        }
    }
}

/// Letterbox and subtitle while a scene plays.
fn show(scene: Res<Cutscene>, mut bars: Query<&mut Node, With<Letterbox>>, mut subtitle: Query<&mut Text, With<SceneSubtitle>>) {
    let on = scene.playing() || scene.current.is_some();
    for mut node in &mut bars {
        let want = if on { Display::Flex } else { Display::None };
        if node.display != want {
            node.display = want;
        }
    }
    if let Ok(mut t) = subtitle.single_mut() {
        if t.0 != scene.subtitle {
            t.0 = scene.subtitle.clone();
        }
    }
}
