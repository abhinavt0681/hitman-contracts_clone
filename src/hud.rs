//! On-screen mission state: objectives, status, prompts, messages, outcome.

use crate::game::{Actor, Mission, Outcome};
use bevy::prelude::*;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_hud).add_systems(Update, update_hud.run_if(resource_exists::<Mission>));
    }
}

#[derive(Component)]
enum HudText {
    Objectives,
    Status,
    Prompt,
    Messages,
    Outcome,
    Controls,
}

fn text(size: f32, color: Color) -> (TextFont, TextColor, TextShadow) {
    (TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color), TextShadow::default())
}

fn spawn_hud(mut commands: Commands) {
    let pale = Color::srgb(0.92, 0.92, 0.88);
    let at = |top: Option<f32>, bottom: Option<f32>, left: Option<f32>| Node {
        position_type: PositionType::Absolute,
        top: top.map_or(Val::Auto, px),
        bottom: bottom.map_or(Val::Auto, px),
        left: left.map_or(Val::Auto, px),
        ..default()
    };
    commands.spawn((Text::new(""), text(17.0, pale), at(Some(14.0), None, Some(16.0)), HudText::Objectives));
    commands.spawn((Text::new(""), text(16.0, pale), at(None, Some(14.0), Some(16.0)), HudText::Status));
    commands.spawn((
        Text::new(""),
        text(20.0, Color::srgb(1.0, 0.93, 0.6)),
        TextLayout::justify(Justify::Center),
        Node { position_type: PositionType::Absolute, bottom: percent(22.0), width: percent(100.0), ..default() },
        HudText::Prompt,
    ));
    commands.spawn((
        Text::new(""),
        text(18.0, pale),
        TextLayout::justify(Justify::Center),
        Node { position_type: PositionType::Absolute, top: percent(14.0), width: percent(100.0), ..default() },
        HudText::Messages,
    ));
    commands.spawn((
        Text::new(""),
        text(34.0, Color::WHITE),
        TextLayout::justify(Justify::Center),
        Node { position_type: PositionType::Absolute, top: percent(38.0), width: percent(100.0), ..default() },
        HudText::Outcome,
    ));
    commands.spawn((
        Text::new("WASD: Move | Shift: Run | Ctrl / C: Sneak | 1-5 / Scroll: Item | E: Action | G: Drag/Drop Body | Click: Shoot"),
        text(13.0, Color::srgba(0.85, 0.85, 0.82, 0.60)),
        // Top right, clear of the status lines at the bottom.
        Node { position_type: PositionType::Absolute, top: px(14.0), right: px(16.0), max_width: percent(45.0), ..default() },
        HudText::Controls,
    ));
}

fn bar(v: f32) -> String {
    let n = (v.clamp(0.0, 1.0) * 10.0).round() as usize;
    format!("[{}{}]", "#".repeat(n), "-".repeat(10 - n))
}

fn update_hud(
    mission: Res<Mission>,
    actors: Query<&Actor>,
    player: Query<&crate::player::Player>,
    exposure: Res<crate::atmosphere::Exposure>,
    mut q: Query<(&HudText, &mut Text)>,
) {
    let suspicion = actors.iter().filter(|a| a.alive).map(|a| if a.alerted { 1.0 } else { a.suspicion }).fold(0.0, f32::max);
    let hunted = actors.iter().any(|a| a.alive && a.alerted);
    let crouching = player.single().is_ok_and(|p| p.crouching);
    let dragging = mission.dragging_body.is_some();
    let posture = if dragging {
        "   [DRAGGING BODY]"
    } else if crouching {
        "   [SNEAKING]"
    } else {
        ""
    };

    for (kind, mut t) in &mut q {
        let s = match kind {
            HudText::Objectives => mission
                .objectives
                .iter()
                .map(|o| format!("{} {}", if o.done { "[x]" } else { "[ ]" }, o.text))
                .collect::<Vec<_>>()
                .join("\n"),
            HudText::Status => format!(
                "Health {}   Suspicion {}{}{}\nLight {}{}\nOutfit: {}\nItem: [{}] {}{}{}",
                bar(mission.health / 100.0),
                bar(suspicion),
                match (hunted, mission.alarm) {
                    (true, _) => "   HUNTED",
                    (false, true) => "   ALERT",
                    _ => "",
                },
                posture,
                bar(exposure.light),
                if exposure.gust > 0.5 { "   [SQUALL]" } else { "" },
                crate::game::outfit_label(mission.outfit),
                match mission.active_item {
                    crate::game::Item::Holstered => "1",
                    crate::game::Item::FiberWire => "2",
                    crate::game::Item::SilencedPistol => "3",
                    crate::game::Item::PoisonVial => "4",
                    crate::game::Item::BombRemote => "5",
                },
                mission.active_item.label(),
                if mission.tnt > 0 { format!("   Dynamite x{}", mission.tnt) } else { String::new() },
                if mission.sub_bombs_placed > 0 { format!("   Sub Bombs: {}/3", mission.sub_bombs_placed) } else { String::new() },
            ),
            HudText::Prompt => mission.prompt.clone().unwrap_or_default(),
            HudText::Messages => mission.messages.iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>().join("\n"),
            HudText::Outcome => match mission.outcome {
                Some(Outcome::Complete) => format!(
                    "MISSION COMPLETE\nRating: {}\nTime {:.0}:{:02.0}\n\nEnter to play again",
                    mission.rating(),
                    (mission.elapsed / 60.0).floor(),
                    mission.elapsed % 60.0
                ),
                Some(Outcome::Failed) => "MISSION FAILED\n\nEnter to try again".to_owned(),
                None => String::new(),
            },
            HudText::Controls => continue,
        };
        if t.0 != s {
            t.0 = s;
        }
    }
}
