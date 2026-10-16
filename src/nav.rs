//! Navigation: a 2.5D walkable grid sampled from the level collision, and A* over it.
//!
//! Each cell (`CELL` m square) stores every floor found under it, so stacked floors (ship
//! decks, the submarine, upstairs rooms) each get their own node. A node links to a floor in
//! a neighbouring cell when the step is low enough and nothing blocks the way at knee and chest
//! height. Doors count as open, since guards open them. The grid takes a few seconds to sample,
//! so it is built on worker threads and cached in `data/cache/`.

use crate::{collision::WorldCollision, game::Door};
use bevy::prelude::*;
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
    path::PathBuf,
};

pub const CELL: f32 = 0.75;
/// Highest ledge a character steps up between neighbouring cells.
const STEP: f32 = 0.5;
/// Headroom a floor needs to count as walkable.
const HEADROOM: f32 = 1.7;
/// Body radius kept clear of walls.
const CLEARANCE: f32 = 0.3;
/// Floors per cell beyond which the rest are ignored.
const MAX_FLOORS: usize = 6;
const MIN_FLOOR_NY: f32 = 0.6;
const CACHE_VERSION: u32 = 4;

const DIRS: [(i32, i32); 8] = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];

pub struct NavPlugin;

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, build_nav);
    }
}

#[derive(Clone, Copy)]
struct Node {
    y: f32,
    /// Per direction (DIRS order): index+1 of the floor reached in that neighbour, 0 = none.
    links: [u8; 8],
}

#[derive(Resource)]
pub struct NavGrid {
    origin: Vec2,
    w: usize,
    h: usize,
    /// Per cell: first node index and count.
    cells: Vec<(u32, u8)>,
    nodes: Vec<Node>,
}

type NodeId = u32;

impl NavGrid {
    fn cell_of(&self, p: Vec3) -> Option<(usize, usize)> {
        let c = ((p.xz() - self.origin) / CELL).floor();
        (c.x >= 0.0 && c.y >= 0.0 && (c.x as usize) < self.w && (c.y as usize) < self.h).then(|| (c.x as usize, c.y as usize))
    }

    fn center(&self, cx: usize, cz: usize) -> Vec2 {
        self.origin + (Vec2::new(cx as f32, cz as f32) + 0.5) * CELL
    }

    fn node_pos(&self, id: NodeId) -> Vec3 {
        let (cx, cz) = self.cell_xy(id);
        let c = self.center(cx, cz);
        Vec3::new(c.x, self.nodes[id as usize].y, c.y)
    }

    fn cell_xy(&self, id: NodeId) -> (usize, usize) {
        // Binary search the cell whose node range contains id.
        let i = self.cells.partition_point(|&(start, _)| start <= id) - 1;
        (i % self.w, i / self.w)
    }

    /// The walkable floor point nearest to `p`.
    pub fn snap(&self, p: Vec3) -> Option<Vec3> {
        self.nearest(p).map(|id| self.node_pos(id))
    }

    /// Nearest walkable node to `p`: the floor closest in height in its cell, else a nearby cell.
    pub fn nearest(&self, p: Vec3) -> Option<NodeId> {
        let (cx, cz) = self.cell_of(p)?;
        for r in 0..4i32 {
            let mut best: Option<(f32, NodeId)> = None;
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs().max(dz.abs()) != r {
                        continue;
                    }
                    let (x, z) = (cx as i32 + dx, cz as i32 + dz);
                    if x < 0 || z < 0 || x as usize >= self.w || z as usize >= self.h {
                        continue;
                    }
                    let (start, n) = self.cells[z as usize * self.w + x as usize];
                    for k in 0..n as u32 {
                        let id = start + k;
                        let d = self.node_pos(id).distance(p) + (self.nodes[id as usize].y - p.y).abs() * 2.0;
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, id));
                        }
                    }
                }
            }
            if let Some((_, id)) = best {
                return Some(id);
            }
        }
        None
    }

    fn neighbours(&self, id: NodeId) -> impl Iterator<Item = (NodeId, f32)> + '_ {
        let (cx, cz) = self.cell_xy(id);
        let node = self.nodes[id as usize];
        DIRS.iter().enumerate().filter_map(move |(d, &(dx, dz))| {
            let floor = node.links[d];
            if floor == 0 {
                return None;
            }
            let (x, z) = ((cx as i32 + dx) as usize, (cz as i32 + dz) as usize);
            let (start, _) = self.cells[z * self.w + x];
            let cost = if dx != 0 && dz != 0 { std::f32::consts::SQRT_2 } else { 1.0 };
            Some((start + floor as u32 - 1, cost * CELL))
        })
    }

    /// A* from `from` to `to`; the path is simplified to the corners that matter.
    pub fn path(&self, from: Vec3, to: Vec3, world: &WorldCollision) -> Option<Vec<Vec3>> {
        let (start, goal) = (self.nearest(from)?, self.nearest(to)?);
        let goal_pos = self.node_pos(goal);
        let mut open = BinaryHeap::new();
        let mut came: HashMap<NodeId, NodeId> = HashMap::new();
        let mut cost: HashMap<NodeId, f32> = HashMap::new();
        cost.insert(start, 0.0);
        open.push(Open(self.node_pos(start).distance(goal_pos), start));
        let mut expanded = 0;
        while let Some(Open(_, cur)) = open.pop() {
            if cur == goal {
                let mut ids = vec![cur];
                while let Some(&p) = came.get(ids.last().unwrap()) {
                    ids.push(p);
                }
                ids.reverse();
                let mut pts: Vec<Vec3> = ids.iter().map(|&i| self.node_pos(i)).collect();
                pts.push(to);
                return Some(simplify(&pts, world));
            }
            expanded += 1;
            if expanded > 25_000 {
                return None; // unreachable or too far; callers fall back to steering
            }
            let g = cost[&cur];
            for (next, step) in self.neighbours(cur) {
                let ng = g + step;
                if cost.get(&next).is_none_or(|&c| ng < c) {
                    cost.insert(next, ng);
                    came.insert(next, cur);
                    open.push(Open(ng + self.node_pos(next).distance(goal_pos), next));
                }
            }
        }
        None
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
}

struct Open(f32, NodeId);
impl PartialEq for Open {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}
impl Eq for Open {}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0) // min-heap
    }
}

/// Drops intermediate points while the straight line between kept points stays clear.
fn simplify(pts: &[Vec3], world: &WorldCollision) -> Vec<Vec3> {
    let mut out = vec![pts[0]];
    let mut i = 0;
    while i + 1 < pts.len() {
        let mut j = pts.len() - 1;
        while j > i + 1 && !clear_line(world, pts[i], pts[j]) {
            j -= 1;
        }
        out.push(pts[j]);
        i = j;
    }
    out
}

fn clear_line(world: &WorldCollision, a: Vec3, b: Vec3) -> bool {
    if (a.y - b.y).abs() > STEP {
        return false;
    }
    [0.5, 1.2].iter().all(|h| {
        let (p, q) = (a + Vec3::Y * h, b + Vec3::Y * h);
        let d = q - p;
        world.raycast(p, d, d.length()).is_none()
    })
}

/// Every walkable floor under a cell centre, highest first.
fn floors_at(world: &WorldCollision, c: Vec2, top: f32, bottom: f32) -> Vec<f32> {
    let mut out = Vec::new();
    let mut y = top;
    while y > bottom && out.len() < MAX_FLOORS {
        let origin = Vec3::new(c.x, y, c.y);
        let Some(hit) = world.raycast(origin, Vec3::NEG_Y, y - bottom) else { break };
        let fy = y - hit.distance;
        if hit.normal.y >= MIN_FLOOR_NY {
            // Headroom and room to stand, clear of walls at knee and chest.
            let p = Vec3::new(c.x, fy, c.y);
            let head = out.last().is_none_or(|&above: &f32| above - fy >= HEADROOM);
            let roomy = [0.6, 1.3].iter().all(|h| world.walls_near(p + Vec3::Y * h, CLEARANCE).is_empty());
            if head && roomy {
                out.push(fy);
            }
        }
        y = fy - 0.05;
    }
    out
}

fn build(world: &WorldCollision) -> NavGrid {
    let (lo, hi) = world.bounds();
    let origin = lo.xz();
    let w = ((hi.x - lo.x) / CELL).ceil() as usize + 1;
    let h = ((hi.z - lo.z) / CELL).ceil() as usize + 1;
    let (top, bottom) = (hi.y + 1.0, lo.y - 1.0);

    // 1. Floors per cell, sampled on all cores.
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows: Vec<Vec<Vec<f32>>> = std::thread::scope(|s| {
        let chunk = h.div_ceil(threads);
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                s.spawn(move || {
                    (t * chunk..((t + 1) * chunk).min(h))
                        .map(|z| (0..w).map(|x| floors_at(world, origin + (Vec2::new(x as f32, z as f32) + 0.5) * CELL, top, bottom)).collect())
                        .collect::<Vec<Vec<Vec<f32>>>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });

    let mut cells = Vec::with_capacity(w * h);
    let mut nodes = Vec::new();
    for row in &rows {
        for floors in row {
            cells.push((nodes.len() as u32, floors.len() as u8));
            nodes.extend(floors.iter().map(|&y| Node { y, links: [0; 8] }));
        }
    }
    let mut grid = NavGrid { origin, w, h, cells, nodes };

    // 2. Links to neighbouring floors.
    let mut links = vec![[0u8; 8]; grid.nodes.len()];
    std::thread::scope(|s| {
        let g = &grid;
        let chunk = g.nodes.len().div_ceil(threads).max(1);
        for (t, out) in links.chunks_mut(chunk).enumerate() {
            s.spawn(move || {
                for (k, l) in out.iter_mut().enumerate() {
                    let id = (t * chunk + k) as NodeId;
                    let (cx, cz) = g.cell_xy(id);
                    let here = g.node_pos(id);
                    for (d, &(dx, dz)) in DIRS.iter().enumerate() {
                        let (x, z) = (cx as i32 + dx, cz as i32 + dz);
                        if x < 0 || z < 0 || x as usize >= g.w || z as usize >= g.h {
                            continue;
                        }
                        let (start, n) = g.cells[z as usize * g.w + x as usize];
                        let best = (0..n).filter(|&f| (g.nodes[(start + f as u32) as usize].y - here.y).abs() <= STEP).min_by(|&a, &b| {
                            let ya = (g.nodes[(start + a as u32) as usize].y - here.y).abs();
                            let yb = (g.nodes[(start + b as u32) as usize].y - here.y).abs();
                            ya.total_cmp(&yb)
                        });
                        if let Some(f) = best {
                            let there = g.node_pos(start + f as u32);
                            if clear_line(world, here, there) {
                                l[d] = f + 1;
                            }
                        }
                    }
                }
            });
        }
    });
    for (node, l) in grid.nodes.iter_mut().zip(links) {
        node.links = l;
    }
    grid
}

fn cache_path(level: &str) -> PathBuf {
    PathBuf::from(format!("data/cache/nav_{level}_v{CACHE_VERSION}.bin"))
}

fn save(grid: &NavGrid, path: &PathBuf) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(16 + grid.cells.len() * 5 + grid.nodes.len() * 12);
    for v in [grid.origin.x, grid.origin.y] {
        b.extend(v.to_le_bytes());
    }
    for v in [grid.w as u32, grid.h as u32, grid.nodes.len() as u32] {
        b.extend(v.to_le_bytes());
    }
    for &(_, n) in &grid.cells {
        b.push(n);
    }
    for n in &grid.nodes {
        b.extend(n.y.to_le_bytes());
        b.extend(n.links);
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, b)
}

fn load(path: &PathBuf) -> Option<NavGrid> {
    let b = std::fs::read(path).ok()?;
    let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let (w, h, n) = (u(8), u(12), u(16));
    let mut p = 20;
    let mut cells = Vec::with_capacity(w * h);
    let mut start = 0u32;
    for _ in 0..w * h {
        let c = *b.get(p)?;
        cells.push((start, c));
        start += c as u32;
        p += 1;
    }
    let mut nodes = Vec::with_capacity(n);
    for _ in 0..n {
        let y = f(p);
        let mut links = [0u8; 8];
        links.copy_from_slice(b.get(p + 4..p + 12)?);
        nodes.push(Node { y, links });
        p += 12;
    }
    Some(NavGrid { origin: Vec2::new(f(0), f(4)), w, h, cells, nodes })
}

fn build_nav(mut commands: Commands, world: Res<WorldCollision>, doors: Query<&Door>) {
    let level = std::env::args().nth(1).unwrap_or_else(|| "C02-1".to_owned());
    let path = cache_path(&level);
    let t0 = std::time::Instant::now();
    let grid = match load(&path) {
        Some(g) => g,
        None => {
            // Doors stand open for the purpose of navigation.
            let mut open = world.clone();
            for d in &doors {
                for &s in &d.collision_sources {
                    open.disable_source(s);
                }
            }
            let g = build(&open);
            if let Err(e) = save(&g, &path) {
                warn!("could not cache navigation grid: {e}");
            }
            g
        }
    };
    info!("navigation: {} nodes over {}x{} cells in {:.1}s", grid.len(), grid.w, grid.h, t0.elapsed().as_secs_f32());
    commands.insert_resource(grid);
}
