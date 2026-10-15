//! Night and blizzard: the look of the original mission (see `docs/ATMOSPHERE.md`).
//!
//! Lights sit where the level places its `Omni*`/`Fspot*` light entities; the data carries no
//! colour or radius for them, so those follow the reference screenshots: sodium pools
//! outdoors, pale-green fluorescent indoors. "Indoors" means level geometry overhead: the
//! `InsideBound_*` volumes are visibility regions that also cover open ground. The cargo
//! plane's named navigation lights blink red and white. The snow itself is `snow.rs`.
//!
//! The weather and the lamps also matter to the guards: `Gust` is a squall that thickens the
//! fog and drowns out footsteps for a few seconds, and `Exposure` says how lit 47 stands.

use crate::{
    audio::{WindLoop, WIND_VOLUME},
    collision::WorldCollision,
    player::Player,
    view::LevelBounds,
};
use bevy::{audio::{AudioSink, AudioSinkPlayback, Volume}, pbr::{DistanceFog, FogFalloff}, post_process::bloom::Bloom, prelude::*};
use hitman_clone::glacier::level::Level;

/// Night sky and fog: a dark teal that swallows everything past a few dozen metres.
pub const NIGHT: Color = Color::srgb(0.035, 0.065, 0.08);
pub const FOG_DENSITY: f32 = 0.028;
/// What the fog turns to in a full squall: a grey-teal whiteout lit by the lamps it swallows.
const WHITEOUT: Color = Color::srgb(0.15, 0.22, 0.245);
const LIGHT_CLASSES: [u32; 2] = [0x800023, 0x800024];
const SODIUM: Color = Color::srgb(1.0, 0.78, 0.5);
const FLUORESCENT: Color = Color::srgb(0.78, 1.0, 0.82);

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(NIGHT))
            .insert_resource(GlobalAmbientLight { color: Color::srgb(0.55, 0.75, 0.85), brightness: 120.0, ..default() })
            .init_resource::<Gust>()
            .init_resource::<Exposure>()
            .add_systems(Update, (gusts, blink, expose));
    }
}

/// Camera components for the night look.
pub fn camera_bundle() -> impl Bundle {
    (DistanceFog { color: NIGHT, falloff: FogFalloff::Exponential { density: FOG_DENSITY }, ..default() }, Bloom::NATURAL)
}

/// A squall: `strength` ramps 0 → 1 → 0 over a few seconds, every half minute or so.
#[derive(Resource)]
pub struct Gust {
    pub strength: f32,
    /// Seconds into the current calm + squall cycle, and its length parts.
    t: f32,
    calm: f32,
    hold: f32,
    seed: u32,
}

impl Default for Gust {
    fn default() -> Self {
        Gust { strength: 0.0, t: 0.0, calm: 20.0, hold: 5.0, seed: 0x9e37_79b9 }
    }
}

const GUST_RISE: f32 = 1.5;
const GUST_FALL: f32 = 2.5;

/// How visible 47 is: `light` 0 (dark) … 1 (under a lamp); `gust` the squall strength where he
/// stands (0 under a roof).
#[derive(Resource, Default)]
pub struct Exposure {
    pub light: f32,
    pub gust: f32,
    pub indoors: bool,
}

/// The level's lamps: they light 47 up for the guards, and the snow around them.
#[derive(Resource, Default)]
pub struct Lamps(pub Vec<Lamp>);

pub struct Lamp {
    pub pos: Vec3,
    pub range: f32,
    /// How brightly snow glows in this lamp's light.
    pub snow_glow: LinearRgba,
}

/// Overcast moonlight: never quite pitch black.
const MOON: f32 = 0.12;

/// A roof within this height above a point makes it indoors.
const ROOF_CHECK: f32 = 7.0;

fn indoors(world: &WorldCollision, p: Vec3) -> bool {
    world.raycast(p + Vec3::Y * 0.2, Vec3::Y, ROOF_CHECK).is_some()
}

/// Blinking navigation light: on for `on` seconds every `period`, offset by `phase`.
#[derive(Component)]
struct Beacon {
    period: f32,
    on: f32,
    phase: f32,
    intensity: f32,
}

pub fn spawn(
    level: &Level,
    bounds: &LevelBounds,
    world: &WorldCollision,
    commands: &mut Commands,
) {
    // Faint cold moonlight through the overcast; it only shapes silhouettes.
    commands.spawn((
        DirectionalLight { illuminance: 250.0, color: Color::srgb(0.6, 0.75, 1.0), shadow_maps_enabled: std::env::var_os("HITMAN_SHADOWS").is_some(), ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, -0.9, 0.0)),
    ));

    let mut lamps = 0;
    let mut lit = Lamps::default();
    for (i, e) in level.entities.iter().enumerate() {
        if !LIGHT_CLASSES.contains(&e.type_id) || e.position == [0.0; 3] {
            continue;
        }
        let p = bounds.entity_world[i];
        let inside = indoors(world, p);
        commands.spawn((
            PointLight {
                color: if inside { FLUORESCENT } else { SODIUM },
                intensity: if inside { 12_000.0 } else { 140_000.0 },
                range: if inside { 9.0 } else { 14.0 },
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(p),
            Name::new(e.name.clone()),
        ));
        let color = if inside { FLUORESCENT } else { SODIUM };
        lit.0.push(Lamp { pos: p, range: if inside { 9.0 } else { 14.0 }, snow_glow: color.to_linear() * if inside { 0.5 } else { 1.4 } });
        lamps += 1;
    }

    // The cargo plane's navigation lights: steady red, white strobes.
    let mut beacons = 0;
    for (i, e) in level.entities.iter().enumerate() {
        let (color, period, on, intensity) = match e.name.as_str() {
            "RedTailLight" | "RedFrontLight" | "RedPosLight" => (Color::srgb(1.0, 0.08, 0.05), 1.6, 0.9, 150_000.0),
            "TailStrobeLight" | "LeftStrobeLight" | "RightStrobeLight" => (Color::WHITE, 1.3, 0.08, 900_000.0),
            _ => continue,
        };
        commands.spawn((
            PointLight { color, intensity, range: 18.0, ..default() },
            Transform::from_translation(bounds.entity_world[i]),
            Beacon { period, on, phase: i as f32 * 0.37, intensity },
            Name::new(e.name.clone()),
        ));
        beacons += 1;
    }
    info!("atmosphere: {lamps} lamps, {beacons} beacons");
    commands.insert_resource(lit);
}

fn blink(time: Res<Time>, mut beacons: Query<(&Beacon, &mut PointLight)>) {
    let t = time.elapsed_secs();
    for (b, mut light) in &mut beacons {
        let on = (t + b.phase).rem_euclid(b.period) < b.on;
        light.intensity = if on { b.intensity } else { 0.0 };
    }
}

/// Drives the squall cycle, and with it the fog, the wind's loudness and the snow's speed.
fn gusts(
    time: Res<Time>,
    mut gust: ResMut<Gust>,
    exposure: Res<Exposure>,
    mut fog: Query<&mut DistanceFog>,
    mut clear: ResMut<ClearColor>,
    mut wind: Query<&mut AudioSink, With<WindLoop>>,
) {
    let g = &mut *gust;
    g.t += time.delta_secs();
    let s = g.t - g.calm;
    g.strength = if s < 0.0 {
        0.0
    } else if s < GUST_RISE {
        s / GUST_RISE
    } else if s < GUST_RISE + g.hold {
        1.0
    } else if s < GUST_RISE + g.hold + GUST_FALL {
        1.0 - (s - GUST_RISE - g.hold) / GUST_FALL
    } else {
        // Next cycle: 18–40 s of calm, a 3–8 s squall.
        g.seed ^= g.seed << 13;
        g.seed ^= g.seed >> 17;
        g.seed ^= g.seed << 5;
        let r = g.seed as f32 / u32::MAX as f32;
        g.t = 0.0;
        g.calm = 18.0 + 22.0 * r;
        g.hold = 3.0 + 5.0 * (r * 7.0).fract();
        0.0
    };
    // `HITMAN_GUST=0.0..1.0` holds the squall at that strength, for screenshots.
    if let Some(v) = std::env::var("HITMAN_GUST").ok().and_then(|v| v.parse::<f32>().ok()) {
        g.strength = v;
    }
    // Under a roof the squall is only heard, muffled.
    let felt = if exposure.indoors { 0.0 } else { g.strength };
    let color = fog_color(felt);
    for mut f in &mut fog {
        f.falloff = FogFalloff::Exponential { density: FOG_DENSITY * (1.0 + 1.3 * felt) };
        f.color = color;
    }
    clear.0 = color;
    for mut sink in &mut wind {
        sink.set_volume(Volume::Linear(WIND_VOLUME * (1.0 + if exposure.indoors { 0.3 } else { 1.0 } * g.strength)));
    }
}

/// Fog colour for a squall of `strength` where the camera is.
pub fn fog_color(strength: f32) -> Color {
    NIGHT.mix(&WHITEOUT, 0.85 * strength)
}

/// How lit 47 is, and whether the squall reaches him.
fn expose(
    world: Res<WorldCollision>,
    gust: Res<Gust>,
    lamps: Option<Res<Lamps>>,
    player: Query<&GlobalTransform, With<Player>>,
    mut exposure: ResMut<Exposure>,
) {
    let Ok(pt) = player.single() else { return };
    let chest = pt.translation() + Vec3::Y * 1.2;
    let mut light = MOON;
    for &Lamp { pos: p, range, .. } in lamps.iter().flat_map(|l| l.0.iter()) {
        let d = p.distance(chest);
        if d >= range {
            continue;
        }
        let l = (1.0 - d / range).powf(0.7);
        if l > light && lamp_reaches(&world, p, chest) {
            light = l;
        }
    }
    exposure.indoors = indoors(&world, pt.translation());
    exposure.light = light.min(1.0);
    exposure.gust = if exposure.indoors { 0.0 } else { gust.strength };
}

/// Clear line from a lamp to a point (the lamp's own fitting may sit right at a surface).
fn lamp_reaches(world: &WorldCollision, from: Vec3, to: Vec3) -> bool {
    let d = to - from;
    let len = d.length();
    world.raycast(from + d / len * 0.3, d, len - 0.3).is_none_or(|h| h.distance > len - 0.6)
}
