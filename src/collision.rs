//! Static level collision: a triangle soup bucketed into a uniform XZ grid.
//!
//! Built from the opaque render geometry plus the level's invisible `Coli*` hulls. The real
//! engine collision lives in `.OCT`, which isn't decoded yet; render meshes are a decent
//! stand-in for walking around.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

const CELL: f32 = 4.0;

#[derive(Resource, Default, Clone)]
pub struct WorldCollision {
    tris: Vec<[Vec3; 3]>,
    /// Index of the GMS entity each triangle came from, for debugging what you bumped into.
    source: Vec<u32>,
    /// What each triangle is made of, for footsteps and bullet impacts.
    surface: Vec<Surface>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    disabled_sources: HashSet<u32>,
}

pub struct Hit {
    pub distance: f32,
    pub normal: Vec3,
    /// Source id of the triangle hit (the GMS entity, unless the builder chose otherwise).
    pub source: u32,
    pub surface: Surface,
}

/// Material of a surface, from its texture name; picks step and impact sounds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Surface {
    #[default]
    Snow,
    Gravel,
    Concrete,
    Steel,
    Wood,
    Carpet,
    Water,
    Dirt,
}

impl Surface {
    /// From a texture name such as `HMC_C2_Metal_12a` or `HMC_C2_Ground_14a`.
    pub fn from_texture(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        let has = |k: &str| n.contains(k);
        if has("water") || has("liquid") {
            Surface::Water
        } else if has("ground_14") || has("gravel") || has("pebble") {
            Surface::Gravel
        } else if has("dirt") {
            Surface::Dirt
        } else if has("metal") || has("submarine") || has("planetransport") || has("trolley") || has("steel") || has("grate") {
            Surface::Steel
        } else if has("wood") || has("board") || has("plank") {
            Surface::Wood
        } else if has("fabric") || has("cloth") || has("carpet") {
            Surface::Carpet
        } else if has("stone") || has("concrete") || has("tile") || has("ceramic") {
            Surface::Concrete
        } else {
            Surface::Snow
        }
    }
}

fn cell_of(v: f32) -> i32 {
    (v / CELL).floor() as i32
}

impl WorldCollision {
    pub fn add_triangle(&mut self, t: [Vec3; 3], source: u32) {
        self.add_surface_triangle(t, source, Surface::Snow);
    }

    pub fn add_surface_triangle(&mut self, t: [Vec3; 3], source: u32, surface: Surface) {
        let n = (t[1] - t[0]).cross(t[2] - t[0]);
        if n.length_squared() < 1e-10 {
            return;
        }
        let min = t[0].min(t[1]).min(t[2]);
        let max = t[0].max(t[1]).max(t[2]);
        // Skybox-scale triangles would land in thousands of cells; nothing walkable is that big.
        if (max - min).max_element() > 400.0 {
            return;
        }
        let id = self.tris.len() as u32;
        self.tris.push(t);
        self.source.push(source);
        self.surface.push(surface);
        for x in cell_of(min.x)..=cell_of(max.x) {
            for z in cell_of(min.z)..=cell_of(max.z) {
                self.grid.entry((x, z)).or_default().push(id);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.tris.len()
    }

    /// Height of the highest surface over each cell of a `w`×`h` grid (cell centres at
    /// `origin + (i + 0.5) * cell`), or `f32::MIN` where nothing is overhead: a top-down depth
    /// map of the level, used to keep snow out from under roofs.
    pub fn top_heights(&self, origin: Vec2, cell: f32, w: usize, h: usize) -> Vec<f32> {
        let mut top = vec![f32::MIN; w * h];
        for t in &self.tris {
            let min = t[0].min(t[1]).min(t[2]).xz();
            let max = t[0].max(t[1]).max(t[2]).xz();
            let lo = ((min - origin) / cell - 0.5).ceil().as_ivec2().max(IVec2::ZERO);
            let hi = ((max - origin) / cell - 0.5).floor().as_ivec2().min(IVec2::new(w as i32 - 1, h as i32 - 1));
            let (a, b, c) = (t[0].xz(), t[1].xz(), t[2].xz());
            let area = (b - a).perp_dot(c - a);
            if area.abs() < 1e-8 {
                continue;
            }
            for z in lo.y..=hi.y {
                for x in lo.x..=hi.x {
                    let p = origin + (Vec2::new(x as f32, z as f32) + 0.5) * cell;
                    // Barycentric test and height interpolation.
                    let w0 = (b - p).perp_dot(c - p) / area;
                    let w1 = (c - p).perp_dot(a - p) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                        continue;
                    }
                    let y = w0 * t[0].y + w1 * t[1].y + w2 * t[2].y;
                    let slot = &mut top[z as usize * w + x as usize];
                    *slot = slot.max(y);
                }
            }
        }
        top
    }

    /// World-space bounds of all triangles.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.tris.iter().flatten().fold((Vec3::MAX, Vec3::MIN), |(a, b), v| (a.min(*v), b.max(*v)))
    }

    pub fn disable_source(&mut self, source: u32) {
        self.disabled_sources.insert(source);
    }

    pub fn enable_source(&mut self, source: u32) {
        self.disabled_sources.remove(&source);
    }

    #[allow(dead_code)]
    pub fn is_source_disabled(&self, source: u32) -> bool {
        self.disabled_sources.contains(&source)
    }

    fn candidates(&self, min: Vec3, max: Vec3) -> Vec<u32> {
        let mut out = Vec::new();
        for x in cell_of(min.x)..=cell_of(max.x) {
            for z in cell_of(min.z)..=cell_of(max.z) {
                if let Some(ids) = self.grid.get(&(x, z)) {
                    out.extend_from_slice(ids);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Source entities of wall triangles within `radius` (horizontally) of `center`.
    pub fn walls_near(&self, center: Vec3, radius: f32) -> Vec<u32> {
        let r = Vec3::splat(radius);
        let mut out: Vec<u32> = self
            .candidates(center - r, center + r)
            .into_iter()
            .filter(|&id| {
                if self.disabled_sources.contains(&self.source[id as usize]) {
                    return false;
                }
                let [a, b, c] = self.tris[id as usize];
                let p = closest_point_on_triangle(center, a, b, c);
                let diff = center - p;
                let dy = diff.y.abs();
                if dy >= radius {
                    return false;
                }
                let r_slice = (radius * radius - dy * dy).sqrt();
                let mut d = diff;
                d.y = 0.0;
                (b - a).cross(c - a).normalize().y.abs() <= 0.7 && d.length() < r_slice
            })
            .map(|id| self.source[id as usize])
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Nearest spot to `feet` (searching outward in rings up to `max_radius`) with a floor
    /// within a meter and no walls within `clearance` at knee and chest height.
    pub fn free_spot(&self, feet: Vec3, clearance: f32, max_radius: f32) -> Option<Vec3> {
        let try_at = |p: Vec3| -> Option<Vec3> {
            let hit = self.raycast(p + Vec3::Y * 1.0, Vec3::NEG_Y, 2.0)?;
            let floor = p + Vec3::Y * (1.0 - hit.distance);
            let blocked = [0.75, 1.5].iter().any(|h| !self.walls_near(floor + Vec3::Y * h, clearance).is_empty());
            (hit.normal.y > 0.7 && !blocked).then_some(floor)
        };
        let step = 0.25;
        for ring in 0..=(max_radius / step) as i32 {
            let r = ring as f32 * step;
            let samples = (ring * 8).max(1);
            for k in 0..samples {
                let a = k as f32 / samples as f32 * std::f32::consts::TAU;
                if let Some(p) = try_at(feet + Vec3::new(a.cos() * r, 0.0, a.sin() * r)) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Nearest hit along a ray segment (both faces count).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<Hit> {
        let dir = dir.normalize_or_zero();
        let end = origin + dir * max_dist;
        let mut best: Option<Hit> = None;
        for id in self.candidates(origin.min(end), origin.max(end)) {
            if self.disabled_sources.contains(&self.source[id as usize]) {
                continue;
            }
            let [a, b, c] = self.tris[id as usize];
            // Möller–Trumbore
            let (e1, e2) = (b - a, c - a);
            let p = dir.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-8 {
                continue;
            }
            let inv = 1.0 / det;
            let s = origin - a;
            let u = s.dot(p) * inv;
            const EPS: f32 = 1e-4;
            if u < -EPS || u > 1.0 + EPS {
                continue;
            }
            let q = s.cross(e1);
            let v = dir.dot(q) * inv;
            if v < -EPS || u + v > 1.0 + EPS {
                continue;
            }
            let t = e2.dot(q) * inv;
            if t >= 0.0 && t <= max_dist && best.as_ref().is_none_or(|h| t < h.distance) {
                let mut normal = e1.cross(e2).normalize();
                if normal.dot(dir) > 0.0 {
                    normal = -normal;
                }
                best = Some(Hit { distance: t, normal, source: self.source[id as usize], surface: self.surface[id as usize] });
            }
        }
        best
    }

    /// Horizontal push that moves a sphere out of walls. Floor-like faces are ignored so
    /// the sphere doesn't get lifted by the ground it stands on.
    pub fn push_out_horizontal(&self, center: Vec3, radius: f32) -> Vec3 {
        let mut push = Vec3::ZERO;
        let r = Vec3::splat(radius);
        for id in self.candidates(center - r, center + r) {
            if self.disabled_sources.contains(&self.source[id as usize]) {
                continue;
            }
            let [a, b, c] = self.tris[id as usize];
            let n = (b - a).cross(c - a).normalize();
            if n.y.abs() > 0.7 {
                continue;
            }
            let p = closest_point_on_triangle(center + push, a, b, c);
            let diff = center + push - p;
            let dy = diff.y.abs();
            if dy >= radius {
                continue;
            }
            let r_slice = (radius * radius - dy * dy).sqrt();
            let mut d = diff;
            d.y = 0.0;
            let len = d.length();
            if len < r_slice {
                if len > 1e-5 {
                    push += d / len * (r_slice - len);
                } else {
                    let n_horiz = Vec3::new(n.x, 0.0, n.z);
                    let n_len = n_horiz.length();
                    if n_len > 1e-5 {
                        push += (n_horiz / n_len) * r_slice;
                    }
                }
            }
        }
        push
    }
}

/// Real-Time Collision Detection (Ericson), 5.1.5.
fn closest_point_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}
