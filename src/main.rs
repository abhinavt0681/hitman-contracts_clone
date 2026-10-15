//! Hitman: Contracts clone — starting with C02-1, "The Bjarkhov Bomb".
//!
//! Reads the original game data from `data/Scenes` (copied from your own install).
//!
//! Controls (47): click to capture the mouse, Esc to release, WASD to walk, Shift to run,
//! E to interact (fiber wire from behind, take an outfit, pick up / place dynamite, exit),
//! left click to fire the silenced pistol.
//! F1 toggles the debug fly camera: right mouse / M to capture, WASD + E/Q, Shift fast,
//! scroll for speed.

mod audio;
mod character;
mod collision;
mod game;
mod hud;
mod player;
mod shot;
mod view;

use bevy::{
    camera_controller::free_camera::FreeCameraPlugin,
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
};
use hitman_clone::glacier;
use std::path::PathBuf;

const DEFAULT_LEVEL: &str = "C02-1";

fn main() {
    App::new()
        .insert_resource(ClearColor(Color::srgb(0.62, 0.67, 0.74)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.85, 0.9, 1.0), brightness: 600.0, ..default() })
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window { title: "Hitman Contracts clone — The Bjarkhov Bomb".into(), ..default() }),
            ..default()
        }))
        .add_plugins((FreeCameraPlugin, player::PlayerPlugin, character::CharacterPlugin, game::GamePlugin, hud::HudPlugin, shot::ShotPlugin))
        .init_resource::<collision::WorldCollision>()
        .add_systems(Startup, (load_level, audio::setup_audio))
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
        if let Some(set) = lib.set_index("Hero.anm") {
            let idle = lib.first_of(set, &["Stand_Do_Nothing", "Stand_Relaxed"]).unwrap_or_default();
            commands.entity(root).insert((character::Gait::new(&lib, set, idle.clone()), character::Animator::new(set, idle)));
        }
    }
    let mut wardrobe = build_wardrobe(&level);
    wardrobe.0.extend(actor_models);
    info!("wardrobe: {:?}", wardrobe.0.keys().collect::<Vec<_>>());
    commands.insert_resource(wardrobe);
    commands.insert_resource(find_anchors(&level));
    commands.insert_resource(game::Mission::new(&level.strings));
    commands.insert_resource(lib);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(start + Vec3::new(0.0, 2.0, 3.0)),
        player::orbit_camera(),
        DistanceFog {
            color: Color::srgb(0.62, 0.67, 0.74),
            falloff: FogFalloff::Linear { start: 60.0, end: 400.0 },
            ..default()
        },
    ));
    // Overcast Siberian daylight. The 2004 game had baked lighting only, and real-time
    // shadows from its giant fog cards and glacier walls darken everything, so they're opt-in.
    commands.spawn((
        DirectionalLight { illuminance: 6000.0, shadow_maps_enabled: std::env::var_os("HITMAN_SHADOWS").is_some(), ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, -0.9, 0.0)),
    ));
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

/// Mission locations, from GMS entity names.
fn find_anchors(level: &glacier::level::Level) -> game::Anchors {
    let at = |name: &str| level.entities.iter().find(|e| e.name == name && e.position != [0.0; 3]).map(|e| view::glacier_to_bevy(e.position));
    let tnt = level.entities.iter().filter(|e| e.name.starts_with("TNT_") && e.position != [0.0; 3]).map(|e| (view::glacier_to_bevy(e.position), false)).collect();
    let bomb_site = at("Bomb_places").unwrap_or(Vec3::splat(f32::MAX));
    let mut bomb_sites: Vec<(Vec3, bool)> = level
        .entities
        .iter()
        .filter(|e| e.name.starts_with("bombicon_") && e.position != [0.0; 3])
        .map(|e| (view::glacier_to_bevy(e.position), false))
        .collect();
    if bomb_sites.is_empty() {
        bomb_sites.push((bomb_site, false));
    }
    let soup_pot = at("SoupCookLoc")
        .or_else(|| at("Poison_actionbox"))
        .unwrap_or(Vec3::new(4.49, 1.01, -41.10));
    let radiation_suit = at("cb_BombWorker_01")
        .unwrap_or(Vec3::new(-323.89, 0.01, -36.86));
    let bjarkhov_office = at("CommanderBjarkhov_01")
        .or_else(|| at("BjarkhovOffice"))
        .unwrap_or(Vec3::new(-153.27, 7.57, 87.93));
    let bjarkhov_desk = at("GreetFuchsLoc")
        .unwrap_or(Vec3::new(-153.30, 7.56, 89.91));
    let ship_gangway = at("FuchsComing_triggerbox")
        .unwrap_or(Vec3::new(-138.85, 1.07, 78.83));
    game::Anchors {
        tnt,
        bomb_sites,
        soup_pot,
        radiation_suit,
        exit: at("Exitbox").or_else(|| at("PlaneLoc")).unwrap_or(Vec3::splat(f32::MAX)),
        // The irradiated sub interior: around the bomb site, bounded by the radiation warning.
        sub: (bomb_site, at("RadiationWarning").map_or(14.0, |r| r.distance(bomb_site).max(8.0))),
        bjarkhov_office,
        bjarkhov_desk,
        ship_gangway,
    }
}

/// Places every scripted character (guards, workers, Bjarkhov…) at its GMS position. An
/// actor entity is followed by a `Ground` child whose primitive is its skinned model;
/// `_HMas` (47's disguise outfits) and `_NUDE` variants are templates, not people.
/// Returns each actor's model by name, for the wardrobe.
fn extract_waypoints(level: &glacier::level::Level, start_idx: usize) -> Vec<Vec3> {
    let mut waypoints = Vec::new();
    let count = level.entities.len();
    if start_idx >= count {
        return waypoints;
    }
    let mut curr = start_idx;
    if level.entities[curr].position == [0.0; 3] && curr + 1 < count {
        curr += 1;
    }
    while curr < count && waypoints.len() < 32 {
        let e = &level.entities[curr];
        let name_lower = e.name.to_ascii_lowercase();
        if e.position == [0.0; 3] && (name_lower.contains("patrol") || name_lower.contains("guard")) {
            break;
        }
        if e.position != [0.0; 3]
            && (name_lower.contains("arrow")
                || name_lower.contains("walk")
                || name_lower.contains("wait")
                || name_lower.contains("loc")
                || name_lower.contains("point"))
        {
            waypoints.push(view::glacier_to_bevy(e.position));
            curr += 1;
        } else if !waypoints.is_empty() && (e.position == [0.0; 3] || name_lower.contains("patrol")) {
            break;
        } else {
            curr += 1;
        }
    }
    waypoints
}

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
        let mut waypoints = actor
            .props
            .as_ref()
            .and_then(|p| if p.waypoints > 0 { Some(extract_waypoints(level, p.waypoints as usize)) } else { None })
            .unwrap_or_default();
        if waypoints.is_empty() {
            let patrol_name = format!("{}_patrol", actor.name);
            let guard_name = format!("{}_GuardPoint", actor.name);
            if let Some(pos_idx) = level.entities.iter().position(|e| e.name == patrol_name || e.name == guard_name) {
                waypoints = extract_waypoints(level, pos_idx);
            }
        }
        if !waypoints.is_empty() {
            info!("{} patrolling {} waypoints", actor.name, waypoints.len());
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
        commands.entity(root).insert((
            Transform { rotation: game::facing(fwd), ..t },
            game::Actor::new(&actor.name),
            game::Patrol {
                waypoints,
                current: 0,
                wait_timer: 0.0,
                forward: true,
            },
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
