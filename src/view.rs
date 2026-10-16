//! Turns a parsed Glacier level into Bevy entities.
//!
//! Coordinates: Glacier world is Y up, centimeters. Entity matrices are nine floats that
//! act on row vectors (equivalently: stored rows are columns), with determinant −1 because PRM vertices store X and Z swapped relative to them: a world
//! point is `R·P·v + p` with `P` = swap(x, z). (Evidence: 12 m fence segments spaced along
//! world X are 12 m long on vertex X but carry an x↔z matrix; same for the plane.)
//! Bevy is right-handed, Y up, meters. We bake `P` into the mesh vertices, mirror world Z
//! (`S = diag(1, 1, −1)`) and scale by 0.01, so the instance transform is `S·R` (a proper
//! rotation) and `S·p`.

use crate::collision::{Surface, WorldCollision};
use crate::game::{Door, DoorLeaf};
use hitman_clone::glacier::{level::Level, prm, tex};
use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use std::collections::HashMap;

pub const CM_TO_M: f32 = 0.01;

/// Engine classes that are pure volumes: room portals (`Gate_*`) and room bounds.
const HELPER_TYPES: [u32; 2] = [0x200015, 0x20001c];
/// Engine class of placed static level geometry. Other classes with meshes are characters'
/// attachments (47's tie sits at his spawn), items and effects, none of which should block.
const STATIC_GEOM_TYPE: u32 = 0x200002;
/// Entity flag on effect templates (smoke, flashes, glows) that the engine spawns on demand.
const FLAG_FX_TEMPLATE: u32 = 0x400;
const MIRROR: Vec3 = Vec3::new(1.0, 1.0, -1.0);

/// Marks everything spawned from level data, as opposed to the player, UI, etc.
#[derive(Component)]
pub struct LevelGeom;

/// GMS entity names by index, for debug output.
#[derive(Resource)]
pub struct EntityNames(pub Vec<String>);

/// Where the spawned geometry is, for camera placement.
#[derive(Resource)]
pub struct LevelBounds {
    pub min: Vec3,
    pub max: Vec3,
    /// Per-axis median of instance positions; robust against far-off prop libraries.
    pub median: Vec3,
    /// World position (m) of every GMS entity, composed through the scene tree.
    pub entity_world: Vec<Vec3>,
    /// World-space boxes of the level's ladder guides (see `ladder`).
    pub ladder_guides: Vec<LadderGuide>,
}

/// Engine class of a ladder guide: an invisible box spanning a climbable ladder
/// (`LadderSub_T1Guide`, `MountainLadder_T1Guide_01`, …).
const LADDER_GUIDE_CLASS: u32 = 0x4000022;

pub struct LadderGuide {
    pub name: String,
    pub min: Vec3,
    pub max: Vec3,
    /// From the ladder mesh at the guide, if any: the centre line of its rungs (low and high
    /// points of its middle) and the horizontal direction the rungs run along. Ladders needn't
    /// be axis-aligned: the submarine's is turned about 30°.
    pub rail: Option<LadderRail>,
}

pub struct LadderRail {
    pub low: Vec3,
    pub high: Vec3,
    pub across: Vec3,
}


pub fn glacier_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::from(p) * MIRROR * CM_TO_M
}

/// What one PRM model turns into: render parts and local-space collision triangles.
#[derive(Default)]
struct CachedModel {
    parts: Vec<(Handle<Mesh>, MaterialKey)>,
    solid: Vec<[Vec3; 3]>,
    /// Every visible triangle, blended layers included: candidates for walkable floor
    /// (snow drifts, the submarine deck) that the collision proxies don't cover.
    surface: Vec<([Vec3; 3], Surface)>,
    /// What the model is mostly made of (its largest visible texture), for its proxies.
    material: Surface,
}

pub fn spawn_level(
    level: &Level,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    collision: &mut WorldCollision,
) -> LevelBounds {
    // Keyed by model and by the per-entity flags that change what counts as solid.
    let mut model_cache: HashMap<(u32, bool, bool), CachedModel> = HashMap::new();
    let mut material_cache: HashMap<MaterialKey, Handle<StandardMaterial>> = HashMap::new();
    let mut positions = Vec::new();
    // Debug: `HITMAN_HIDE=Shadow,Fog` skips entities whose names contain any of the substrings.
    let mut map_textures: HashMap<u32, bool> = HashMap::new();
    let mut tex_surfaces: HashMap<u32, Surface> = HashMap::new();
    let coverage = crate::shot::env_vec3("HITMAN_COVERAGE");
    let holes = std::env::var_os("HITMAN_HOLES").is_some();
    let mut drawn = WorldCollision::default();
    let hide_tex: Vec<u32> = std::env::var("HITMAN_HIDE_TEX").map(|v| v.split(',').filter_map(|t| t.trim().parse().ok()).collect()).unwrap_or_default();
    let hide: Vec<String> = std::env::var("HITMAN_HIDE").map(|v| v.split(',').map(str::to_owned).collect()).unwrap_or_default();

    // First, resolve the full scene graph hierarchy to get world positions for all entities.
    struct Frame {
        world_pos: [f32; 3],
        rot: [[f32; 3]; 3],
        is_physical: bool,
        entity_idx: usize,
    }

    let mut stack: Vec<Frame> = Vec::new();
    let mut world_positions = Vec::with_capacity(level.entities.len());
    let mut world_rotations: Vec<[[f32; 3]; 3]> = Vec::with_capacity(level.entities.len());
    let mut parent_indices: Vec<Option<usize>> = Vec::with_capacity(level.entities.len());

    for (index, e) in level.entities.iter().enumerate() {
        let pops = (e.tree_bits >> 1) as usize;
        for _ in 0..pops {
            stack.pop();
        }
        let parent = stack.last();
        let parent_idx = parent.map(|p| p.entity_idx);
        parent_indices.push(parent_idx);

        let (world_pos, is_child_of_physical) = if let Some(parent) = parent {
            if parent.is_physical {
                let v = e.position;
                let r = parent.rot;
                let rotated = [
                    v[0] * r[0][0] + v[1] * r[1][0] + v[2] * r[2][0],
                    v[0] * r[0][1] + v[1] * r[1][1] + v[2] * r[2][1],
                    v[0] * r[0][2] + v[1] * r[1][2] + v[2] * r[2][2],
                ];
                ([
                    parent.world_pos[0] + rotated[0],
                    parent.world_pos[1] + rotated[1],
                    parent.world_pos[2] + rotated[2],
                ], true)
            } else {
                (e.position, false)
            }
        } else {
            (e.position, false)
        };

        let is_physical = is_child_of_physical
            || e.position[0].abs() > 1.0
            || e.position[1].abs() > 1.0
            || e.position[2].abs() > 1.0;

        // Rotations compose like positions: for row vectors, world = local · parent.
        let world_rot = match stack.last() {
            Some(p) if is_child_of_physical => mat3_mul(e.rotation, p.rot),
            _ => e.rotation,
        };
        if (e.tree_bits & 1) != 0 {
            stack.push(Frame {
                world_pos,
                rot: world_rot,
                is_physical,
                entity_idx: index,
            });
        }
        world_positions.push(world_pos);
        world_rotations.push(world_rot);
    }

    let lifts = layer_flats(level, &world_positions, &world_rotations);
    let ladder_guides: Vec<LadderGuide> = level
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| e.type_id == LADDER_GUIDE_CLASS && e.primitive != 0)
        .filter_map(|(i, e)| {
            let model = level.prm.model(e.primitive).ok()?;
            let world = instance_transform(world_rotations[i], world_positions[i]).to_matrix();
            // Guides carry no geometry, only their model's bounding box.
            let (a, b) = (model.bbox_min, model.bbox_max);
            let (min, max) = (0..8)
                .map(|k| [if k & 1 == 0 { a[0] } else { b[0] }, if k & 2 == 0 { a[1] } else { b[1] }, if k & 4 == 0 { a[2] } else { b[2] }])
                .map(|c| world.transform_point3(Vec3::from(swap_xz(c))))
                .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
            (min.x <= max.x).then(|| LadderGuide { name: e.name.clone(), min, max, rail: None })
        })
        .collect();
    // Each ladder mesh near a guide gives that guide its rail: the mean of its bottom and top
    // tenth of vertices.
    let mut ladder_guides = ladder_guides;
    for (i, e) in level.entities.iter().enumerate().filter(|(_, e)| e.name.contains("Ladder") && e.type_id == STATIC_GEOM_TYPE && e.primitive != 0) {
        let Ok(model) = level.prm.model(e.primitive) else { continue };
        let world = instance_transform(world_rotations[i], world_positions[i]).to_matrix();
        let points: Vec<Vec3> = model.submeshes.iter().filter(|s| s.in_lod0() && !s.is_proxy()).flat_map(|s| s.vertices.iter()).map(|v| world.transform_point3(Vec3::from(swap_xz(v.pos)))).collect();
        let (lo, hi) = points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
        if points.is_empty() || hi - lo < 1.0 {
            continue;
        }
        let band = |from: f32, to: f32| -> Vec<Vec3> { points.iter().copied().filter(|p| p.y >= lo + from * (hi - lo) && p.y <= lo + to * (hi - lo)).collect() };
        let mean = |ps: &[Vec3]| ps.iter().copied().sum::<Vec3>() / ps.len().max(1) as f32;
        // The middle of the ladder: clear of hooks at the top and feet at the bottom.
        let (low_band, high_band, mid) = (band(0.2, 0.4), band(0.6, 0.8), band(0.2, 0.8));
        if low_band.is_empty() || high_band.is_empty() {
            continue;
        }
        let (low, high) = (mean(&low_band), mean(&high_band));
        // The rungs' direction: the main horizontal axis of the middle's vertices.
        let c = mean(&mid);
        let (mut xx, mut xz, mut zz) = (0.0, 0.0, 0.0);
        for p in &mid {
            let (dx, dz) = (p.x - c.x, p.z - c.z);
            xx += dx * dx;
            xz += dx * dz;
            zz += dz * dz;
        }
        let angle = 0.5 * (2.0 * xz).atan2(xx - zz);
        let across = Vec3::new(angle.cos(), 0.0, angle.sin());
        if let Some(g) = ladder_guides.iter_mut().filter(|g| g.rail.is_none()).find(|g| ((g.min + g.max) * 0.5).with_y(0.0).distance(c.with_y(0.0)) < 1.0 && g.max.y > lo && g.min.y < hi) {
            g.rail = Some(LadderRail { low, high, across });
    }
    }
    let mut ddoor_leaves: HashMap<usize, Vec<(bevy::prelude::Entity, u32)>> = HashMap::new();

    for (index, e) in level.entities.iter().enumerate() {
        if e.primitive == 0 || !level.prm.is_record(e.primitive) {
            continue;
        }
        // The in-game map screen's floor plans (`MAP_*`, `C02-1__MAP_Hangar`, …) are laid over
        // the real floors; they're drawn with the map's colour texture.
        if is_map_geometry(level, e, &mut map_textures) {
            spawn_map_piece(level, e, instance_transform(world_rotations[index], world_positions[index]), commands, meshes, materials, images);
            continue;
        }
        if HELPER_TYPES.contains(&e.type_id) || e.flags & FLAG_FX_TEMPLATE != 0 {
            continue;
        }
        if hide.iter().any(|h| !h.is_empty() && e.name.contains(h.as_str())) {
            continue;
        }
        let is_coli_hull = e.name.contains("Coli");
        // Effects and sky: never something to stand on.
        let is_effect = ["Sky", "Blowing", "Fog", "Glitter", "GlowVol", "GlowRain", "Snow_", "SnowBlowing"].iter().any(|k| e.name.contains(k));
        let is_non_solid = is_effect
            || e.name.contains("Cable")
            || e.name.contains("Decal")
            || e.name.contains("Light")
            || e.name.contains("Lamp")
            || e.name.contains("Wire")
            || e.name.contains("Icicle")
            || e.name.contains("Shadow")
            || e.name.contains("Glow")
            || e.name.contains("Corona");
        let cached = model_cache.entry((e.primitive, is_coli_hull, is_non_solid)).or_insert_with(|| match level.prm.model(e.primitive) {
            // Untextured models are collision hulls, position boxes and emitters; only the
            // hulls matter, and only for collision.
            Ok(model) if model.submeshes.iter().all(|sm| sm.is_proxy()) => CachedModel {
                parts: Vec::new(),
                solid: if is_coli_hull { model.submeshes.iter().flat_map(local_triangles).collect() } else { Vec::new() },
                surface: Vec::new(),
                material: Surface::default(),
            },
            Ok(model) => {
                // Proxies (untextured, or render state bit 0x800000) are simplified collision
                // shapes: a cylinder for a drum, boxes for doors, fences and the plane. Never drawn.
                let lod0 = model.submeshes.iter().filter(|sm| sm.in_lod0());
                let (proxies, visible): (Vec<_>, Vec<_>) = lod0.partition(|sm| sm.is_proxy());
                let solid_parts = if is_non_solid {
                    Vec::new()
                } else if !proxies.is_empty() {
                    proxies
                } else {
                    visible.clone()
                };
                CachedModel {
                    parts: visible
                        .iter()
                        .map(|sm| (meshes.add(build_mesh(sm)), MaterialKey { texture: sm.texture, blended: sm.blended }))
                        .collect(),
                    solid: solid_parts
                        .iter()
                        .filter(|sm| sm.is_proxy() || !sm.blended)
                        .flat_map(|sm| local_triangles(sm))
                        .collect(),
                    surface: if is_non_solid {
                        Vec::new()
                    } else {
                        visible.iter().flat_map(|sm| {
                            let kind = surface_of(level, sm.texture, &mut tex_surfaces);
                            local_triangles(sm).map(move |t| (t, kind))
                        }).collect()
                    },
                    material: visible
                        .iter()
                        .max_by(|a, b| area(a).total_cmp(&area(b)))
                        .map_or(Surface::default(), |sm| surface_of(level, sm.texture, &mut tex_surfaces)),
                }
            }
            Err(err) => {
                warn!("{}: {err:#}", e.name);
                CachedModel::default()
            }
        });

        let col_world = instance_transform(world_rotations[index], world_positions[index]).to_matrix();
        for t in cached.solid.iter().filter(|_| e.type_id == STATIC_GEOM_TYPE) {
            collision.add_surface_triangle(t.map(|v| col_world.transform_point3(v)), index as u32, cached.material);
        }
        // Floors the proxies miss: any visible triangle facing up is something to stand on
        // (snow drifts drawn as blended layers, the submarine's deck, road ends). Big
        // structures (the cargo plane, sheds, the sub) also keep their visible walls: their
        // proxies are coarse and leave gaps 47 could walk through.
        if e.type_id == STATIC_GEOM_TYPE && !cached.surface.is_empty() {
            let (lo, hi) = cached.surface.iter().flat_map(|(t, _)| t.iter()).fold((Vec3::MAX, Vec3::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
            let big = (hi - lo).x.max((hi - lo).z) * CM_TO_M > 6.0;
            for (t, kind) in &cached.surface {
                let w = t.map(|v| col_world.transform_point3(v));
                let n = (w[1] - w[0]).cross(w[2] - w[0]).normalize_or_zero();
                if n.y.abs() > 0.7 || big {
                    collision.add_surface_triangle(w, index as u32, *kind);
                }
            }
        }

        let mut transform = instance_transform(world_rotations[index], world_positions[index]);
        transform.translation.y += lifts.get(&index).copied().unwrap_or(0.0);
        if cached.parts.is_empty() {
            continue;
        }

        positions.push(transform.translation);
        // `HITMAN_COVERAGE=x,z,r` / `HITMAN_HOLES`: collect drawn triangles (debugging).
        if coverage.is_some() || holes {
            let world = transform.to_matrix();
            if let Ok(model) = level.prm.model(e.primitive) {
                for sm in model.submeshes.iter().filter(|sm| sm.in_lod0() && !sm.is_proxy() && !hide_tex.contains(&sm.texture)) {
                    for t in local_triangles(sm) {
                        drawn.add_triangle(t.map(|v| world.transform_point3(v)), if holes { index as u32 } else { sm.texture });
                    }
                }
            }
        }
        // `HITMAN_LIST_TEX=321` logs the entities drawing with that texture (debugging).
        if let Ok(t) = std::env::var("HITMAN_LIST_TEX") {
            if cached.parts.iter().any(|(_, k)| t.split(',').any(|x| x.parse() == Ok(k.texture))) {
                info!("tex user: {} type {:#x} flags {:#x} parent {:?}", e.name, e.type_id, e.flags, parent_indices[index].map(|p| &level.entities[p].name));
            }
        }
        // `HITMAN_LIST_NEAR=x,z,r` logs the meshes placed within r metres of (x, z) (debugging).
        if let Some(v) = crate::shot::env_vec3("HITMAN_LIST_NEAR") {
            if transform.translation.xz().distance(Vec2::new(v.x, v.y)) < v.z {
                let model = level.prm.model(e.primitive).ok();
                let parts: Vec<String> = model
                    .iter()
                    .flat_map(|m| m.submeshes.iter().filter(|sm| sm.in_lod0()))
                    .map(|sm| format!("tex {} rs {:#x}{}", sm.texture, sm.render_state, if sm.blended { " blended" } else { "" }))
                    .collect();
                let bounds = model.iter().flat_map(|m| m.submeshes.iter().flat_map(|sm| sm.vertices.iter())).map(|v| transform.transform_point(Vec3::from(swap_xz(v.pos)))).fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(p), hi.max(p)));
                info!("near: {} at {:.1?} lift {:.3} bounds {:.2?}..{:.2?} parts {:?}", e.name, transform.translation.to_array(), lifts.get(&index).copied().unwrap_or(0.0), bounds.0, bounds.1, parts);
            }
        }
        if let Some(props) = &e.cloth {
            // Cloth: a private world-space copy of the mesh, held by the anchor boxes.
            let anchors: Vec<(Vec3, Vec3)> = props
                .anchors
                .iter()
                .filter_map(|&a| {
                    let ae = level.entities.get(a)?;
                    let model = level.prm.model(ae.primitive).ok()?;
                    let to_world = instance_transform(world_rotations[a], world_positions[a]).to_matrix();
                    model
                        .submeshes
                        .iter()
                        .flat_map(|sm| sm.vertices.iter().map(|v| to_world.transform_point3(Vec3::from(swap_xz(v.pos)))))
                        .fold(None, |acc: Option<(Vec3, Vec3)>, p| Some(acc.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p)))))
                })
                .collect();
            let wind = Vec3::from(props.wind).length();
            for (k, (mesh, key)) in cached.parts.iter().enumerate() {
                let Some((dynamic, world, indices)) = meshes.get(mesh).and_then(|m| crate::cloth::dynamic_mesh(m, &GlobalTransform::from(transform)))
                else {
                    continue;
                };
                let handle = meshes.add(dynamic);
                let material = material_cache.entry(*key).or_insert_with(|| materials.add(build_material(&level.tex, *key, images))).clone();
                commands.spawn((
                    Mesh3d(handle.clone()),
                    MeshMaterial3d(material),
                    crate::cloth::Cloth::new(handle, &world, &indices, &anchors, wind, index as f32 + k as f32),
                    bevy::camera::visibility::NoFrustumCulling,
                    LevelGeom,
                    Name::new(e.name.clone()),
                ));
            }
            info!("cloth {}: {} anchors, wind {:.2}", e.name, anchors.len(), wind);
            continue;
        }
        let bevy_entity = commands
            .spawn((transform, Visibility::default(), LevelGeom, Name::new(e.name.clone())))
            .with_children(|c| {
                for (mesh, key) in cached.parts.iter() {
                    // `HITMAN_HIDE_TEX=48,39` skips parts using those textures (debugging).
                    if hide_tex.contains(&key.texture) {
                        continue;
                    }
                    let material = material_cache
                        .entry(*key)
                        .or_insert_with(|| materials.add(build_material(&level.tex, *key, images)))
                        .clone();
                    c.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material)));
                }
            })
            .id();

        let is_ddoor_leaf = parent_indices[index].is_some_and(|pi| {
            level.entities[pi].name.starts_with("DDoor")
        }) && (e.name.starts_with("D1") || e.name.starts_with("D2") || e.name.starts_with("D0"));

        let is_single_door = !is_ddoor_leaf && (e.name.starts_with("Door_") || e.name.starts_with("FenceDoor"));

        if is_ddoor_leaf {
            let swing = if e.position[0] > 0.0 || (e.position[0] == 0.0 && e.position[2] > 0.0) {
                1.4
            } else {
                -1.4
            };
            let open_rotation = transform.rotation * Quat::from_rotation_y(swing);
            commands.entity(bevy_entity).insert(DoorLeaf {
                source_id: index as u32,
                closed_rotation: transform.rotation,
                open_rotation,
            });
            if let Some(pi) = parent_indices[index] {
                ddoor_leaves.entry(pi).or_default().push((bevy_entity, index as u32));
            }
        } else if is_single_door {
            let open_rotation = transform.rotation * Quat::from_rotation_y(1.4);
            commands.entity(bevy_entity).insert((
                DoorLeaf {
                    source_id: index as u32,
                    closed_rotation: transform.rotation,
                    open_rotation,
                },
                Door {
                    name: e.name.clone(),
                    interaction_point: transform.translation + transform.rotation * Vec3::new(0.48, 1.0, 0.0),
                    is_open: false,
                    progress: 0.0,
                    target: 0.0,
                    auto_close_timer: 0.0,
                    leaves: vec![bevy_entity],
                    collision_sources: vec![index as u32],
                },
            ));
        }
    }

    commands.insert_resource(weapon_models(level, &world_positions, &world_rotations, &parent_indices, meshes, materials, images));

    for (ddoor_idx, leaves) in ddoor_leaves {
        let parent_e = &level.entities[ddoor_idx];
        let center_world = glacier_to_bevy(world_positions[ddoor_idx]) + Vec3::Y * 1.0;
        let leaf_entities: Vec<bevy::prelude::Entity> = leaves.iter().map(|(ent, _)| *ent).collect();
        let collision_sources: Vec<u32> = leaves.iter().map(|(_, src)| *src).collect();

        commands.spawn((
            Transform::from_translation(center_world),
            Visibility::default(),
            LevelGeom,
            Name::new(parent_e.name.clone()),
            Door {
                name: parent_e.name.clone(),
                interaction_point: center_world,
                is_open: false,
                progress: 0.0,
                target: 0.0,
                auto_close_timer: 0.0,
                leaves: leaf_entities,
                collision_sources,
            },
        ));
    }

    if holes {
        report_holes(level, &drawn, collision);
    }
    if let Some(c) = coverage {
        // Top-down map: '#' drawn ground within 1.5 m of y=0, '^' only something higher, '.' nothing.
        let r = c.z as i32;
        let mut map = String::new();
        let mut others: std::collections::BTreeMap<u32, usize> = Default::default();
        for dz in -r..=r {
            for dx in -r..=r {
                let (x, z) = (c.x + dx as f32, c.y + dz as f32);
                // Topmost drawn surface near the ground, by texture: s = snow 39, g = gravel 48.
                map.push(match drawn.raycast(Vec3::new(x, 1.5, z), Vec3::NEG_Y, 3.0).map(|h| h.source) {
                    Some(39) => 's',
                    Some(48) => 'g',
                    Some(t) => {
                        *others.entry(t).or_insert(0) += 1;
                        char::from(b'A' + (t % 26) as u8)
                    }
                    None => '.',
                });
            }
            map.push('\n');
        }
        info!("drawn ground coverage around ({}, {}):\n{map}other textures (letter = A + id % 26): {others:?}", c.x, c.y);
    }
    info!(
        "spawned {} instances of {} models, {} materials, {} collision triangles",
        positions.len(),
        model_cache.len(),
        material_cache.len(),
        collision.len()
    );
    let (min, max) = positions.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let mut axis = |k: usize| {
        positions.sort_by(|a, b| a[k].total_cmp(&b[k]));
        positions.get(positions.len() / 2).map_or(0.0, |p| p[k])
    };
    let median = Vec3::new(axis(0), axis(1), axis(2));
    let entity_world: Vec<Vec3> = world_positions.iter().map(|p| glacier_to_bevy(*p)).collect();
    LevelBounds { min, max, median, entity_world, ladder_guides }
}

/// Finds visible floors with no collision under them (where 47 would fall through), on a
/// 1 m grid, and logs them grouped by the mesh that is drawn there.
fn report_holes(level: &Level, drawn: &WorldCollision, collision: &WorldCollision) {
    let (min, max) = collision.bounds();
    let mut by_entity: HashMap<u32, (usize, Vec3)> = HashMap::new();
    let mut total = 0;
    let mut z = min.z.floor();
    while z < max.z {
        let mut x = min.x.floor();
        while x < max.x {
            // Every upward-facing drawn surface in this column, top to bottom.
            let mut y = max.y + 1.0;
            while let Some(h) = drawn.raycast(Vec3::new(x, y, z), Vec3::NEG_Y, y - min.y + 1.0) {
                let floor = y - h.distance;
                if h.normal.y.abs() > 0.8 {
                    let solid = collision.raycast(Vec3::new(x, floor + 0.6, z), Vec3::NEG_Y, 1.2).is_some();
                    if !solid {
                        total += 1;
                        let e = by_entity.entry(h.source).or_insert((0, Vec3::new(x, floor, z)));
                        e.0 += 1;
                    }
                }
                y = floor - 0.05;
            }
            x += 1.0;
        }
        z += 1.0;
    }
    let mut list: Vec<_> = by_entity.into_iter().collect();
    list.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    info!("holes: {total} m² of visible floor without collision");
    for (index, (n, at)) in list.iter().take(40) {
        let name = level.entities.get(*index as usize).map_or("?", |e| e.name.as_str());
        info!("  {n:5} m²  {name}  e.g. at {:.1?}", at.to_array());
    }
}

/// Surface material of texture `id`, by its name (cached).
fn surface_of(level: &Level, id: u32, cache: &mut HashMap<u32, Surface>) -> Surface {
    *cache.entry(id).or_insert_with(|| level.tex.decode(id).ok().flatten().map_or(Surface::default(), |t| Surface::from_texture(&t.name)))
}

/// Rough area of a submesh (cm²), to find what a model is mostly made of.
fn area(sm: &prm::SubMesh) -> f32 {
    sm.triangles
        .iter()
        .map(|t| {
            let p = t.map(|i| Vec3::from(sm.vertices[i as usize].pos));
            (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5
        })
        .sum()
}

/// A gun or the fiber wire as the level's template assembles it: parts relative to the
/// template, and the grip (`PosBox_Hand`) where the hand holds it.
pub struct WeaponModel {
    pub parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>, Transform)>,
    pub grip: Transform,
}

/// The weapons 47 can hold, by template name.
#[derive(Resource, Default)]
pub struct WeaponModels(pub HashMap<String, WeaponModel>);

/// Templates whose models 47 holds.
const HELD_WEAPONS: [&str; 3] = ["Gun_Hardballer_Silenced_01", "Rifle_AK47_01", "CC_FiberWire_01"];

fn weapon_models(
    level: &Level,
    positions: &[[f32; 3]],
    rotations: &[[[f32; 3]; 3]],
    parents: &[Option<usize>],
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> WeaponModels {
    let mut out = WeaponModels::default();
    for name in HELD_WEAPONS {
        let Some(t) = level.entities.iter().position(|e| e.name == name) else { continue };
        let to_template = instance_transform(rotations[t], positions[t]).to_matrix().inverse();
        let relative = |i: usize| Transform::from_matrix(to_template * instance_transform(rotations[i], positions[i]).to_matrix());
        let mut model = WeaponModel { parts: Vec::new(), grip: Transform::IDENTITY };
        for (i, e) in level.entities.iter().enumerate().filter(|(i, _)| parents[*i] == Some(t)) {
            if e.name == "PosBox_Hand" {
                model.grip = relative(i);
                continue;
            }
            // `FiberWire_Inactive` is the coiled wire; the taut one is drawn when in use.
            if e.name.starts_with("PosBox") || e.name == "FiberWire_Inactive" || e.primitive == 0 {
                continue;
            }
            let Ok(prm) = level.prm.model(e.primitive) else { continue };
            for sm in prm.submeshes.iter().filter(|sm| sm.in_lod0() && !sm.is_proxy()) {
                let material = materials.add(build_material(&level.tex, MaterialKey { texture: sm.texture, blended: sm.blended }, images));
                model.parts.push((meshes.add(build_mesh(sm)), material, relative(i)));
            }
        }
        info!("weapon model {name}: {} parts", model.parts.len());
        out.0.insert(name.to_owned(), model);
    }
    out
}

/// Marks the in-game map's floor plans, drawn only by the map camera (`map.rs`).
#[derive(Component)]
pub struct MapGeometry;

/// Render layer of the map screen.
pub const MAP_LAYER: usize = 1;

/// One of the map screen's floor plans: unlit, on the map layer only.
fn spawn_map_piece(
    level: &Level,
    e: &hitman_clone::glacier::gms::Entity,
    transform: Transform,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) {
    let Ok(model) = level.prm.model(e.primitive) else { return };
    let layer = bevy::camera::visibility::RenderLayers::layer(MAP_LAYER);
    let parent = commands.spawn((transform, Visibility::default(), MapGeometry, layer.clone(), Name::new(e.name.clone()))).id();
    for sm in model.submeshes.iter().filter(|sm| sm.in_lod0() && !sm.is_proxy()) {
        let mut material = build_material(&level.tex, MaterialKey { texture: sm.texture, blended: sm.blended }, images);
        material.unlit = true;
        material.fog_enabled = false;
        let child = commands.spawn((Mesh3d(meshes.add(build_mesh(sm))), MeshMaterial3d(materials.add(material)), layer.clone())).id();
        commands.entity(parent).add_child(child);
    }
}

/// Whether `e` belongs to the in-game map screen: named `…MAP_…`, or drawn only with the map's
/// textures (`HMC_IngameMap_*`). `cache` remembers texture names by id.
fn is_map_geometry(level: &Level, e: &hitman_clone::glacier::gms::Entity, cache: &mut HashMap<u32, bool>) -> bool {
    if e.name.contains("MAP_") {
        return true;
    }
    let Ok(model) = level.prm.model(e.primitive) else { return false };
    let mut textures = model.submeshes.iter().filter(|sm| sm.texture != 0).map(|sm| sm.texture).peekable();
    textures.peek().is_some()
        && textures.all(|t| {
            *cache.entry(t).or_insert_with(|| level.tex.decode(t).ok().flatten().is_some_and(|tex| tex.name.contains("IngameMap")))
        })
}

/// Applies `P` (see module docs): PRM vertex space → entity-matrix space.
fn mat3_mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

pub(crate) fn swap_xz([x, y, z]: [f32; 3]) -> [f32; 3] {
    [z, y, x]
}

fn local_triangles(sm: &prm::SubMesh) -> impl Iterator<Item = [Vec3; 3]> + '_ {
    sm.triangles.iter().map(|t| t.map(|i| Vec3::from(swap_xz(sm.vertices[i as usize].pos))))
}

pub(crate) fn instance_transform(r: [[f32; 3]; 3], p: [f32; 3]) -> Transform {
    // The nine stored floats act on row vectors (v·R), so as a column-vector matrix each
    // stored row is a column. Symmetric matrices hide this; the plane's hold segments don't.
    let r = Mat3::from_cols(Vec3::from(r[0]), Vec3::from(r[1]), Vec3::from(r[2]));
    let linear = Mat3::from_diagonal(MIRROR) * r * CM_TO_M;
    Transform::from_matrix(Mat4::from_mat3_translation(linear, glacier_to_bevy(p)))
}

/// Vertical step (m) between stacked coplanar layers, and the extra lift for blended decals.
const LAYER_STEP: f32 = 0.003;
const DECAL_LIFT: f32 = 0.008;

/// The level stacks many flat pieces on exactly the same plane: snow sheets over ground tiles,
/// puddles on floors. The original engine resolved them by draw order; with a depth buffer
/// alone they fight and the floor flickers as the camera moves. This recreates the order: among
/// flat models overlapping at the same height, later entities sit higher, and blended decals sit
/// above opaque floors. Returns a lift (m) per entity index.
fn layer_flats(level: &Level, positions: &[[f32; 3]], rotations: &[[[f32; 3]; 3]]) -> HashMap<usize, f32> {
    struct Flat {
        index: usize,
        height: i32,
        min: Vec2,
        max: Vec2,
        decal: bool,
    }
    let mut flats = Vec::new();
    let mut map_textures: HashMap<u32, bool> = HashMap::new();
    for (index, e) in level.entities.iter().enumerate() {
        if e.type_id != STATIC_GEOM_TYPE || !level.prm.is_record(e.primitive) || is_map_geometry(level, e, &mut map_textures) {
            continue;
        }
        let Ok(model) = level.prm.model(e.primitive) else { continue };
        let parts: Vec<_> = model.submeshes.iter().filter(|s| s.in_lod0() && !s.is_proxy()).collect();
        if parts.is_empty() {
            continue;
        }
        let world = instance_transform(rotations[index], positions[index]).to_matrix();
        let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
        for v in parts.iter().flat_map(|s| s.vertices.iter()) {
            let w = world.transform_point3(Vec3::from(swap_xz(v.pos)));
            lo = lo.min(w);
            hi = hi.max(w);
        }
        if hi.y - lo.y > 0.015 {
            continue; // not a flat piece
        }
        flats.push(Flat { index, height: (hi.y * 500.0).round() as i32, min: lo.xz(), max: hi.xz(), decal: parts.iter().all(|s| s.blended) });
    }
    let mut lifts = HashMap::new();
    for (k, f) in flats.iter().enumerate() {
        let below = flats[..k]
            .iter()
            .filter(|g| (g.height - f.height).abs() <= 2 && g.decal == f.decal && g.min.cmplt(f.max).all() && f.min.cmplt(g.max).all())
            .count()
            .min(4);
        let lift = below as f32 * LAYER_STEP + if f.decal { DECAL_LIFT } else { 0.0 };
        if lift > 0.0 {
            lifts.insert(f.index, lift);
        }
    }
    lifts
}

/// How far (cm) underlay quads are sunk below the layers drawn over them.
const UNDERLAY_SINK: f32 = 4.0;

/// A single large flat quad, such as the dirt under each `Ground_20x14` tile. The snow patches
/// laid over it sit at exactly the same height; the original resolved that by draw order, so
/// we sink the underlay instead, or the two fight and the ground flickers as the camera moves.
fn is_underlay(sm: &prm::SubMesh) -> bool {
    if sm.blended || sm.vertices.len() != 4 {
        return false;
    }
    let (lo, hi) = sm.vertices.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), v| (a.min(Vec3::from(v.pos)), b.max(Vec3::from(v.pos))));
    let size = hi - lo;
    size.y < 1.0 && size.x * size.z > 20.0 * 100.0 * 100.0
}

pub(crate) fn build_mesh(sm: &prm::SubMesh) -> Mesh {
    let sink = if is_underlay(sm) { UNDERLAY_SINK } else { 0.0 };
    let pos: Vec<[f32; 3]> = sm.vertices.iter().map(|v| swap_xz([v.pos[0], v.pos[1] - sink, v.pos[2]])).collect();
    let nrm: Vec<[f32; 3]> = sm.vertices.iter().map(|v| swap_xz(v.normal)).collect();
    let uv: Vec<[f32; 2]> = sm.vertices.iter().map(|v| v.uv).collect();
    let col: Vec<[f32; 4]> = sm.vertices.iter().map(|v| v.color.map(|c| c as f32 / 255.0)).collect();
    let idx: Vec<u16> = sm.triangles.iter().flatten().copied().collect();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nrm)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(Indices::U16(idx))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MaterialKey {
    pub texture: u32,
    pub blended: bool,
}

pub(crate) fn build_material(archive: &tex::TexArchive, key: MaterialKey, images: &mut Assets<Image>) -> StandardMaterial {
    let id = key.texture;
    let mut mat = StandardMaterial {
        perceptual_roughness: 0.9,
        reflectance: 0.1,
        // Strip winding isn't consistent in the source data, so draw both faces. Leave
        // `double_sided` off: it flips normals on back faces, but the authored vertex normals
        // are already right, and flipping them blackens half the terrain.
        cull_mode: None,
        ..default()
    };
    match archive.decode(id) {
        Ok(Some(t)) => {
            mat.alpha_mode = if key.blended { AlphaMode::Blend } else { cutout_mode(&t.rgba) };
            if key.blended {
                // Blended layers (snow over gravel, puddles) lie exactly on the surface beneath;
                // the original drew them second with a depth test that lets equal depths pass.
                mat.depth_bias = std::env::var("HITMAN_DECAL_BIAS").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
            }
            mat.base_color_texture = Some(images.add(build_image(&t)));
            // Vehicle skins (the AN-124 above all) are painted metal: the original gives them an
            // environment-mapped sheen; a glossy finish lets the moon and the lamps trace the
            // fuselage's curve instead of leaving a flat slab.
            if t.name.contains("_VCL_") {
                mat.perceptual_roughness = 0.32;
                mat.reflectance = 0.6;
            }
        }
        Ok(None) => mat.base_color = Color::srgb(0.6, 0.6, 0.6),
        Err(err) => {
            warn!("texture {id}: {err:#}");
            mat.base_color = Color::srgb(1.0, 0.0, 1.0);
        }
    }
    mat
}

/// Non-blended materials with texture alpha are cut-outs (fences, foliage, wire).
fn cutout_mode(rgba: &[u8]) -> AlphaMode {
    if rgba.chunks_exact(4).any(|p| p[3] < 128) { AlphaMode::Mask(0.5) } else { AlphaMode::Opaque }
}

fn build_image(t: &tex::Texture) -> Image {
    let (data, levels) = with_mipmaps(t.width, t.height, &t.rgba);
    // `Image::new` validates a single level, so attach the full chain afterwards.
    let mut img = Image::new(
        Extent3d { width: t.width, height: t.height, depth_or_array_layers: 1 },
        TextureDimension::D2,
        t.rgba.clone(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.data = Some(data);
    img.texture_descriptor.mip_level_count = levels;
    img.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        anisotropy_clamp: 8,
        ..ImageSamplerDescriptor::linear()
    });
    img
}

/// Box-filtered mip chain, all levels concatenated as wgpu expects.
fn with_mipmaps(w: u32, h: u32, rgba: &[u8]) -> (Vec<u8>, u32) {
    let mut out = rgba.to_vec();
    let (mut cw, mut ch) = (w as usize, h as usize);
    let mut prev = rgba.to_vec();
    let mut levels = 1;
    while cw > 1 || ch > 1 {
        let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
        let mut next = vec![0u8; nw * nh * 4];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(cw - 1);
                        let sy = (y * 2 + dy).min(ch - 1);
                        sum += prev[(sy * cw + sx) * 4 + c] as u32;
                    }
                    next[(y * nw + x) * 4 + c] = (sum / 4) as u8;
                }
            }
        }
        out.extend_from_slice(&next);
        prev = next;
        (cw, ch) = (nw, nh);
        levels += 1;
    }
    (out, levels)
}
