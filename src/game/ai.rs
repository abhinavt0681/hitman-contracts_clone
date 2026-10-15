//! Guard behaviour: perception and suspicion, combat, and patrols along the level's paths.
//!
//! A guard who finds 47 odd stops and watches him (`Mind::Wary`); if 47 slips away, or the
//! guard hears a shot or a body fall, he goes to look (`Investigate`), checks a few spots
//! around it (`Search`) and walks back to his route or post. Past full suspicion he turns
//! hostile, radios it in, and hunts 47 from where he was last seen: break line of sight long
//! enough and the hunt winds down into a search, though the base stays on alert. Light and
//! the blizzard decide how far a guard can make 47 out.

use crate::{
    atmosphere::Exposure,
    character::Gait,
    collision::WorldCollision,
    player::Player,
};

use super::*;
use crate::nav::NavGrid;

/// Guards notice 47 within this distance (m) when he stands in light; `RelaxedVisRange=3000`
/// in their properties.
pub(super) const VIEW_RANGE: f32 = 30.0;
/// …and only this close when he keeps to the dark.
const DARK_RANGE: f32 = 9.0;
/// A full squall takes this share off sight and hearing outdoors.
const GUST_SIGHT: f32 = 0.45;
const GUST_HEARING: f32 = 0.6;

/// Half-angle of a guard's vision cone.
pub(super) const VIEW_HALF_ANGLE: f32 = 1.05;

pub(super) const BODY_SPOT_RANGE: f32 = 18.0;

pub(super) const ALARM_SPREAD: f32 = 35.0;

pub(super) const RIFLE_RANGE: f32 = 22.0;

pub(super) const RIFLE_DPS: f32 = 14.0;

pub(super) const GUARD_RUN_SPEED: f32 = 4.0;
const GUARD_WALK_SPEED: f32 = 1.3;
const GUARD_CHECK_SPEED: f32 = 1.8;

/// Suspicion above which a guard stops to watch 47; losing sight of him then sends the guard
/// to look where he was.
const WARY: f32 = 0.4;
/// A hunting guard makes 47 out at least this far, dark or not.
const HUNT_SIGHT: f32 = 12.0;
/// A hunting guard who has not seen 47 for this long gives up and searches.
const LOSE_TRACK: f32 = 20.0;
/// Seconds spent looking around a spot being checked.
const LINGER: f32 = 4.0;
/// Spots checked around the trouble before going back to the post.
const SEARCH_SPOTS: u8 = 3;
const SEARCH_RADIUS: f32 = 7.0;

/// What 47 looks like to one guard.
pub(super) struct Sighting {
    pub outfit: Outfit,
    /// Unit of the man whose uniform he wears.
    pub unit: Option<&'static str>,
    pub dist: f32,
    /// How far this guard can make out 47 now (light, weather).
    pub range: f32,
    pub running: bool,
    pub drawn_weapon: bool,
    /// The base is on alert: faces are checked.
    pub alarm: bool,
}

/// How fast a guard grows suspicious of 47 (per second).
pub(super) fn suspicion_rate(watcher: &Actor, s: &Sighting) -> f32 {
    if watcher.kind != Kind::Guard {
        return 0.0;
    }
    let dist = s.dist;
    // Drawing a weapon in sight of a guard immediately triggers intense suspicion
    if s.drawn_weapon && dist < s.range {
        return 1.4;
    }
    let close = (1.0 - dist / s.range).clamp(0.0, 1.0);
    let run = if s.running { 2.0 } else { 1.0 };
    let same_unit = s.unit.is_some() && s.unit == unit(&watcher.name);
    match (watcher.outfit, s.outfit) {
        // A stranger in a suit at a military base.
        (Outfit::Soldier, Outfit::Suit | Outfit::Civilian) => 0.25 + 0.9 * close * run,
        // Fuchs's men are busy with the deal; they only mind a stranger right next to them.
        (Outfit::Bodyguard, Outfit::Suit | Outfit::Civilian) if dist < 6.0 => 0.6 * run,
        // His own unit knows the face under that cap.
        (a, b) if a == b && same_unit && dist < 8.0 => 0.25 + 0.45 * (1.0 - dist / 8.0) * run,
        // Same uniform: only odd behaviour up close gives 47 away; on alert, faces get a look.
        (a, b) if a == b => {
            if dist < 2.5 || (s.alarm && dist < 5.0) {
                0.12 + if s.alarm { 0.2 } else { 0.0 }
            } else if s.running && dist < 8.0 {
                0.35
            } else {
                0.0
            }
        }
        (_, Outfit::Fuchs) if watcher.outfit == Outfit::Bodyguard && dist < 3.0 => 0.5,
        _ if s.running && dist < 6.0 => 0.2,
        _ => 0.0,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn perceive(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<WorldCollision>,
    text: Res<Text>,
    exposure: Res<Exposure>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut mission: ResMut<Mission>,
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor)>,
) {
    let noises = std::mem::take(&mut mission.noises);
    if mission.outcome.is_some() {
        return;
    }
    let dt = time.delta_secs();
    mission.elapsed += dt;
    let Ok((pt, player)) = player.single() else { return };
    let feet = pt.translation();
    let target = if player.crouching { feet + Vec3::Y * 0.75 } else { feet + Vec3::Y * 1.2 };
    let running = player.speed() > 2.5;
    // Fiber wire, poison and the remote stay hidden in a pocket; only a drawn gun is visible.
    let drawn_weapon = mission.active_item == Item::SilencedPistol;
    let is_dragging = mission.dragging_body.is_some();
    // Lamps and squalls: in the dark guards make 47 out only close up, and a squall blinds and
    // deafens them for a few seconds.
    let sight = DARK_RANGE.lerp(VIEW_RANGE, exposure.light) * (1.0 - GUST_SIGHT * exposure.gust);
    let hearing = 1.0 - GUST_HEARING * exposure.gust;

    let bodies: Vec<(Entity, Vec3, String)> = actors
        .iter()
        .filter(|(e, _, a)| !a.alive && !a.found && Some(*e) != mission.dragging_body)
        .map(|(e, t, a)| (e, t.translation(), a.name.clone()))
        .collect();
    let mut found_bodies = Vec::new();
    // Where the trouble is, for the guards who get the word.
    let mut alarm_at = None;

    for (_, t, mut a) in &mut actors {
        if !a.alive || a.kind != Kind::Guard {
            continue;
        }
        let (eye, fwd) = npc_view(t);
        // Bodies in view.
        for (body, pos, name) in &bodies {
            let to = *pos + Vec3::Y * 0.4 - eye;
            if to.length() < BODY_SPOT_RANGE && fwd.angle_between(to.with_y(0.0)) < VIEW_HALF_ANGLE && line_of_sight(&world, eye, *pos + Vec3::Y * 0.4) {
                found_bodies.push((*body, name.clone()));
                a.alerted = true;
                a.last_seen = Some(*pos);
                alarm_at = Some(*pos);
            }
        }
        // 47 in view.
        let to = target - eye;
        let dist = to.length();
        let in_cone = fwd.angle_between(to.with_y(0.0)) < VIEW_HALF_ANGLE;
        // Sneaking makes footsteps completely silent; guards cannot detect someone behind them unless physically bumped (<0.5m)
        let heard_behind = if player.crouching {
            dist < 0.5
        } else if running {
            dist < 10.0 * hearing
        } else if is_dragging {
            dist < 3.2 * hearing
        } else {
            dist < 1.8 * hearing
        };
        // Hunting guards scan all around; and anyone sees a man walking into him.
        let range = if a.alerted { sight.max(HUNT_SIGHT) } else { sight }.max(2.5);
        let visible = dist < range && (in_cone || heard_behind || a.alerted) && line_of_sight(&world, eye, target);
        if a.alerted {
            if visible {
                a.last_seen = Some(feet);
                a.lost_for = 0.0;
            }
            continue;
        }
        let sighting = Sighting {
            outfit: mission.outfit,
            unit: mission.disguise_unit,
            dist,
            range,
            running,
            // Guards only notice drawn weapons if 47 is in their vision cone (can't see behind their back)
            drawn_weapon: drawn_weapon && in_cone,
            alarm: mission.alarm,
        };
        let mut rate = if !visible {
            0.0
        } else if is_dragging {
            1.5 // dragging a corpse right in front of a guard triggers instant alarm!
        } else {
            suspicion_rate(&a, &sighting)
        };
        if player.crouching {
            rate *= 0.35; // sneaking reduces suspicion accumulation rate in view
        }
        // A guard already on edge, or a base on alert, is quicker to make up his mind.
        if matches!(a.mind, Mind::Investigate { .. } | Mind::Search { .. }) || mission.alarm {
            rate *= 1.4;
        }
        a.suspicion = if rate > 0.0 { a.suspicion + rate * dt } else { (a.suspicion - 0.15 * dt).max(0.0) };
        if a.suspicion >= 1.0 {
            a.alerted = true;
            a.last_seen = Some(feet);
            a.lost_for = 0.0;
            alarm_at = Some(feet);
            continue;
        }
        // Short of that: watch him, and go and look when he slips out of sight.
        if visible && rate > 0.0 && a.suspicion > WARY {
            a.mind = Mind::Wary;
            a.last_seen = Some(feet);
        } else if a.mind == Mind::Wary {
            if !visible {
                a.mind = Mind::Investigate { spot: a.last_seen.unwrap_or(feet), linger: LINGER };
            } else if a.suspicion < WARY * 0.5 {
                a.mind = Mind::Calm;
            }
        }
        // Noises: a shot, a body hitting the snow.
        for &(at, reach) in &noises {
            if t.translation().distance(at) < reach * hearing && !matches!(a.mind, Mind::Wary) {
                a.mind = Mind::Investigate { spot: at, linger: LINGER };
                a.suspicion = a.suspicion.max(WARY * 0.75);
            }
        }
    }

    let sb_ref = sound_bank.as_deref();
    for (body, name) in found_bodies {
        if let Ok((_, _, mut a)) = actors.get_mut(body) {
            a.found = true;
        }
        let msg = text.0.get(&name).or_else(|| text.0.get(name.split('_').next().unwrap_or(""))).map(str::to_owned);
        raise_alarm(Some(&mut commands), sb_ref, &mut mission, &text, msg.as_deref());
    }
    if let Some(at) = alarm_at {
        raise_alarm(Some(&mut commands), sb_ref, &mut mission, &text, None);
        // Word spreads to guards nearby, with where to look.
        let alerted: Vec<Vec3> = actors.iter().filter(|(_, _, a)| a.alerted && a.alive).map(|(_, t, _)| t.translation()).collect();
        for (_, t, mut a) in &mut actors {
            if a.alive && !a.alerted && a.kind == Kind::Guard && alerted.iter().any(|p| p.distance(t.translation()) < ALARM_SPREAD) {
                a.alerted = true;
                a.last_seen = Some(at);
                a.lost_for = 0.0;
            }
        }
    }
}

/// Path following for NPCs: A* over the navigation grid when available, replanned when the
/// goal moves or every so often, with wall push-out and ground snapping on top.
#[derive(Component, Default)]
pub struct Steering {
    path: Vec<Vec3>,
    goal: Option<Vec3>,
    replan: f32,
}

const REPLAN_SECONDS: f32 = 1.0;
const REPLAN_DISTANCE: f32 = 1.5;

/// Moves `t` toward `goal` at `speed`; returns the remaining straight-line distance.
fn steer(t: &mut Transform, s: &mut Steering, goal: Vec3, speed: f32, dt: f32, nav: Option<&NavGrid>, world: &WorldCollision) -> f32 {
    s.replan -= dt;
    // Static goals (patrol nodes) are planned once; moving ones (47) every so often.
    let jumped = s.goal.is_none_or(|g| g.distance(goal) > REPLAN_DISTANCE);
    let drifting = s.goal.is_some_and(|g| g.distance(goal) > 0.3);
    if s.path.is_empty() || jumped || (drifting && s.replan <= 0.0) {
        let t0 = std::time::Instant::now();
        s.path = nav.and_then(|n| n.path(t.translation, goal, world)).unwrap_or_else(|| vec![goal]);
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        if ms > 5.0 {
            debug!("path search took {ms:.1} ms");
        }
        // The first point is where we stand.
        if s.path.len() > 1 {
            s.path.remove(0);
        }
        s.goal = Some(goal);
        s.replan = REPLAN_SECONDS;
    }
    while s.path.len() > 1 && (s.path[0] - t.translation).with_y(0.0).length() < 0.5 {
        s.path.remove(0);
    }
    let next = s.path.first().copied().unwrap_or(goal);
    let to = (next - t.translation).with_y(0.0);
    let dist = to.length();
    if dist > 0.05 {
        let dir = to / dist;
        t.rotation = t.rotation.slerp(facing(dir), (6.0 * dt).min(1.0));
        let mut pos = t.translation + dir * (speed * dt).min(dist);
        pos += world.push_out_horizontal(pos + Vec3::Y * 1.0, 0.3);
        if let Some(h) = world.raycast(pos + Vec3::Y * 0.8, Vec3::NEG_Y, 2.0) {
            pos.y = pos.y + 0.8 - h.distance;
        }
        t.translation = pos;
    }
    (goal - t.translation).with_y(0.0).length()
}

pub(super) fn guard_combat(
    time: Res<Time>,
    world: Res<WorldCollision>,
    nav: Option<Res<NavGrid>>,
    exposure: Res<Exposure>,
    mut mission: ResMut<Mission>,
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(&mut Transform, &GlobalTransform, &mut Actor, Option<&mut Gait>, &mut Steering)>,
) {
    if mission.outcome.is_some() {
        return;
    }
    let dt = time.delta_secs();
    let Ok((pt, player)) = player.single() else { return };
    let target = pt.translation();
    // A running man in the dark, in a squall, is hard to hit.
    let hit_chance = (if player.speed() > 2.5 { 0.5 } else { 1.0 }) * (0.55 + 0.45 * exposure.light) * (1.0 - 0.3 * exposure.gust);
    let mut damage = 0.0;
    for (mut t, gt, mut a, gait, mut steering) in &mut actors {
        if !a.alive || !a.alerted || a.kind != Kind::Guard {
            continue;
        }
        let Some(mut gait) = gait else { continue };
        let dist = (target - t.translation).with_y(0.0).length();
        let (eye, _) = npc_view(gt);
        // `perceive` resets `lost_for` while he has 47 in sight.
        let sees = a.lost_for == 0.0 && line_of_sight(&world, eye, target + Vec3::Y * 1.2);
        a.lost_for += dt;
        if sees && dist < RIFLE_RANGE * 0.7 {
            let to = (target - t.translation).with_y(0.0);
            if to.length() > 0.1 {
                t.rotation = facing(to.normalize());
            }
            gait.speed = 0.0;
            if gait.idle != "Shoot_Pose_Rifle01" {
                a.calm_idle = Some(std::mem::replace(&mut gait.idle, "Shoot_Pose_Rifle01".to_owned()));
            }
            a.fire_cooldown -= dt;
            if a.fire_cooldown <= 0.0 {
                a.fire_cooldown = 0.25;
            }
            // Accuracy falls off with range.
            damage += RIFLE_DPS * hit_chance * (1.0 - dist / RIFLE_RANGE).clamp(0.2, 1.0) * dt;
            continue;
        }
        if let Some(idle) = a.calm_idle.take() {
            gait.idle = idle;
        }
        // Run to where he was last seen, along a real route; lose him, and search there.
        let goal = a.last_seen.unwrap_or(t.translation);
        let left = steer(&mut t, &mut steering, goal, GUARD_RUN_SPEED, dt, nav.as_deref(), &world);
        gait.speed = if left > 1.0 { GUARD_RUN_SPEED } else { 0.0 };
        if a.lost_for > LOSE_TRACK || (left < 1.0 && a.lost_for > LINGER) {
            a.alerted = false;
            a.suspicion = 0.7;
            a.mind = Mind::Search { centre: goal, spot: goal, left: SEARCH_SPOTS + 1, linger: LINGER };
        }
    }
    mission.health -= damage;
}

/// A random walkable spot within `SEARCH_RADIUS` of `centre`, reachable from `from`.
fn search_spot(centre: Vec3, from: Vec3, seed: &mut u32, nav: Option<&NavGrid>, world: &WorldCollision) -> Vec3 {
    let Some(nav) = nav else { return centre };
    for _ in 0..6 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        let a = (*seed & 0xffff) as f32 / 65535.0 * std::f32::consts::TAU;
        let r = (*seed >> 16) as f32 / 65535.0 * SEARCH_RADIUS;
        if let Some(p) = nav.snap(centre + Vec3::new(a.cos() * r, 0.0, a.sin() * r)) {
            if (p.y - centre.y).abs() < 2.0 && nav.path(from, p, world).is_some() {
                return p;
            }
        }
    }
    centre
}

pub(super) fn actor_patrol(
    time: Res<Time>,
    world: Res<WorldCollision>,
    nav: Option<Res<NavGrid>>,
    mission: Option<Res<Mission>>,
    player: Query<&GlobalTransform, With<Player>>,
    mut seed: Local<u32>,
    mut actors: Query<(&mut Transform, &mut Actor, Option<&mut Gait>, &mut Patrol, &mut Steering)>,
) {
    if mission.is_some_and(|m| m.outcome.is_some()) {
        return;
    }
    if *seed == 0 {
        *seed = 0x1234_5679;
    }
    let dt = time.delta_secs();
    let me = player.single().map(|p| p.translation()).ok();
    for (mut t, mut a, gait, mut patrol, mut steering) in &mut actors {
        if !a.alive || a.alerted {
            continue;
        }
        let Some(mut gait) = gait else { continue };
        if a.kind == Kind::Guard && patrol.post.is_none() {
            patrol.post = Some((t.translation, t.rotation));
        }

        match a.mind {
            Mind::Calm => {}
            Mind::Wary => {
                // Stop and stare.
                if let Some(to) = me.map(|p| (p - t.translation).with_y(0.0)).filter(|d| d.length() > 0.1) {
                    t.rotation = t.rotation.slerp(facing(to.normalize()), (4.0 * dt).min(1.0));
                }
                gait.speed = 0.0;
                continue;
            }
            Mind::Investigate { spot, ref mut linger } => {
                if steer(&mut t, &mut steering, spot, GUARD_CHECK_SPEED, dt, nav.as_deref(), &world) > 1.2 {
                    gait.speed = GUARD_CHECK_SPEED;
                } else {
                    gait.speed = 0.0;
                    t.rotate_y(0.9 * dt);
                    *linger -= dt;
                    if *linger <= 0.0 {
                        a.mind = Mind::Search { centre: spot, spot, left: SEARCH_SPOTS + 1, linger: 0.0 };
                    }
                }
                continue;
            }
            Mind::Search { centre, ref mut spot, ref mut left, ref mut linger } => {
                if steer(&mut t, &mut steering, *spot, GUARD_CHECK_SPEED, dt, nav.as_deref(), &world) > 1.2 {
                    gait.speed = GUARD_CHECK_SPEED;
                } else {
                    gait.speed = 0.0;
                    t.rotate_y(-0.9 * dt);
                    *linger -= dt;
                    if *linger <= 0.0 {
                        *left = left.saturating_sub(1);
                        if *left == 0 {
                            a.mind = Mind::Calm;
                            a.suspicion = a.suspicion.min(WARY * 0.5);
                        } else {
                            *spot = search_spot(centre, t.translation, &mut seed, nav.as_deref(), &world);
                            *linger = LINGER * 0.6;
                        }
                    }
                }
                continue;
            }
        }

        if patrol.nodes.len() < 2 {
            // A sentry: back to his post if something drew him away.
            if let Some((post, facing_post)) = patrol.post {
                if steer(&mut t, &mut steering, post, GUARD_WALK_SPEED, dt, nav.as_deref(), &world) > 0.6 {
                    gait.speed = GUARD_WALK_SPEED;
                } else {
                    gait.speed = 0.0;
                    t.rotation = t.rotation.slerp(facing_post, (3.0 * dt).min(1.0));
                }
            }
            continue;
        }
        if patrol.wait_timer > 0.0 {
            patrol.wait_timer -= dt;
            gait.speed = 0.0;
            continue;
        }

        let (target, pause) = patrol.nodes[patrol.current];
        if steer(&mut t, &mut steering, target, GUARD_WALK_SPEED, dt, nav.as_deref(), &world) < 0.8 {
            patrol.wait_timer = pause;
            patrol.advance();
            gait.speed = 0.0;
            continue;
        }
        gait.speed = GUARD_WALK_SPEED;
    }
}

/// Logs guards' changes of mind (`RUST_LOG=hitman_clone::game=debug`).
pub(super) fn log_minds(mut seen: Local<HashMap<Entity, String>>, actors: Query<(Entity, &GlobalTransform, &Actor)>) {
    for (e, t, a) in &actors {
        if a.kind != Kind::Guard {
            continue;
        }
        let state = match (a.alive, a.alerted, a.mind) {
            (false, ..) => "dead".to_owned(),
            (_, true, _) => "hostile".to_owned(),
            (_, _, Mind::Investigate { spot, .. }) => format!("investigate {:.0?}", spot.to_array()),
            (_, _, Mind::Search { left, .. }) => format!("search ({left} left)"),
            (_, _, m) => format!("{m:?}"),
        };
        let state = state.split(" (").next().unwrap_or_default().to_owned();
        if seen.get(&e) != Some(&state) {
            if !seen.contains_key(&e) {
                trace!("{} posted at {:.1?}", a.name, t.translation().to_array());
            } else {
                debug!("{} at {:.0?}: {state}", a.name, t.translation().to_array());
            }
            seen.insert(e, state);
        }
    }
}
