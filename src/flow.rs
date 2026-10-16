//! Game flow around the mission: Diana's briefing, the pause menu and the map screen.
//!
//! Gameplay only runs in `Phase::Playing`; the briefing, pause menu and map stop the world
//! clock (`Time<Virtual>`). The briefing is the original's: Diana's four voiced lines
//! (`C02-1_DS_01..04`) with their subtitles, the intel and the objectives from the `.LOC`.

use crate::{
    audio::SoundBank,
    game::{Mission, Text as Strings},
};
use bevy::{
    audio::{AudioPlayer, PlaybackSettings},
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

#[derive(States, Default, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Phase {
    #[default]
    Briefing,
    Playing,
    Paused,
    Map,
}

pub struct FlowPlugin;

impl Plugin for FlowPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<Phase>()
            .add_systems(Startup, skip_briefing_in_tests)
            .add_systems(OnEnter(Phase::Briefing), (open_briefing, stop_clock))
            .add_systems(OnExit(Phase::Briefing), close_briefing)
            .add_systems(OnEnter(Phase::Paused), (open_pause_menu, stop_clock, free_cursor))
            .add_systems(OnExit(Phase::Paused), close::<PauseMenu>)
            .add_systems(OnEnter(Phase::Map), (stop_clock, free_cursor))
            .add_systems(OnEnter(Phase::Playing), (start_clock, lock_cursor))
            .add_systems(
                Update,
                (
                    run_briefing.run_if(in_state(Phase::Briefing)),
                    open_menus.run_if(in_state(Phase::Playing)),
                    run_pause_menu.run_if(in_state(Phase::Paused)),
                    leave_map.run_if(in_state(Phase::Map)),
                ),
            );
    }
}

/// Scripted test runs (`HITMAN_SCREENSHOT`, `HITMAN_KEYS`, …) start straight in the mission,
/// as does `HITMAN_SKIP_BRIEFING`.
fn skip_briefing_in_tests(mut next: ResMut<NextState<Phase>>) {
    let scripted = ["HITMAN_SCREENSHOT", "HITMAN_KEYS", "HITMAN_AUTOWALK", "HITMAN_SKIP_BRIEFING"].iter().any(|v| std::env::var_os(v).is_some());
    // `HITMAN_PHASE=briefing|paused|map` opens that screen (for screenshots).
    match std::env::var("HITMAN_PHASE").as_deref() {
        Ok("briefing") => next.set(Phase::Briefing),
        Ok("paused") => next.set(Phase::Paused),
        Ok("map") => next.set(Phase::Map),
        _ if scripted => next.set(Phase::Playing),
        _ => {}
    }
}

fn stop_clock(mut time: ResMut<Time<Virtual>>) {
    time.pause();
}

fn start_clock(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

fn free_cursor(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
}

fn lock_cursor(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = false;
    }
}

fn close<T: Component>(mut commands: Commands, roots: Query<Entity, With<T>>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
}

const PALE: Color = Color::srgb(0.88, 0.9, 0.88);
const DIM: Color = Color::srgb(0.55, 0.6, 0.6);
const ACCENT: Color = Color::srgb(0.75, 0.12, 0.1);

/// The UI font lacks typographic quotes and dashes the original text uses.
pub fn plain(s: &str) -> String {
    s.replace(['’', '‘'], "'").replace(['“', '”'], "\"").replace(['–', '—'], "-").replace('…', "...")
}

fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(plain(&s.into())), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

// ---------------------------------------------------------------- briefing

#[derive(Component)]
struct Briefing;

/// The voiced line now shown (index into Diana's lines) and whether its clip is playing.
#[derive(Component)]
struct BriefingLine(usize);

#[derive(Component)]
struct BriefingVoice;

/// Diana's lines and their voice clips.
pub const DIANA: [&str; 4] = ["C02-1_DS_01", "C02-1_DS_02", "C02-1_DS_03", "C02-1_DS_04"];

fn open_briefing(mut commands: Commands, strings: Option<Res<Strings>>) {
    let get = |k: &str| strings.as_ref().and_then(|s| s.0.get(k)).unwrap_or("").to_owned();
    commands
        .spawn((
            Briefing,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(percent(12.0), px(40.0)),
                row_gap: px(14.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.02, 0.025, 0.94)),
            GlobalZIndex(10),
        ))
        .with_children(|b| {
            b.spawn(text("I C A   C O N T R A C T", 14.0, ACCENT));
            b.spawn(text(get("C02-1").to_uppercase(), 38.0, PALE));
            b.spawn((text("", 19.0, PALE), BriefingLine(0), Node { min_height: px(110.0), max_width: px(900.0), ..default() }));
            b.spawn(text(format!("TARGETS\n  {}\n  {}", get("Target1"), get("Target2")), 16.0, PALE));
            b.spawn(text((1..=4).map(|i| get(&format!("Info{i}"))).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n"), 15.0, DIM));
            b.spawn(text(
                format!("{}\n{}", get("MissionObjectives"), (1..=4).map(|i| format!("  {}", get(&format!("Objective{i}")))).collect::<Vec<_>>().join("\n")),
                16.0,
                PALE,
            ));
            b.spawn(text("Enter / click: begin mission     Space: next line", 14.0, DIM));
        });
}

fn close_briefing(mut commands: Commands, roots: Query<Entity, Or<(With<Briefing>, With<BriefingVoice>)>>) {
    for e in &roots {
        commands.entity(e).despawn();
    }
}

/// Plays Diana's lines one after another, showing each as it's spoken.
#[allow(clippy::too_many_arguments)]
fn run_briefing(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    strings: Option<Res<Strings>>,
    bank: Option<Res<SoundBank>>,
    mut line: Query<(&mut BriefingLine, &mut Text)>,
    voice: Query<Entity, With<BriefingVoice>>,
    mut next: ResMut<NextState<Phase>>,
    mut started: Local<bool>,
) {
    if keys.just_pressed(KeyCode::Enter) || mouse.just_pressed(MouseButton::Left) || keys.just_pressed(KeyCode::Escape) {
        next.set(Phase::Playing);
        return;
    }
    let Ok((mut at, mut shown)) = line.single_mut() else { return };
    let skip = keys.just_pressed(KeyCode::Space);
    // Next line when the clip has finished (its player despawns) or on Space.
    if voice.is_empty() || skip {
        if *started || skip {
            at.0 += 1;
        }
        for v in &voice {
            commands.entity(v).despawn();
        }
        if at.0 >= DIANA.len() {
            return;
        }
        *started = true;
        let key = DIANA[at.0];
        shown.0 = plain(strings.as_ref().and_then(|s| s.0.get(key)).unwrap_or(""));
        if let Some(clip) = bank.as_ref().and_then(|b| b.voices.get(key)) {
            commands.spawn((AudioPlayer::new(clip.clone()), PlaybackSettings::DESPAWN, BriefingVoice));
        }
    }
}

// ---------------------------------------------------------------- playing

fn open_menus(keys: Res<ButtonInput<KeyCode>>, mission: Option<Res<Mission>>, mut next: ResMut<NextState<Phase>>) {
    // Once the mission is over the outcome screen owns the keyboard.
    if mission.is_some_and(|m| m.outcome.is_some()) {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        next.set(Phase::Paused);
    } else if keys.just_pressed(KeyCode::KeyM) || keys.just_pressed(KeyCode::Tab) {
        next.set(Phase::Map);
    }
}

fn leave_map(keys: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Phase>>) {
    if [KeyCode::KeyM, KeyCode::Tab, KeyCode::Escape].iter().any(|k| keys.just_pressed(*k)) {
        next.set(Phase::Playing);
    }
}

// ---------------------------------------------------------------- pause menu

#[derive(Component)]
struct PauseMenu;

#[derive(Component, Clone, Copy, PartialEq)]
enum MenuItem {
    Resume,
    Briefing,
    Restart,
    Quit,
}

/// Which item the keyboard has selected.
#[derive(Component)]
struct Selected(usize);

const ITEMS: [(MenuItem, &str); 4] = [(MenuItem::Resume, "Resume"), (MenuItem::Briefing, "Briefing"), (MenuItem::Restart, "Restart mission"), (MenuItem::Quit, "Quit")];

const CONTROLS: &str = "WASD move   Shift run   Ctrl / C sneak   Mouse look\n\
E action   F take a rifle   G drag / drop body   1-6 or wheel: item\n\
Left click: use item / fire   Right mouse: aim   R reload\n\
M / Tab map   Esc menu   F1 free camera";

fn open_pause_menu(mut commands: Commands, mission: Option<Res<Mission>>) {
    commands
        .spawn((
            PauseMenu,
            Selected(0),
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: px(10.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            GlobalZIndex(10),
        ))
        .with_children(|m| {
            m.spawn(text("PAUSED", 30.0, PALE));
            if let Some(mission) = &mission {
                m.spawn((text(crate::hud::objectives_text(mission), 15.0, PALE), Node { margin: UiRect::vertical(px(14.0)), ..default() }));
            }
            for (item, label) in ITEMS {
                m.spawn((
                    bevy::picking::hover::Hovered::default(),
                    item,
                    Node { width: px(280.0), padding: UiRect::all(px(8.0)), justify_content: JustifyContent::Center, ..default() },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|b| {
                    b.spawn(text(label, 20.0, PALE));
                });
            }
            m.spawn((text(CONTROLS, 14.0, DIM), Node { margin: UiRect::top(px(24.0)), ..default() }, TextLayout::justify(Justify::Center)));
        });
}

fn run_pause_menu(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut menu: Query<&mut Selected, With<PauseMenu>>,
    mut buttons: Query<(&MenuItem, &bevy::picking::hover::Hovered, &mut BackgroundColor)>,
    mut next: ResMut<NextState<Phase>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(mut sel) = menu.single_mut() else { return };
    if keys.just_pressed(KeyCode::Escape) {
        next.set(Phase::Playing);
        return;
    }
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        sel.0 = (sel.0 + 1) % ITEMS.len();
    }
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        sel.0 = (sel.0 + ITEMS.len() - 1) % ITEMS.len();
    }
    let mut chosen = (keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space)).then_some(ITEMS[sel.0].0);
    for (item, hovered, mut bg) in &mut buttons {
        let index = ITEMS.iter().position(|(i, _)| i == item).unwrap_or(0);
        if hovered.0 {
            sel.0 = index;
            if mouse.just_pressed(MouseButton::Left) {
                chosen = Some(*item);
            }
        }
        bg.0 = if index == sel.0 { Color::srgba(0.75, 0.12, 0.1, 0.55) } else { Color::NONE };
    }
    match chosen {
        Some(MenuItem::Resume) => next.set(Phase::Playing),
        Some(MenuItem::Briefing) => next.set(Phase::Briefing),
        Some(MenuItem::Restart) => restart_process(&mut exit),
        Some(MenuItem::Quit) => {
            exit.write(AppExit::Success);
        }
        None => {}
    }
}

/// Starts the mission over: a fresh process is the simplest complete reset.
pub fn restart_process(exit: &mut MessageWriter<AppExit>) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(std::env::args().skip(1)).env("HITMAN_SKIP_BRIEFING", "1").spawn();
    }
    exit.write(AppExit::Success);
}
