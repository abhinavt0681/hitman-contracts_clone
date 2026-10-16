//! The blizzard, done on the GPU (`shaders/snow.wesl`, `shaders/veil.wesl`).
//!
//! Three layers, after the original's look in motion: tens of thousands of thin streaks
//! driven almost sideways by the wind, big sheets of blowing snow billowing across open
//! ground, and powder skimming along the surface. All of it rides one accumulated wind
//! offset, so squalls (`atmosphere::Gust`) speed everything up without anything jumping.
//!
//! A top-down height map of the level (the highest surface over each square metre) tells the
//! shaders where the sky is open: snow never falls inside buildings, under the plane's wing
//! or through a roof, and you still see it outside a window. The nearest lamps light the
//! flakes and sheets around them.

use crate::{
    atmosphere::{fog_color, Exposure, Gust, Lamps, FOG_DENSITY},
    collision::WorldCollision,
};
use bevy::{
    asset::{embedded_asset, RenderAssetUsages},
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
};

/// The wind between squalls (m/s): hard from one side, barely falling.
pub const WIND: Vec3 = Vec3::new(11.0, -2.4, 5.0);
/// How much faster a full squall blows.
const GUST_WIND: f32 = 1.6;
const STREAKS: usize = 60_000;
/// Half size of the box of streaks kept around the camera.
const STREAK_BOX: Vec3 = Vec3::new(20.0, 10.0, 20.0);
/// How long the eye "exposes" a moving flake: the length of the streaks.
const SHUTTER: f32 = 0.045;
const SHEETS: usize = 70;
const DRIFTS: usize = 110;
/// Height map resolution (m).
const MAP_CELL: f32 = 1.0;
const MAX_LAMPS: usize = 16;

pub struct SnowPlugin;

impl Plugin for SnowPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/snow.wesl");
        embedded_asset!(app, "shaders/veil.wesl");
        app.add_plugins((MaterialPlugin::<StreakMaterial>::default(), MaterialPlugin::<VeilMaterial>::default()))
            .init_resource::<WindTravel>()
            .add_systems(PostStartup, spawn_snow)
            .add_systems(Update, drive_snow);
    }
}

/// Uniforms shared by both snow shaders; field meanings are documented in the shaders.
#[derive(ShaderType, Clone, Debug, Default)]
pub struct SnowParams {
    travel: Vec4,
    wind: Vec4,
    half_box: Vec4,
    fog: Vec4,
    tint: Vec4,
    map: Vec4,
    lamps: Vec4,
    lamp_pos: [Vec4; MAX_LAMPS],
    lamp_col: [Vec4; MAX_LAMPS],
}

macro_rules! snow_material {
    ($name:ident, $shader:literal) => {
        #[derive(Asset, TypePath, AsBindGroup, Clone)]
        pub struct $name {
            #[uniform(0)]
            params: SnowParams,
            #[texture(1, sample_type = "float", filterable = false)]
            height_map: Handle<Image>,
        }

        impl Material for $name {
            fn vertex_shader() -> ShaderRef {
                concat!("embedded://hitman_clone/shaders/", $shader).into()
            }
            fn fragment_shader() -> ShaderRef {
                concat!("embedded://hitman_clone/shaders/", $shader).into()
            }
            fn alpha_mode(&self) -> AlphaMode {
                AlphaMode::Blend
            }
            fn enable_prepass() -> bool {
                false
            }
            fn enable_shadows() -> bool {
                false
            }
            fn specialize(
                _pipeline: &MaterialPipeline,
                descriptor: &mut RenderPipelineDescriptor,
                layout: &MeshVertexBufferLayoutRef,
                _key: MaterialPipelineKey<Self>,
            ) -> Result<(), SpecializedMeshPipelineError> {
                descriptor.primitive.cull_mode = None;
                // Seed, random parameters and quad corner ride in the position, normal and UV slots.
                descriptor.vertex.buffers = vec![layout.0.get_layout(&[
                    Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                    Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
                    Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
                ])?];
                Ok(())
            }
        }
    };
}

snow_material!(StreakMaterial, "snow.wesl");
snow_material!(VeilMaterial, "veil.wesl");

/// How far the wind has carried the air since the start (m), and the clock.
#[derive(Resource, Default)]
struct WindTravel(Vec3, f32);

#[derive(Component)]
struct Streaks(Handle<StreakMaterial>);

#[derive(Component)]
struct Veils {
    material: Handle<VeilMaterial>,
    ground: bool,
}

/// `n` quads, each with its own random seed position and parameters.
fn quads(n: usize, mut rand: impl FnMut() -> f32) -> Mesh {
    let mut seeds = Vec::with_capacity(n * 4);
    let mut params = Vec::with_capacity(n * 4);
    let mut corners = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);
    for i in 0..n as u32 {
        let seed = [rand(), rand(), rand()];
        let param = [rand(), rand(), rand()];
        for c in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            seeds.push(seed);
            params.push(param);
            corners.push(c);
        }
        indices.extend([0, 1, 2, 0, 2, 3].map(|k| i * 4 + k));
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, seeds)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, params)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, corners)
        .with_inserted_indices(Indices::U32(indices))
}

fn spawn_snow(
    mut commands: Commands,
    world: Res<WorldCollision>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut streaks: ResMut<Assets<StreakMaterial>>,
    mut veils: ResMut<Assets<VeilMaterial>>,
) {
    let t0 = std::time::Instant::now();
    let (min, max) = world.bounds();
    let origin = min.xz().floor();
    let w = ((max.x - origin.x) / MAP_CELL).ceil() as usize + 1;
    let h = ((max.z - origin.y) / MAP_CELL).ceil() as usize + 1;
    let tops = world.top_heights(origin, MAP_CELL, w, h);
    let bytes: Vec<u8> = tops.iter().flat_map(|v| v.to_le_bytes()).collect();
    let map = images.add(Image::new(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        bytes,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    ));
    info!("snow: {w}x{h} height map in {:.0} ms", t0.elapsed().as_secs_f32() * 1000.0);

    let params = SnowParams { map: Vec4::new(origin.x, origin.y, MAP_CELL, 0.0), ..default() };
    let mut rng = 0x2545_f491_u32;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32
    };
    let streak = streaks.add(StreakMaterial { params: params.clone(), height_map: map.clone() });
    commands.spawn((
        Mesh3d(meshes.add(quads(STREAKS, &mut rand))),
        MeshMaterial3d(streak.clone()),
        Streaks(streak),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        Name::new("Blizzard streaks"),
    ));
    for (n, ground) in [(SHEETS, false), (DRIFTS, true)] {
        let material = veils.add(VeilMaterial { params: params.clone(), height_map: map.clone() });
        commands.spawn((
            Mesh3d(meshes.add(quads(n, &mut rand))),
            MeshMaterial3d(material.clone()),
            Veils { material, ground },
            Transform::default(),
            NoFrustumCulling,
            NotShadowCaster,
            Name::new(if ground { "Blowing powder" } else { "Snow sheets" }),
        ));
    }
}

fn drive_snow(
    time: Res<Time>,
    gust: Res<Gust>,
    exposure: Res<Exposure>,
    lamps: Option<Res<Lamps>>,
    mut travel: ResMut<WindTravel>,
    camera: Query<&GlobalTransform, With<crate::player::OrbitCamera>>,
    mut streak_q: Query<(&Streaks, &mut Visibility), Without<Veils>>,
    mut veil_q: Query<(&Veils, &mut Visibility), Without<Streaks>>,
    mut streak_materials: ResMut<Assets<StreakMaterial>>,
    mut veil_materials: ResMut<Assets<VeilMaterial>>,
) {
    let g = gust.strength;
    let wind = WIND * (1.0 + GUST_WIND * g);
    travel.0 += wind * time.delta_secs();
    travel.1 += time.delta_secs();
    let Ok(cam) = camera.single() else { return };
    let eye = cam.translation();

    // The closest lamps, brightest first among equals.
    let mut lamp_pos = [Vec4::ZERO; MAX_LAMPS];
    let mut lamp_col = [Vec4::ZERO; MAX_LAMPS];
    let mut count = 0;
    if let Some(lamps) = &lamps {
        let mut near: Vec<_> = lamps.0.iter().filter(|l| l.pos.distance(eye) < STREAK_BOX.x * 2.5 + l.range).collect();
        near.sort_by(|a, b| a.pos.distance_squared(eye).total_cmp(&b.pos.distance_squared(eye)));
        for (i, l) in near.iter().take(MAX_LAMPS).enumerate() {
            lamp_pos[i] = l.pos.extend(l.range);
            lamp_col[i] = l.snow_glow.to_vec3().extend(1.0);
            count = i + 1;
        }
    }
    let felt = if exposure.indoors { 0.0 } else { g };
    let fog = fog_color(felt).to_linear().to_vec3().extend(FOG_DENSITY * (1.0 + 1.3 * felt));
    let base = |half_box: Vec4, tint: Vec4, wind_w: f32| SnowParams {
        travel: travel.0.extend(travel.1),
        wind: wind.extend(wind_w),
        half_box,
        fog,
        tint,
        map: Vec4::ZERO,
        lamps: Vec4::new(count as f32, 0.0, 0.0, 0.0),
        lamp_pos,
        lamp_col,
    };

    // `HITMAN_NO_SNOW` hides the blizzard for pixel comparisons in tests.
    let vis = if std::env::var_os("HITMAN_NO_SNOW").is_some() { Visibility::Hidden } else { Visibility::Inherited };
    // `HITMAN_SNOW_LAYERS=streaks,sheets,powder` shows only those layers (profiling).
    let layers = std::env::var("HITMAN_SNOW_LAYERS").ok();
    let shown = |layer: &str, vis: Visibility| match &layers {
        Some(l) if !l.split(',').any(|x| x == layer) => Visibility::Hidden,
        _ => vis,
    };
    for (s, mut v) in &mut streak_q {
        *v = shown("streaks", vis);
        if let Some(mut m) = streak_materials.get_mut(&s.0) {
            let map = m.params.map;
            m.params = base(STREAK_BOX.extend(0.55 + 0.45 * g), Vec4::new(0.30, 0.38, 0.42, 0.45 + 0.15 * g), SHUTTER);
            m.params.map = map;
        }
    }
    for (veil, mut v) in &mut veil_q {
        *v = shown(if veil.ground { "powder" } else { "sheets" }, vis);
        if let Some(mut m) = veil_materials.get_mut(&veil.material) {
            let map = m.params.map;
            m.params = if veil.ground {
                base(Vec4::new(30.0, 1.3, 30.0, 1.0), Vec4::new(0.20, 0.27, 0.30, 0.30 + 0.35 * g), 0.0)
            } else {
                base(Vec4::new(45.0, 8.0, 45.0, 0.0), Vec4::new(0.14, 0.20, 0.22, 0.16 + 0.40 * g), 0.0)
            };
            m.params.map = map;
        }
    }
}
