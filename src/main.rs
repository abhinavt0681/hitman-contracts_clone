//! Hitman: Contracts clone — starting with C02-1, "The Bjarkhov Bomb".
//!
//! Reads the original game data from `data/Scenes` (copied from your own install).
//!
//! Controls (47): click to capture the mouse, Esc to release, WASD to walk, Shift to run,
//! E to interact (fiber wire from behind, take an outfit, pick up / place dynamite, exit),
//! left click to fire the silenced pistol.
//! Bodies are ragdolls (`ragdoll`, on Rapier).
//! F1 toggles the debug fly camera (only with `HITMAN_FREECAM=1`): right mouse / M to capture, WASD + E/Q, Shift fast,
//! scroll for speed.

mod ambience;
mod atmosphere;
mod audio;
mod character;
mod cutscene;
mod cloth;
mod collision;
mod flow;
mod footsteps;
mod game;
mod hud;
mod ladder;
mod map;
mod music;
mod nav;
mod player;
mod ragdoll;
mod shot;
mod snow;
mod tie;
mod view;

use bevy::{
    camera_controller::free_camera::FreeCameraPlugin,
    prelude::*,
};
use hitman_clone::glacier;
use std::path::PathBuf;

const DEFAULT_LEVEL: &str = "C02-1";

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Hitman Contracts clone — The Bjarkhov Bomb".into(), ..default() }),
            ..default()
        }))
        .add_plugins((atmosphere::AtmospherePlugin, ambience::AmbiencePlugin, footsteps::FootstepsPlugin, snow::SnowPlugin, cloth::ClothPlugin, nav::NavPlugin, music::MusicPlugin))
        .add_plugins((FreeCameraPlugin, flow::FlowPlugin, map::MapPlugin, player::PlayerPlugin, character::CharacterPlugin, game::GamePlugin, hud::HudPlugin, shot::ShotPlugin, ragdoll::RagdollPlugin, ladder::LadderPlugin, cutscene::CutscenePlugin, tie::TiePlugin))
        .init_resource::<collision::WorldCollision>()
        .add_systems(Startup, (load_level, audio::setup_audio))
        .add_systems(Update, audio::ambience)
        .run();
}

fn load_level(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut bindposes: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    mut collision: ResMut<collision::WorldCollision>,
) {
    let id = std::env::args().nth(1).unwrap_or_else(|| DEFAULT_LEVEL.to_owned());
    let scenes = std::env::var_os("HITMAN_SCENES").map_or_else(|| PathBuf::from("data/Scenes"), PathBuf::from);
    let mut level = match glacier::level::Level::load(&scenes, &id) {
        Ok(l) => l,
        Err(err) => {
            error!("failed to load level {id} from {}: {err:#}", scenes.display());
            std::process::exit(1);
        }
    };
    info!("{id}: {} entities", level.entities.len());
    let bounds = view::spawn_level(&level, &mut commands, &mut meshes, &mut materials, &mut images, &mut collision);
    // `HITMAN_PROBE="x,y,z;dx,dy,dz|…"` logs what collision rays hit (debugging).
    if let Ok(spec) = std::env::var("HITMAN_PROBE") {
        for ray in spec.split('|') {
            let v: Vec<f32> = ray.split([',', ';']).filter_map(|s| s.trim().parse().ok()).collect();
            if v.len() == 6 {
                let (o, d) = (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5]).normalize());
                match collision.raycast(o, d, 60.0) {
                    Some(h) => info!("probe {o} -> {d}: hit {:.2} m, {}", h.distance, level.entities.get(h.source as usize).map_or("?", |e| e.name.as_str())),
                    None => info!("probe {o} -> {d}: nothing"),
                }
            }
        }
    }

    info!("level bounds {:?}..{:?}, median {:?}", bounds.min, bounds.max, bounds.median);
    // 47's template entity marks where the mission starts.
    let start = level
        .entities
        .iter()
        .rev()
        .find(|e| e.name == "Hitman2" && e.position != [0.0; 3])
        .map_or(bounds.median, |e| view::glacier_to_bevy(e.position));
    let start = shot::env_vec3("HITMAN_START").unwrap_or(start);
    // The template position can be wedged in cargo (the real game places 47 by script);
    // stand him on the nearest clear floor instead.
    let start = collision.free_spot(start, 0.4, 8.0).unwrap_or(start);
    info!("47 spawn position: {:?}", start);
    let lib = character::AnimLibrary(std::mem::take(&mut level.anm));
    info!("{} animation sets, {} animations", lib.0.sets.len(), lib.0.sets.iter().map(|s| s.animations.len()).sum::<usize>());
    let mut cast = character::CharacterAssets {
        meshes: &mut meshes,
        materials: &mut materials,
        images: &mut images,
        bindposes: &mut bindposes,
        textures: &level.tex,
    };
    let actor_models = spawn_actors(&level, &lib, &mut commands, &mut cast);

    // 47 himself: the `Ground` child of his template carries the model.
    let hitman = level.entities.windows(2).find(|w| w[0].name == "Hitman2" && w[1].name == "Ground").map(|w| (&w[0], w[1].primitive));
    // The player entity faces −Z (Bevy's forward); the rig faces +Z, so turn it around.
    let model = hitman.and_then(|(e, prim)| {
        let mut t = view::instance_transform(e.rotation, [0.0; 3]);
        t.rotate_y(std::f32::consts::PI);
        level.prm.skinned_model(prim).ok().map(|m| (t, m))
    });
    let root = player::spawn(&mut commands, cast.meshes, cast.materials, start + Vec3::Y * 0.1, model.as_ref().map(|(t, _)| *t));
    if let Some((_, m)) = &model {
        character::attach(&mut commands, root, m, &mut cast);
        // His tie: its own object in the template, hung on his chest (`tie`).
        let tie = hitman.and_then(|_| {
            let e = level.entities.iter().find(|e| e.name == "Tie" && e.type_id == 0x2000ff)?;
            // Its offset from the 47 template it belongs to (the nearest `Hitman2`).
            let d = |a: [f32; 3]| (a[0] - e.position[0]).powi(2) + (a[1] - e.position[1]).powi(2) + (a[2] - e.position[2]).powi(2);
            let template = level.entities.iter().filter(|t| t.name == "Hitman2" && t.position != [0.0; 3]).min_by(|a, b| d(a.position).total_cmp(&d(b.position)))?;
            let sm = level.prm.model(e.primitive).ok()?.submeshes.into_iter().find(|s| s.in_lod0() && !s.is_proxy())?;
            let offset = [e.position[0] - template.position[0], e.position[1] - template.position[1], e.position[2] - template.position[2]];
            let (top, bottom) = sm.vertices.iter().fold((f32::MIN, f32::MAX), |(t, b), v| (t.max(v.pos[1]), b.min(v.pos[1])));
            let mean_of = |sel: &dyn Fn(f32) -> bool| {
                let vs: Vec<Vec3> = sm.vertices.iter().filter(|v| sel(v.pos[1])).map(|v| Vec3::from(view::swap_xz(v.pos))).collect();
                vs.iter().copied().sum::<Vec3>() / vs.len().max(1) as f32
            };
            let (knot, tip) = (mean_of(&|y| y > top - 2.0), mean_of(&|y| y < bottom + 2.0));
            // The original simulates the tie: it's authored jutting out at 45° from the knot,
            // and gravity and three collision planes lay it on his shirt. Hang it at rest
            // instead, from the knot: as high as the template places it, as far in front of his
            // centre line, down and a little forward over his chest, its flat face to the front.
            // His front, side and up in the model's space (centimetres), from the model's own
            // transform (the player faces −Z).
            let to_model = model.as_ref()?.0.to_matrix().inverse();
            let (front, side, up) = [Vec3::NEG_Z, Vec3::X, Vec3::Y].map(|v| to_model.transform_vector3(v).normalize()).into();
            let knot_in_model = up * (offset[1] + knot.y) + front * (knot.with_y(0.0).length() + KNOT_OUT);
            let frame = |length: Vec3, width: Vec3| {
                let a = length.normalize();
                let b = (width - a * width.dot(a)).normalize();
                Mat3::from_cols(a, b, a.cross(b))
            };
            // In the mesh the tie is 6 cm wide along its (swapped) Z.
            let from = frame(tip - knot, Vec3::Z);
            let to = frame(-up + front * TIE_LEAN, side);
            let in_model = Mat4::from_translation(knot_in_model) * Mat4::from_mat3(to * from.transpose()) * Mat4::from_translation(-knot);
            let material = view::build_material(&level.tex, view::MaterialKey { texture: sm.texture, blended: sm.blended }, cast.images);
            Some(tie::TieModel { mesh: cast.meshes.add(view::build_mesh(&sm)), material: cast.materials.add(material), in_model, knot })
        });
        match tie {
            Some(t) => commands.insert_resource(t),
            None => warn!("47's tie not found in the level"),
        }
        if let Some(set) = lib.set_index("Hero.anm") {
            let idle = lib.first_of(set, &["Stand_Do_Nothing", "Stand_Relaxed"]).unwrap_or_default();
            commands.entity(root).insert((character::Gait::new(&lib, set, idle.clone()), character::Animator::new(set, idle)));
        }
    }
    let mut wardrobe = build_wardrobe(&level);
    wardrobe.0.extend(actor_models);
    // His own suit, to change back into from the bundle he leaves (`game::bundle`).
    if let Some((_, m)) = &model {
        wardrobe.0.insert(game::bundle::SUIT.to_owned(), m.clone());
    }
    // The folded suit itself: `Bundle_01` under `CB_Hitman2`.
    if let Some(b) = level.entities.iter().find(|e| e.name == "Bundle_01" && e.primitive != 0) {
        if let Ok(m) = level.prm.model(b.primitive) {
            let parts = m
                .submeshes
                .iter()
                .filter(|s| s.in_lod0() && !s.is_proxy())
                .map(|s| (meshes.add(view::build_mesh(s)), materials.add(view::build_material(&level.tex, view::MaterialKey { texture: s.texture, blended: s.blended }, &mut images))))
                .collect();
            commands.insert_resource(game::bundle::SuitBundleModel { parts, transform: view::instance_transform(b.rotation, [0.0; 3]) });
        }
    }
    info!("wardrobe: {:?}", wardrobe.0.keys().collect::<Vec<_>>());
    commands.insert_resource(wardrobe);
    commands.insert_resource(find_anchors(&level));
    commands.insert_resource(game::Mission::new(&level.strings));
    commands.insert_resource(lib);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(start + Vec3::new(0.0, 2.0, 3.0)),
        player::orbit_camera(),
        atmosphere::camera_bundle(),
        IsDefaultUiCamera,
        bevy::audio::SpatialListener::new(0.3),
    ));
    atmosphere::spawn(&level, &bounds, &collision, &mut commands);
    ambience::spawn(&level, &bounds.entity_world, &mut commands);
    commands.insert_resource(ladder::build(&bounds.ladder_guides, &collision));
    commands.insert_resource(bounds);
    commands.insert_resource(view::EntityNames(level.entities.iter().map(|e| e.name.clone()).collect()));
    commands.insert_resource(game::Text(std::mem::take(&mut level.strings)));
    commands.insert_resource(game::Textures(std::mem::take(&mut level.tex)));
}

/// Outfit and stripped-body models, from the `_HMas` / `_NUDE` template actors.
fn build_wardrobe(level: &glacier::level::Level) -> game::Wardrobe {
    let mut out = std::collections::HashMap::new();
    for w in level.entities.windows(2) {
        let (actor, ground) = (&w[0], &w[1]);
        if ground.name == "Ground" && (actor.name.ends_with("_HMas") || actor.name.ends_with("_NUDE")) && !out.contains_key(&actor.name) {
            if let Ok(m) = level.prm.skinned_model(ground.primitive) {
                out.insert(actor.name.clone(), m);
            }
        }
    }
    game::Wardrobe(out)
}

/// Engine class of trigger boxes (`…_triggerbox`, `Box_0N`, `RadiationWarning`).
const TRIGGER_CLASS: u32 = 0x2000e1;

/// Mission locations, from GMS entity names.
fn find_anchors(level: &glacier::level::Level) -> game::Anchors {
    let at = |name: &str| level.entities.iter().find(|e| e.name == name && e.position != [0.0; 3]).map(|e| view::glacier_to_bevy(e.position));
    let tnt = level.entities.iter().filter(|e| e.name.starts_with("TNT_") && e.position != [0.0; 3]).map(|e| (view::glacier_to_bevy(e.position), false)).collect();
    // The map shows three bomb icons over the submarine; the plantable spots are the
    // `Box_0N` trigger boxes on the deck beneath them.
    let boxes: Vec<Vec3> = level
        .entities
        .iter()
        .filter(|e| e.name.starts_with("Box_0") && e.type_id == TRIGGER_CLASS && e.position != [0.0; 3])
        .map(|e| view::glacier_to_bevy(e.position))
        .collect();
    let bomb_sites: Vec<(Vec3, bool)> = level
        .entities
        .iter()
        .filter(|e| e.name.starts_with("bombicon_") && e.position != [0.0; 3])
        .map(|e| {
            let icon = view::glacier_to_bevy(e.position);
            let spot = boxes.iter().copied().filter(|b| b.xz().distance(icon.xz()) < 6.0).min_by(|a, b| a.xz().distance(icon.xz()).total_cmp(&b.xz().distance(icon.xz())));
            (spot.unwrap_or_else(|| { warn!("no deck trigger under {}", e.name); icon }), false)
        })
        .collect();
    // Every anchor comes from a named level entity; a missing one is reported, not guessed.
    let require = |names: &[&str]| -> Vec3 {
        names.iter().find_map(|n| at(n)).unwrap_or_else(|| {
            warn!("mission anchor {names:?} not found in level data; that interaction is disabled");
            Vec3::splat(f32::MAX)
        })
    };
    let soup_pot = require(&["SoupCookLoc", "Poison_actionbox"]);
    let radiation_suit = require(&["cb_BombWorker_01"]);
    let bjarkhov_office = require(&["BjarkhovOffice", "CommanderBjarkhov_01"]);
    let bjarkhov_desk = require(&["GreetFuchsLoc"]);
    let ship_gangway = require(&["FuchsComing_triggerbox"]);
    info!("anchors: suit {radiation_suit:.1?}, gangway {ship_gangway:.1?}, office {bjarkhov_office:.1?}, frisk {:.1?}", ["FriskHitman_triggerbox", "FriskHitman_triggerbox#01"].iter().filter_map(|n| at(n)).collect::<Vec<_>>());
    game::Anchors {
        tnt,
        bomb_sites,
        soup_pot,
        radiation_suit,
        exit: at("Exitbox").or_else(|| at("PlaneLoc")).unwrap_or(Vec3::splat(f32::MAX)),
        // The irradiated sub interior: around the bomb site, bounded by the radiation warning.
        // The irradiated hull: centred on the `SubRadiactionBox` trigger (sic), reaching out to the
        // radiation warning at the gangway.
        sub: {
            let core = require(&["SubRadiactionBox"]);
            (core, at("RadiationWarning").map_or(14.0, |r| r.xz().distance(core.xz()).max(8.0)))
        },
        bjarkhov_office,
        bjarkhov_desk,
        ship_gangway,
        frisk: ["FriskHitman_triggerbox", "FriskHitman_triggerbox#01"].iter().filter_map(|n| at(n)).collect(),
    }
}

/// Places every scripted character (guards, workers, Bjarkhov…) at its GMS position. An
/// actor entity is followed by a `Ground` child whose primitive is its skinned model;
/// `_HMas` (47's disguise outfits) and `_NUDE` variants are templates, not people.
/// Returns each actor's model by name, for the wardrobe.
/// An actor's patrol route from its `WayPoints` path: node positions plus a pause at nodes the
/// level designers named `…Wait…`.
fn patrol_route(level: &glacier::level::Level, actor: &glacier::gms::Entity) -> Vec<(Vec3, f32)> {
    let Some(path) = actor.props.as_ref().and_then(|p| p.waypoints).and_then(|i| level.entities.get(i)) else { return Vec::new() };
    path.path
        .iter()
        .filter_map(|&n| level.entities.get(n))
        .map(|n| (view::glacier_to_bevy(n.position), if n.name.contains("Wait") { 4.0 } else { 0.0 }))
        .collect()
}

/// 47's tie at rest: the knot this far (cm) further out than the template has it, and the tie
/// leaning forward by this much per unit of drop, so it lies on his shirt instead of inside it.
const KNOT_OUT: f32 = 2.0;
const TIE_LEAN: f32 = 0.44;

fn spawn_actors(
    level: &glacier::level::Level,
    lib: &character::AnimLibrary,
    commands: &mut Commands,
    cast: &mut character::CharacterAssets,
) -> std::collections::HashMap<String, glacier::skin::SkinnedModel> {
    let mut by_actor = std::collections::HashMap::new();
    let mut models = std::collections::HashMap::new();
    let mut spawned = 0;
    for w in level.entities.windows(2) {
        let (actor, ground) = (&w[0], &w[1]);
        if ground.name != "Ground" || level.prm.kind_at(ground.primitive) != Some(1) || actor.position == [0.0; 3] {
            continue;
        }
        if actor.name.ends_with("_HMas") || actor.name.ends_with("_NUDE") || actor.name.starts_with("Hitman") {
            continue;
        }
        let model = models.entry(ground.primitive).or_insert_with(|| level.prm.skinned_model(ground.primitive).map_err(|e| warn!("{}: {e:#}", actor.name)).ok());
        let Some(model) = model else { continue };
        by_actor.insert(actor.name.clone(), model.clone());
        let root = commands.spawn((view::instance_transform(actor.rotation, actor.position), Visibility::default(), Name::new(actor.name.clone()))).id();
        character::attach(commands, root, model, cast);
        let route = patrol_route(level, actor);
        if !route.is_empty() {
            info!("{} patrols {} nodes", actor.name, route.len());
        }

        if let Some((set, clip)) = idle_for(lib, &actor.name) {
            // Moving characters walk and run with the regular male clips.
            let gait_set = lib.set_index("Reg_Male.anm").unwrap_or(set);
            let mut gait = character::Gait::new(lib, gait_set, clip.clone());
            gait.idle = clip.clone();
            commands.entity(root).insert((character::Animator::new(set, clip), gait));
        }
        // Stand upright, facing where the level placed them (P-space forward is −X).
        let t = view::instance_transform(actor.rotation, actor.position);
        let fwd = (t.rotation * Vec3::NEG_X).with_y(0.0).normalize_or(Vec3::NEG_Z);
        // `WarpToPathPoint=1` (the default) starts a patroller on its route.
        let warp = actor.props.as_ref().is_none_or(|p| p.int_props.get("WarpToPathPoint").copied().unwrap_or(1) != 0);
        let start = match route.first() {
            Some((p, _)) if warp => *p,
            _ => t.translation,
        };
        commands.entity(root).insert((
            Transform { rotation: game::facing(fwd), translation: start, ..t },
            game::Actor::new(&actor.name),
            game::Patrol::new(route),
            game::Steering::default(),
        ));
        spawned += 1;
    }
    info!("spawned {spawned} characters from {} models", models.len());
    by_actor
}

/// Animation set and idle clip for an actor. The real assignment lives in each actor's
/// property blob (not decoded yet); names make it unambiguous for this level.
fn idle_for(lib: &character::AnimLibrary, actor: &str) -> Option<(usize, String)> {
    let specific = [
        ("Bjarkhov", "Bjarkov.anm"),
        ("BombWorker", "Bomb_Builder.anm"),
        ("FabianFuchs", "FabianFuchs.anm"),
        ("ServiceWorker", "Worker_C2-1.anm"),
        ("Chef", "Chef_C2-1.anm"),
    ];
    let idles = ["Stand_Freezing", "Stand_Relaxed", "Stand_Do_Nothing", "Stand_Tense", "Stand"];
    if let Some(set) = specific.iter().find(|(k, _)| actor.contains(k)).and_then(|(_, f)| lib.set_index(f)) {
        if let Some(clip) = lib.first_of(set, &idles) {
            return Some((set, clip));
        }
    }
    let set = lib.set_index("Reg_Male.anm")?;
    Some((set, lib.first_of(set, &idles)?))
}
