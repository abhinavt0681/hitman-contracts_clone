//! The HUD, after the original's: next to nothing on screen.
//!
//! - Bottom left, two thin bars (`HealthStealth_01a` in the level's textures): health, with a
//!   red band at its foot that glows as 47 weakens; and suspicion, the most suspicious guard's,
//!   a black block opening out from the middle that fills the bar when 47 is hunted.
//! - Bottom right, the item in hand: the weapon itself (its model, rendered live on a layer of
//!   its own as the original renders its icons), the calibre, the magazine as a column of
//!   marks and every round carried as one number.
//! - A small crosshair that opens with the spread of the next shot.
//!
//! Prompts, messages and the outcome stay; objectives live on the map and pause screens, and
//! the controls on the pause screen.

use crate::{
    game::{
        Actor, Item, Mission, Outcome,
        weapons::{self, Gunplay},
    },
    player::{OrbitCamera, Player},
    view::WeaponModels,
};
use bevy::{
    camera::{RenderTarget, ScalingMode, visibility::RenderLayers},
    prelude::*,
    render::render_resource::TextureFormat,
    text::LetterSpacing,
};

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudFont>()
            .add_systems(Startup, spawn_hud)
            .add_systems(Update, (stage_icons, update_hud.run_if(resource_exists::<Mission>)));
    }
}

/// The HUD's typeface: Michroma (OFL), close to the original's Bank Gothic.
#[derive(Resource, Clone)]
pub struct HudFont(pub Handle<Font>);

impl FromWorld for HudFont {
    fn from_world(world: &mut World) -> Self {
        let font = Font::from_bytes(include_bytes!("fonts/Michroma-Regular.ttf").to_vec());
        HudFont(world.resource_mut::<Assets<Font>>().add(font))
    }
}

/// Text in the HUD's typeface, tracked out like the original's.
pub fn hud_text(font: &HudFont, size: f32, color: Color) -> (TextFont, TextColor, LetterSpacing, TextShadow) {
    (
        TextFont { font: font.0.clone().into(), font_size: FontSize::Px(size), ..default() },
        TextColor(color),
        LetterSpacing::Px(size * 0.12),
        TextShadow { color: Color::srgba(0.0, 0.0, 0.0, 0.6), ..default() },
    )
}

const PALE: Color = Color::srgb(0.88, 0.9, 0.88);
/// The translucent grey of the bars and the item panel.
const GLASS: Color = Color::srgba(0.72, 0.76, 0.76, 0.32);
const GLASS_DARK: Color = Color::srgba(0.08, 0.1, 0.11, 0.35);
const EDGE: Color = Color::srgba(0.95, 0.97, 0.97, 0.85);
const BLOOD: Color = Color::srgb(0.5, 0.0, 0.0);
const ROUND: Color = Color::srgba(0.06, 0.07, 0.08, 0.85);
const SPENT: Color = Color::srgba(0.06, 0.07, 0.08, 0.18);

const BAR_W: f32 = 15.0;
const BAR_H: f32 = 66.0;
const MAGAZINE_H: f32 = 74.0;
/// The most marks the magazine column holds (the AK-47's 30).
const MAX_ROUNDS: u32 = 30;

/// The icon stage: a render layer of its own, far below the level.
const ICON_LAYER: usize = 2;
const ICON_AT: Vec3 = Vec3::new(0.0, -2000.0, 0.0);
const ICON_SIZE: UVec2 = UVec2::new(320, 160);

#[derive(Component, Clone, Copy, PartialEq)]
enum HudPart {
    HealthFill,
    HealthBand,
    SuspicionBlock,
    Panel,
    Magazine,
    Round(u32),
    Crosshair,
    /// Up, down, left, right.
    Tick(u8),
}

#[derive(Component)]
enum HudLine {
    ItemName,
    Carried,
    Extra,
    Prompt,
    Messages,
    Outcome,
}

/// A weapon model on the icon stage.
#[derive(Component)]
struct IconModel(Item);

fn spawn_hud(mut commands: Commands, font: Res<HudFont>, mut images: ResMut<Assets<Image>>) {
    let icon = images.add(Image::new_target_texture(ICON_SIZE.x, ICON_SIZE.y, TextureFormat::Rgba8UnormSrgb, None));
    spawn_icon_stage(&mut commands, icon.clone());

    let absolute = || Node { position_type: PositionType::Absolute, ..default() };
    let bar = || {
        (Node { width: px(BAR_W), height: px(BAR_H), border: UiRect::bottom(px(1.0)), ..default() }, BorderColor::all(EDGE))
    };
    let fill = |part: HudPart, color: Color, height: f32| {
        (part, Node { position_type: PositionType::Absolute, bottom: px(0.0), width: percent(100.0), height: percent(height), ..default() }, BackgroundColor(color))
    };

    // Health and suspicion.
    commands
        .spawn(Node { position_type: PositionType::Absolute, left: px(16.0), bottom: px(16.0), column_gap: px(3.0), ..default() })
        .with_children(|v| {
            v.spawn((bar(), BackgroundColor(GLASS_DARK))).with_children(|b| {
                b.spawn(fill(HudPart::HealthFill, GLASS, 100.0));
                b.spawn(fill(HudPart::HealthBand, BLOOD, 9.0));
            });
            v.spawn((bar(), BackgroundColor(GLASS))).with_children(|b| {
                b.spawn((
                    Node { position_type: PositionType::Absolute, top: percent(50.0), width: percent(100.0), height: px(1.0), ..default() },
                    BackgroundColor(ROUND),
                ));
                b.spawn((
                    HudPart::SuspicionBlock,
                    Node { position_type: PositionType::Absolute, top: percent(50.0), width: percent(100.0), height: px(0.0), ..default() },
                    BackgroundColor(Color::BLACK),
                ));
            });
        });

    // The item in hand.
    commands
        .spawn((
            HudPart::Panel,
            Node { position_type: PositionType::Absolute, right: px(16.0), bottom: px(16.0), border: UiRect::bottom(px(1.0)), ..default() },
            BorderColor::all(EDGE),
            BackgroundColor(GLASS),
        ))
        .with_children(|p| {
            p.spawn(Node { flex_direction: FlexDirection::Column, align_items: AlignItems::FlexEnd, padding: UiRect::new(px(10.0), px(8.0), px(2.0), px(5.0)), ..default() })
                .with_children(|c| {
                    c.spawn((ImageNode::new(icon), Node { width: px(ICON_SIZE.x as f32 * 0.5), height: px(ICON_SIZE.y as f32 * 0.5), ..default() }));
                    c.spawn((Text::new(""), hud_text(&font, 13.0, PALE), HudLine::ItemName));
                });
            p.spawn((
                HudPart::Magazine,
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::FlexEnd,
                    width: px(52.0),
                    padding: UiRect::new(px(6.0), px(6.0), px(6.0), px(5.0)),
                    border: UiRect::left(px(1.0)),
                    ..default()
                },
                BorderColor::all(Color::srgba(0.1, 0.12, 0.13, 0.6)),
                BackgroundColor(Color::NONE),
            ))
            .with_children(|m| {
                // Rounds stack from the bottom; the top ones go first.
                m.spawn(Node { flex_direction: FlexDirection::ColumnReverse, width: px(22.0), height: px(MAGAZINE_H), row_gap: px(1.0), ..default() })
                    .with_children(|col| {
                        for i in 0..MAX_ROUNDS {
                            col.spawn((HudPart::Round(i), Node { width: percent(100.0), height: px(2.0), ..default() }, BackgroundColor(ROUND)));
                        }
                    });
                m.spawn((Text::new(""), hud_text(&font, 13.0, PALE), HudLine::Carried, Node { margin: UiRect::top(px(4.0)), ..default() }));
            });
        });
    commands.spawn((Text::new(""), hud_text(&font, 12.0, PALE), Node { right: px(18.0), bottom: px(150.0), ..absolute() }, HudLine::Extra));

    // Crosshair: four ticks around the point of aim.
    commands
        .spawn((HudPart::Crosshair, Node { left: percent(50.0), top: percent(50.0), width: px(0.0), height: px(0.0), ..absolute() }, BackgroundColor(Color::NONE)))
        .with_children(|c| {
            for i in 0..4 {
                c.spawn((HudPart::Tick(i), absolute(), BackgroundColor(Color::srgba(0.92, 0.95, 0.93, 0.85))));
            }
        });

    let centred = |top: f32| Node { top: percent(top), width: percent(100.0), ..absolute() };
    commands.spawn((Text::new(""), hud_text(&font, 15.0, Color::srgb(1.0, 0.93, 0.6)), TextLayout::justify(Justify::Center), centred(74.0), HudLine::Prompt));
    commands.spawn((Text::new(""), hud_text(&font, 14.0, PALE), TextLayout::justify(Justify::Center), centred(14.0), HudLine::Messages));
    commands.spawn((Text::new(""), hud_text(&font, 26.0, Color::WHITE), TextLayout::justify(Justify::Center), centred(38.0), HudLine::Outcome));
}

/// A camera and a light that see only the icon layer, rendering into the panel's image.
fn spawn_icon_stage(commands: &mut Commands, image: Handle<Image>) {
    let layer = RenderLayers::layer(ICON_LAYER);
    commands.spawn((
        Camera3d::default(),
        Camera { order: -1, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
        RenderTarget::Image(image.into()),
        Projection::Orthographic(OrthographicProjection { scaling_mode: ScalingMode::Fixed { width: 2.0, height: 1.0 }, ..OrthographicProjection::default_3d() }),
        Transform::from_translation(ICON_AT + Vec3::Z * 10.0).looking_at(ICON_AT, Vec3::Y),
        AmbientLight { color: Color::srgb(0.75, 0.85, 0.9), brightness: 900.0, ..default() },
        layer.clone(),
        Name::new("Icon camera"),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 4000.0, ..default() },
        Transform::from_translation(ICON_AT + Vec3::new(-2.0, 4.0, 5.0)).looking_at(ICON_AT, Vec3::Y),
        layer,
    ));
}

/// Puts the held weapons' models on the icon stage once the level has loaded, each turned
/// side-on (thinnest axis towards the camera, longest across) and scaled to fill the frame.
fn stage_icons(mut commands: Commands, models: Option<Res<WeaponModels>>, meshes: Res<Assets<Mesh>>, staged: Query<(), With<IconModel>>) {
    let Some(models) = models else { return };
    if !staged.is_empty() {
        return;
    }
    let layer = RenderLayers::layer(ICON_LAYER);
    for item in [Item::SilencedPistol, Item::AssaultRifle, Item::FiberWire] {
        let Some(model) = weapons::model_for(item).and_then(|n| models.0.get(n)) else { continue };
        let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
        for (mesh, _, t) in &model.parts {
            let Some(positions) = meshes.get(mesh).and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION)).and_then(|a| a.as_float3()) else { continue };
            for p in positions {
                let p = t.transform_point(Vec3::from(*p));
                (lo, hi) = (lo.min(p), hi.max(p));
            }
        }
        if lo.x > hi.x {
            return; // meshes not ready yet
        }
        // Muzzle to the left, as the original draws its weapons.
        let rotation = Quat::from_rotation_y(std::f32::consts::PI) * side_on(hi - lo);
        let size = rotation * (hi - lo);
        let scale = (1.8 / size.x.abs()).min(0.8 / size.y.abs());
        let centre = (lo + hi) * 0.5;
        let holder = commands
            .spawn((
                IconModel(item),
                Transform { translation: ICON_AT - rotation * centre * scale, rotation, scale: Vec3::splat(scale) },
                Visibility::Hidden,
            ))
            .id();
        for (mesh, material, t) in &model.parts {
            let part = commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), *t, layer.clone())).id();
            commands.entity(holder).add_child(part);
        }
    }
}

/// The rotation laying a model's longest axis along X (across the icon) and its thinnest
/// along Z (towards the camera).
fn side_on(extent: Vec3) -> Quat {
    let mut axes = [(extent.x, Vec3::X), (extent.y, Vec3::Y), (extent.z, Vec3::Z)];
    axes.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (long, mid) = (axes[0].1, axes[1].1);
    // Model axes → icon axes: long → X, mid → Y, and Z follows (a proper rotation).
    Quat::from_mat3(&Mat3::from_cols(long, mid, long.cross(mid)).transpose())
}

fn ammo_name(item: Item) -> &'static str {
    match item {
        Item::SilencedPistol => ".45 ACP",
        Item::AssaultRifle => "5.56 MM",
        Item::FiberWire => "FIBER WIRE",
        Item::PoisonVial => "POISON",
        Item::BombRemote => "REMOTE",
        Item::Holstered => "",
    }
}

#[allow(clippy::too_many_arguments)]
fn update_hud(
    time: Res<Time<Real>>,
    mission: Res<Mission>,
    gun: Res<Gunplay>,
    actors: Query<&Actor>,
    player: Query<&Player>,
    camera: Query<&OrbitCamera>,
    mut parts: Query<(&HudPart, &mut Node, &mut BackgroundColor)>,
    mut lines: Query<(&HudLine, &mut Text)>,
    mut icons: Query<(&IconModel, &mut Visibility)>,
) {
    let health = (mission.health / 100.0).clamp(0.0, 1.0);
    let hunted = actors.iter().any(|a| a.alive && a.alerted);
    let suspicion = if hunted { 1.0 } else { actors.iter().filter(|a| a.alive).map(|a| a.suspicion).fold(0.0, f32::max).min(1.0) };
    let item = mission.active_item;
    let playing = mission.outcome.is_none();
    let weapon = weapons::weapon_of(item);
    let ammo = if item == Item::AssaultRifle { mission.rifle.unwrap_or_default() } else { mission.pistol };
    let (aim, speed, crouching) = (camera.single().map_or(0.0, |c| c.aim), player.single().map_or(0.0, |p| p.speed()), player.single().is_ok_and(|p| p.crouching));
    // Pixels from the point of aim to each tick: wider as the next shot gets wilder.
    let gap = weapon.map_or(4.0, |w| (2.0 + 5.0 * gun.cone(w, aim, speed, crouching)).clamp(2.0, 48.0));
    // The red band at the foot of the health bar pulses once 47 is badly hurt.
    let band = if health < 0.3 { BLOOD.mix(&Color::srgb(0.9, 0.05, 0.02), 0.5 + 0.5 * (time.elapsed_secs() * 5.0).sin()) } else { BLOOD };

    for (part, mut node, mut color) in &mut parts {
        match *part {
            HudPart::HealthFill => node.height = percent(health * 100.0),
            HudPart::HealthBand => color.0 = band,
            HudPart::SuspicionBlock => {
                node.top = percent(50.0 * (1.0 - suspicion));
                node.height = percent(100.0 * suspicion);
                color.0 = if hunted { Color::srgb(0.25, 0.0, 0.0) } else { Color::BLACK };
            }
            HudPart::Panel => node.display = if playing && item != Item::Holstered { Display::Flex } else { Display::None },
            HudPart::Magazine => node.display = if weapon.is_some() { Display::Flex } else { Display::None },
            HudPart::Round(i) => {
                let magazine = weapon.map_or(0, |w| w.magazine);
                node.display = if i < magazine { Display::Flex } else { Display::None };
                node.height = px(((MAGAZINE_H + 1.0) / magazine.max(1) as f32 - 1.0).floor().max(1.0));
                color.0 = if i < ammo.loaded { ROUND } else { SPENT };
            }
            HudPart::Crosshair => node.display = if playing && weapon.is_some() { Display::Flex } else { Display::None },
            HudPart::Tick(i) => {
                let (len, thick) = (6.0, 2.0);
                let (left, top, w, h) = match i {
                    0 => (-thick / 2.0, -gap - len, thick, len),
                    1 => (-thick / 2.0, gap, thick, len),
                    2 => (-gap - len, -thick / 2.0, len, thick),
                    _ => (gap, -thick / 2.0, len, thick),
                };
                (node.left, node.top, node.width, node.height) = (px(left), px(top), px(w), px(h));
            }
        }
    }
    for (model, mut visibility) in &mut icons {
        let want = if model.0 == item { Visibility::Inherited } else { Visibility::Hidden };
        if *visibility != want {
            *visibility = want;
        }
    }
    for (line, mut t) in &mut lines {
        let s = match line {
            HudLine::ItemName => {
                let state = if gun.reloading() > 0.0 { "RELOADING   " } else if weapon.is_some() && ammo.loaded == 0 { "EMPTY  [R]   " } else { "" };
                format!("{state}{}", ammo_name(item))
            }
            HudLine::Carried => (ammo.loaded + ammo.spare).to_string(),
            HudLine::Extra if playing && mission.tnt > 0 => format!("DYNAMITE  x{}", mission.tnt),
            HudLine::Extra => String::new(),
            HudLine::Prompt => mission.prompt.clone().unwrap_or_default(),
            HudLine::Messages => mission.messages.iter().map(|(m, _)| m.as_str()).collect::<Vec<_>>().join("\n"),
            HudLine::Outcome => match mission.outcome {
                Some(Outcome::Complete) => format!(
                    "MISSION COMPLETE\nRating: {}\nTime {:.0}:{:02.0}\n\nEnter to play again",
                    mission.rating(),
                    (mission.elapsed / 60.0).floor(),
                    mission.elapsed % 60.0
                ),
                Some(Outcome::Failed) => "MISSION FAILED\n\nEnter to try again".to_owned(),
                None => String::new(),
            },
        };
        if t.0 != s {
            t.0 = s;
        }
    }
}

/// The objectives, ticked off, for the map and pause screens.
pub fn objectives_text(mission: &Mission) -> String {
    let mut s = mission.objectives.iter().map(|o| format!("{}  {}", if o.done { "[x]" } else { "[  ]" }, o.text)).collect::<Vec<_>>().join("\n");
    if mission.sub_bombs_placed > 0 {
        s += &format!("\nCharges on the submarine: {}/3", mission.sub_bombs_placed);
    }
    s
}
