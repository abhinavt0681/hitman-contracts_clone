//! Cloth in the wind: the level's `ZCloth` objects (the flag over the yard, the drape on the
//! submarine's deck) simulated as Verlet particles on their own meshes.
//!
//! The engine's settings come from the GMS (`ClothProps`): the anchor boxes that hold the
//! cloth, and how much of the wind it feels. Mesh vertices are welded by position, since the
//! models carry front and back faces as separate vertices; edges keep their rest length and
//! pairs across shared edges resist folding. Wind pushes each triangle along its normal
//! (relative to how fast the cloth already moves) and follows the blizzard's squalls.

use crate::{atmosphere::Gust, snow::WIND};
use bevy::{asset::RenderAssetUsages, mesh::{Indices, PrimitiveTopology, VertexAttributeValues}, prelude::*};
use std::collections::HashMap;

const STEP: f32 = 1.0 / 90.0;
const ITERATIONS: usize = 10;
const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);
const DAMPING: f32 = 0.995;
/// Wind force per square metre per (m/s) of wind across the cloth.
const DRAG: f32 = 0.9;
/// The flag's `WindSpeed` is about 1 and should feel the full blizzard.
const WIND_SCALE: f32 = 1.0;
/// Flag canvas (kg/m²).
const AREAL_DENSITY: f32 = 0.25;

pub struct ClothPlugin;

impl Plugin for ClothPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, simulate);
    }
}

#[derive(Component)]
pub struct Cloth {
    mesh: Handle<Mesh>,
    pos: Vec<Vec3>,
    prev: Vec<Vec3>,
    pinned: Vec<bool>,
    /// Particle of each mesh vertex.
    vertex_particle: Vec<u32>,
    triangles: Vec<[u32; 3]>,
    /// (a, b, rest length, stiffness).
    links: Vec<(u32, u32, f32, f32)>,
    /// How much of the wind this cloth feels.
    wind: f32,
    /// Mass of one particle (kg).
    mass: f32,
    clock: f32,
    phase: f32,
}

impl Cloth {
    /// Builds a cloth from a world-space mesh (`positions`, triangle list `indices`). Particles
    /// inside any of the `anchors` boxes (world AABBs) stay put.
    pub fn new(mesh: Handle<Mesh>, positions: &[Vec3], indices: &[u32], anchors: &[(Vec3, Vec3)], wind: f32, phase: f32) -> Self {
        // Weld vertices that share a position (front and back faces, UV seams).
        let mut by_pos: HashMap<[i32; 3], u32> = HashMap::new();
        let mut pos = Vec::new();
        let vertex_particle: Vec<u32> = positions
            .iter()
            .map(|p| {
                let key = (*p * 200.0).round().as_ivec3().to_array();
                *by_pos.entry(key).or_insert_with(|| {
                    pos.push(*p);
                    pos.len() as u32 - 1
                })
            })
            .collect();
        let triangles: Vec<[u32; 3]> = indices
            .chunks_exact(3)
            .map(|t| [vertex_particle[t[0] as usize], vertex_particle[t[1] as usize], vertex_particle[t[2] as usize]])
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .collect();
        // A flag's two anchors are the top and bottom of its hoist: the whole edge between them
        // is lashed to the pole.
        let hoist = (anchors.len() == 2).then(|| ((anchors[0].0 + anchors[0].1) / 2.0, (anchors[1].0 + anchors[1].1) / 2.0));
        let pinned: Vec<bool> = pos
            .iter()
            .map(|p| {
                anchors.iter().any(|(lo, hi)| p.cmpge(*lo - 0.05).all() && p.cmple(*hi + 0.05).all())
                    || hoist.is_some_and(|(a, b)| {
                        let t = ((*p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                        p.distance(a + (b - a) * t) < 0.06
                    })
            })
            .collect();

        // Structural links on every edge, bending links across every shared edge.
        let mut edges: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for t in &triangles {
            for k in 0..3 {
                let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                edges.entry((a.min(b), a.max(b))).or_default().push(c);
            }
        }
        let mut links = Vec::new();
        for (&(a, b), opposite) in &edges {
            links.push((a, b, pos[a as usize].distance(pos[b as usize]), 1.0));
            for (i, &c) in opposite.iter().enumerate() {
                for &d in &opposite[i + 1..] {
                    if c != d {
                        links.push((c, d, pos[c as usize].distance(pos[d as usize]), 0.25));
                    }
                }
            }
        }
        if !pinned.iter().any(|&p| p) {
            warn!("cloth with {} particles has nothing holding it", pos.len());
        }
        let area: f32 = triangles.iter().map(|t| (pos[t[1] as usize] - pos[t[0] as usize]).cross(pos[t[2] as usize] - pos[t[0] as usize]).length() * 0.5).sum();
        // Front and back faces were welded into the same particles, so each area counts twice.
        let mass = (area / 2.0 * AREAL_DENSITY / pos.len().max(1) as f32).max(1e-4);
        info!("cloth: {} particles ({} pinned), {} triangles, {} links", pos.len(), pinned.iter().filter(|&&p| p).count(), triangles.len(), links.len());
        Cloth { mesh, prev: pos.clone(), pos, pinned, vertex_particle, triangles, links, wind, mass, clock: 0.0, phase }
    }

    fn step(&mut self, wind: Vec3) {
        let n = self.pos.len();
        let mut force = vec![GRAVITY * self.mass; n];
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| i as usize);
            let (pa, pb, pc) = (self.pos[a], self.pos[b], self.pos[c]);
            let cross = (pb - pa).cross(pc - pa);
            let area = cross.length() * 0.5;
            if area < 1e-6 {
                continue;
            }
            let normal = cross / (2.0 * area);
            let velocity = (pa - self.prev[a] + pb - self.prev[b] + pc - self.prev[c]) / (3.0 * STEP);
            let relative = wind - velocity;
            let f = normal * (DRAG * area * relative.dot(normal)) / 3.0;
            force[a] += f;
            force[b] += f;
            force[c] += f;
        }
        for i in 0..n {
            if self.pinned[i] {
                continue;
            }
            let p = self.pos[i];
            self.pos[i] = p + (p - self.prev[i]) * DAMPING + force[i] / self.mass * STEP * STEP;
            self.prev[i] = p;
        }
        for _ in 0..ITERATIONS {
            for &(a, b, rest, stiffness) in &self.links {
                let (a, b) = (a as usize, b as usize);
                let d = self.pos[b] - self.pos[a];
                let len = d.length();
                if len < 1e-6 {
                    continue;
                }
                let (wa, wb) = (!self.pinned[a] as u8 as f32, !self.pinned[b] as u8 as f32);
                if wa + wb == 0.0 {
                    continue;
                }
                let fix = d * ((len - rest) / len) * stiffness / (wa + wb);
                self.pos[a] += fix * wa;
                self.pos[b] -= fix * wb;
            }
        }
    }
}

/// Splits every triangle into four at its edge midpoints (shared between neighbours), so the
/// cloth has enough particles to ripple. Vertex attributes are interpolated.
fn subdivide(pos: &mut Vec<Vec3>, uv: &mut Vec<[f32; 2]>, color: &mut Vec<[f32; 4]>, indices: &mut Vec<u32>) {
    let mut mid: HashMap<(u32, u32), u32> = HashMap::new();
    let mut out = Vec::with_capacity(indices.len() * 4);
    let mut midpoint = |a: u32, b: u32, pos: &mut Vec<Vec3>, uv: &mut Vec<[f32; 2]>, color: &mut Vec<[f32; 4]>| {
        *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
            let (a, b) = (a as usize, b as usize);
            pos.push((pos[a] + pos[b]) / 2.0);
            uv.push([(uv[a][0] + uv[b][0]) / 2.0, (uv[a][1] + uv[b][1]) / 2.0]);
            color.push(std::array::from_fn(|k| (color[a][k] + color[b][k]) / 2.0));
            pos.len() as u32 - 1
        })
    };
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (t[0], t[1], t[2]);
        let ab = midpoint(a, b, pos, uv, color);
        let bc = midpoint(b, c, pos, uv, color);
        let ca = midpoint(c, a, pos, uv, color);
        out.extend([a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
    }
    *indices = out;
}

/// Subdivisions for a cloth mesh of `triangles`: aim for a few hundred to a thousand.
fn subdivisions(triangles: usize) -> usize {
    (0..3).find(|&k| triangles * 4usize.pow(k as u32 + 1) > 1200).unwrap_or(3)
}

/// A world-space copy of a level mesh that the cloth can rewrite every frame.
pub fn dynamic_mesh(source: &Mesh, to_world: &GlobalTransform) -> Option<(Mesh, Vec<Vec3>, Vec<u32>)> {
    let Some(VertexAttributeValues::Float32x3(local)) = source.attribute(Mesh::ATTRIBUTE_POSITION) else { return None };
    let mut world: Vec<Vec3> = local.iter().map(|p| to_world.transform_point(Vec3::from(*p))).collect();
    let mut indices: Vec<u32> = source.indices()?.iter().map(|i| i as u32).collect();
    let mut uv = match source.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(v)) => v.clone(),
        _ => vec![[0.0; 2]; world.len()],
    };
    let mut color = match source.attribute(Mesh::ATTRIBUTE_COLOR) {
        Some(VertexAttributeValues::Float32x4(v)) => v.clone(),
        _ => vec![[1.0; 4]; world.len()],
    };
    for _ in 0..subdivisions(indices.len() / 3) {
        subdivide(&mut world, &mut uv, &mut color, &mut indices);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::all())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, world.iter().map(|p| p.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, color)
        .with_inserted_indices(Indices::U32(indices.clone()));
    mesh.compute_smooth_normals();
    Some((mesh, world, indices))
}

fn simulate(time: Res<Time>, gust: Res<Gust>, mut cloths: Query<&mut Cloth>, mut meshes: ResMut<Assets<Mesh>>) {
    // Never try to catch up more than a few steps after a hitch.
    let dt = time.delta_secs().min(0.05);
    for mut cloth in &mut cloths {
        let cloth = &mut *cloth;
        cloth.clock += dt;
        let t = time.elapsed_secs() + cloth.phase;
        // The blizzard's wind, swelling with squalls and buffeting in short bursts.
        let buffet = 1.0 + 0.35 * (t * 2.3).sin() * (t * 0.7).sin() + 0.15 * (t * 7.1).sin();
        let side = Vec3::new((t * 1.3).sin(), 0.25 * (t * 1.9).sin(), (t * 1.1).cos()) * 2.0;
        let wind = (WIND * (1.0 + 1.6 * gust.strength) * buffet + side) * cloth.wind * WIND_SCALE;
        while cloth.clock >= STEP {
            cloth.clock -= STEP;
            cloth.step(wind);
        }
        let Some(mut mesh) = meshes.get_mut(&cloth.mesh) else { continue };
        let positions: Vec<[f32; 3]> = cloth.vertex_particle.iter().map(|&i| cloth.pos[i as usize].to_array()).collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.compute_smooth_normals();
    }
}
