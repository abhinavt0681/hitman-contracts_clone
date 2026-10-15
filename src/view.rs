//! Turns a parsed Glacier level into Bevy entities.
//!
//! Coordinates: Glacier world is Y up, centimeters. Entity matrices are nine floats that
//! act on row vectors (equivalently: stored rows are columns), with determinant −1 because PRM vertices store X and Z swapped relative to them: a world
//! point is `R·P·v + p` with `P` = swap(x, z). (Evidence: 12 m fence segments spaced along
//! world X are 12 m long on vertex X but carry an x↔z matrix; same for the plane.)
//! Bevy is right-handed, Y up, meters. We bake `P` into the mesh vertices, mirror world Z
//! (`S = diag(1, 1, −1)`) and scale by 0.01, so the instance transform is `S·R` (a proper
//! rotation) and `S·p`.

use crate::collision::WorldCollision;
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
}


pub fn glacier_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::from(p) * MIRROR * CM_TO_M
}

/// What one PRM model turns into: render parts and local-space collision triangles.
#[derive(Default)]
struct CachedModel {
    parts: Vec<(Handle<Mesh>, MaterialKey)>,
    solid: Vec<[Vec3; 3]>,
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
    let mut ddoor_leaves: HashMap<usize, Vec<(bevy::prelude::Entity, u32)>> = HashMap::new();

    for (index, e) in level.entities.iter().enumerate() {
        if e.primitive == 0 || !level.prm.is_record(e.primitive) {
            continue;
        }
        // `MAP_*` geometry is the in-game map screen's floor plan, placed over the real floors.
        if e.name.starts_with("MAP_") {
            continue;
        }
        if HELPER_TYPES.contains(&e.type_id) || e.flags & FLAG_FX_TEMPLATE != 0 {
            continue;
        }
        if hide.iter().any(|h| !h.is_empty() && e.name.contains(h.as_str())) {
            continue;
        }
        let is_coli_hull = e.name.contains("Coli");
        let is_non_solid = e.name.contains("Cable")
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
            Ok(model) if model.submeshes.iter().all(|sm| sm.texture == 0) => CachedModel {
                parts: Vec::new(),
                solid: if is_coli_hull { model.submeshes.iter().flat_map(local_triangles).collect() } else { Vec::new() },
            },
            Ok(model) => {
                // Untextured parts (render state 0xC000 bits) are simplified collision proxies:
                // a cylinder for a drum, boxes for doors and fences. Never drawn.
                let lod0 = model.submeshes.iter().filter(|sm| sm.in_lod0());
                let (proxies, visible): (Vec<_>, Vec<_>) = lod0.partition(|sm| sm.texture == 0);
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
                        .filter(|sm| sm.texture == 0 || !sm.blended)
                        .flat_map(|sm| local_triangles(sm))
                        .collect(),
                }
            }
            Err(err) => {
                warn!("{}: {err:#}", e.name);
                CachedModel::default()
            }
        });

        let mut transform = instance_transform(world_rotations[index], world_positions[index]);
        transform.translation.y += lifts.get(&index).copied().unwrap_or(0.0);
        let world = transform.to_matrix();
        for t in cached.solid.iter().filter(|_| e.type_id == STATIC_GEOM_TYPE) {
            collision.add_triangle(t.map(|v| world.transform_point3(v)), index as u32);
        }
        if cached.parts.is_empty() {
            continue;
        }

        positions.push(transform.translation);
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
    LevelBounds { min, max, median, entity_world }
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
    for (index, e) in level.entities.iter().enumerate() {
        if e.type_id != STATIC_GEOM_TYPE || !level.prm.is_record(e.primitive) || e.name.starts_with("MAP_") {
            continue;
        }
        let Ok(model) = level.prm.model(e.primitive) else { continue };
        let parts: Vec<_> = model.submeshes.iter().filter(|s| s.in_lod0() && s.texture != 0).collect();
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

fn build_mesh(sm: &prm::SubMesh) -> Mesh {
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
            mat.base_color_texture = Some(images.add(build_image(&t)));
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
