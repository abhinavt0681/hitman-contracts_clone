//! What 47 does: item switching, close kills, outfits, bodies, doors, items, the pistol and
//! the bomb remote.

use crate::{
    character::{self, AnimLibrary, Animator, CharacterAssets, Gait},
    collision::WorldCollision,
    player::{OrbitCamera, Player},
};
use bevy::{
    input::mouse::MouseButton,
    mesh::skinning::SkinnedMeshInverseBindposes,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use super::*;

pub(super) const GUNSHOT_HEARING: f32 = 30.0;
/// Guards this close to a silenced shot know it for what it is.
pub(super) const GUNSHOT_ALARM: f32 = 5.0;
/// A body dropped in the snow carries this far (m).
pub(super) const BODY_DROP_HEARING: f32 = 7.0;

pub(super) const REACH: f32 = 1.6;

/// Head parts sit entirely above this height (cm) in the bind pose.
pub(super) const HEAD_MIN_HEIGHT: f32 = 140.0;

#[allow(clippy::too_many_arguments)]
pub(super) fn player_actions(
    mut commands: Commands,
    inputs: (Res<ButtonInput<KeyCode>>, Option<Res<bevy::input::mouse::AccumulatedMouseScroll>>),
    time: Res<Time>,
    world: Res<WorldCollision>,
    text: Res<Text>,
    lib: Res<AnimLibrary>,
    wardrobe: Res<Wardrobe>,
    textures: Res<Textures>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut anchors: ResMut<Anchors>,
    mut mission: ResMut<Mission>,
    mut assets: (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>, ResMut<Assets<SkinnedMeshInverseBindposes>>),
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
    mut player_anim: Query<&mut Animator, Without<Actor>>,
    mut doors: Query<&mut Door>,
) {
    let (keys, mouse_scroll) = (&inputs.0, &inputs.1);
    // Weapon / Item switching with keys 1..=5 and mouse wheel
    let items = [
        Item::Holstered,
        Item::FiberWire,
        Item::SilencedPistol,
        Item::PoisonVial,
        Item::BombRemote,
    ];
    let mut switch = if keys.just_pressed(KeyCode::Digit1) {
        Some(Item::Holstered)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(Item::FiberWire)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(Item::SilencedPistol)
    } else if keys.just_pressed(KeyCode::Digit4) {
        Some(Item::PoisonVial)
    } else if keys.just_pressed(KeyCode::Digit5) {
        Some(Item::BombRemote)
    } else {
        None
    };

    if switch.is_none() {
        if let Some(scroll) = &mouse_scroll {
            if scroll.delta.y.abs() > 0.1 {
                let current_idx = items.iter().position(|&i| i == mission.active_item).unwrap_or(0);
                let next_idx = if scroll.delta.y > 0.0 {
                    (current_idx + items.len() - 1) % items.len()
                } else {
                    (current_idx + 1) % items.len()
                };
                switch = Some(items[next_idx]);
            }
        }
    }

    if let Some(item) = switch {
        if item != mission.active_item {
            mission.active_item = item;
            let label = item.label();
            mission.say(format!("Equipped: {label}"));
            if let Some(sb) = &sound_bank {
                if item == Item::Holstered {
                    crate::audio::play_sound(&mut commands, &sb.gun_holster, 0.5);
                } else {
                    crate::audio::play_sound(&mut commands, &sb.gun_pull, 0.5);
                }
            }
        }
    }

    mission.busy = (mission.busy - time.delta_secs()).max(0.0);
    mission.prompt = None;
    if mission.outcome.is_some() || mission.busy > 0.0 {
        return;
    }
    let Ok((pt, player)) = player.single() else { return };
    let me = pt.translation();
    let use_key = keys.just_pressed(KeyCode::KeyE);

    // Drop currently dragged body
    if mission.dragging_body.is_some() {
        mission.prompt = Some("[G] Drop body".to_owned());
        if keys.just_pressed(KeyCode::KeyG) {
            mission.dragging_body = None;
            mission.say("Body dropped");
            mission.noises.push((me, BODY_DROP_HEARING));
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.body_drop, 0.6);
            }
            return;
        }
    }

    // Closest actor in reach.
    let near = actors
        .iter()
        .map(|(e, t, a, _)| (e, t.translation().distance(me), t, a))
        .filter(|(_, d, _, _)| *d < REACH)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _, t, a)| (e, *t, a.alive, a.stripped, a.outfit, a.kind, a.name.clone()));

    if let Some((e, t, alive, stripped, outfit, kind, name)) = near {
        let (_, fwd) = npc_view(&t);
        let behind = fwd.dot((me - t.translation()).with_y(0.0).normalize_or_zero()) < -0.2;
        if alive && behind {
            let is_poison = mission.active_item == Item::PoisonVial;
            match mission.active_item {
                Item::PoisonVial => mission.prompt = Some("[E] Inject Cyanide Poison".to_owned()),
                Item::FiberWire => mission.prompt = Some("[E] Strangle with Fiber Wire".to_owned()),
                _ => {
                    // As in the original: the kill needs the right tool in hand.
                    mission.prompt = Some("Equip Fiber Wire [2] or Poison [4] to take them out".to_owned());
                    return;
                }
            }
            if use_key {
                if let Some(sb) = &sound_bank {
                    if is_poison {
                        crate::audio::play_sound(&mut commands, &sb.clothes_touch, 0.6);
                    } else {
                        crate::audio::play_sound(&mut commands, &sb.fiber_wire, 0.7);
                    }
                }
                if let Ok((_, gt, mut a, mut anim)) = actors.get_mut(e) {
                    a.alive = false;
                    let clip = lib.first_of(anim.set(), &["Die_Throat_Cut_Alt_01", "Die_Forward"]).unwrap_or_default();
                    anim.play_once_at(&clip, 2.5);
                    commands.entity(e).remove::<Gait>();
                    // Anyone watching sees the kill.
                    let spot = gt.translation();
                    if actors.iter().any(|(o, ot, oa, _)| {
                        o != e && oa.alive && oa.kind == Kind::Guard && {
                            let (eye, f) = npc_view(ot);
                            let to = spot + Vec3::Y - eye;
                            to.length() < 20.0 && f.angle_between(to.with_y(0.0)) < VIEW_HALF_ANGLE && line_of_sight(&world, eye, spot + Vec3::Y)
                        }
                    }) {
                        raise_alarm(Some(&mut commands), sound_bank.as_deref(), &mut mission, &text, None);
                    }
                }
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Cut_Throat");
                }
                mission.busy = 1.6;
                on_kill(&mut mission, &text, kind, &name);
            }
            return;
        }

        if !alive && mission.dragging_body.is_none() {
            let can_take = !stripped && outfit.templates().0.is_some_and(|t| wardrobe.0.contains_key(t)) && outfit != mission.outfit;
            if can_take {
                mission.prompt = Some(format!("[E] Take outfit: {}   |   [G] Drag body", outfit.label()));
            } else {
                mission.prompt = Some("[G] Drag body".to_owned());
            }

            if keys.just_pressed(KeyCode::KeyG) {
                mission.dragging_body = Some(e);
                mission.say("Dragging body (press G to drop)");
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.body_drag, 0.6);
                }
                return;
            }

            if can_take && use_key {
                let (wear, nude) = outfit.templates();
                let mut cast = CharacterAssets {
                    meshes: &mut assets.0,
                    materials: &mut assets.1,
                    images: &mut assets.2,
                    bindposes: &mut assets.3,
                    textures: &textures.0,
                };
                if let Some(model) = wear.and_then(|w| wardrobe.0.get(w)) {
                    let root = player.model();
                    commands.entity(root).despawn_children().remove::<character::Rig>();
                    character::attach(&mut commands, root, model, &mut cast);
                }
                if let Some(body) = nude.and_then(|n| wardrobe.0.get(n)) {
                    // Stripped bodies have no head of their own: keep the victim's.
                    let model = match wardrobe.0.get(&name) {
                        Some(own) => body.with_parts_from(own, |p| p.min_height() > HEAD_MIN_HEIGHT),
                        None => body.clone(),
                    };
                    commands.entity(e).despawn_children().remove::<character::Rig>();
                    character::attach(&mut commands, e, &model, &mut cast);
                }
                if let Ok((_, _, mut a, _)) = actors.get_mut(e) {
                    a.stripped = true;
                }
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Change_Clothes");
                }
                mission.busy = 2.0;
                mission.outfit = outfit;
                mission.disguise_unit = unit(&name);
                mission.say(format!("Disguised as {}", outfit.label()));
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.clothes_touch, 0.7);
                    crate::audio::play_sound(&mut commands, &sb.osd_hint, 0.6);
                }
                return;
            }
        }
    }

    // Door interaction
    let mut closest_door: Option<(Mut<Door>, f32)> = None;
    for door in &mut doors {
        let d = door.interaction_point.distance(me);
        if d < 2.5 {
            if closest_door.as_ref().map_or(true, |(_, best)| d < *best) {
                closest_door = Some((door, d));
            }
        }
    }
    if let Some((mut door, _)) = closest_door {
        if mission.prompt.is_none() {
            if !door.is_open {
                mission.prompt = Some("[E] Open door".to_owned());
            } else {
                mission.prompt = Some("[E] Close door".to_owned());
            }
            if use_key {
                door.is_open = !door.is_open;
                door.target = if door.is_open { 1.0 } else { 0.0 };
                if door.is_open {
                    door.auto_close_timer = 7.0;
                }
                if let Some(sb) = &sound_bank {
                    let is_metal = door.name.contains("Metal") || door.name.contains("DDoor") || door.name.contains("Fence");
                    let snd = if door.is_open {
                        if is_metal { &sb.door_metal_open } else { &sb.door_wood_open }
                    } else {
                        if is_metal { &sb.door_metal_close } else { &sb.door_wood_close }
                    };
                    crate::audio::play_sound(&mut commands, snd, 0.65);
                }
                mission.say(if door.is_open { "Door opened" } else { "Door closed" });
                return;
            }
        }
    }

    // Radiation suit pickup in northwest warehouse
    if me.distance(anchors.radiation_suit) < 2.5 && mission.outfit != Outfit::BombWorker {
        mission.prompt = Some("[E] Change into Radiation Suit".to_owned());
        if use_key {
            let (wear, _) = Outfit::BombWorker.templates();
            let mut cast = CharacterAssets {
                meshes: &mut assets.0,
                materials: &mut assets.1,
                images: &mut assets.2,
                bindposes: &mut assets.3,
                textures: &textures.0,
            };
            if let Some(model) = wear.and_then(|w| wardrobe.0.get(w)) {
                let root = player.model();
                commands.entity(root).despawn_children().remove::<character::Rig>();
                character::attach(&mut commands, root, model, &mut cast);
            }
            if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                anim.play_once("Change_Clothes");
            }
            mission.busy = 2.0;
            mission.outfit = Outfit::BombWorker;
            mission.disguise_unit = None;
            mission.say("Disguised as Bomb Worker (Radiation Suit)");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.clothes_touch, 0.7);
                crate::audio::play_sound(&mut commands, &sb.osd_hint, 0.6);
            }
            return;
        }
    }

    // Soup poisoning in mess hall kitchen
    if me.distance(anchors.soup_pot) < 2.5 && !mission.soup_poisoned {
        if mission.active_item == Item::PoisonVial {
            mission.prompt = Some("[E] Poison soup with cyanide".to_owned());
            if use_key {
                mission.soup_poisoned = true;
                mission.soup_timer = Some(12.0);
                mission.busy = 2.0;
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Pick_Up_RightHand");
                }
                mission.say("Poisoned the soup with cyanide");
                return;
            }
        } else {
            mission.prompt = Some("Equip Cyanide Poison [4] to poison soup".to_owned());
        }
    }

    // Bjarkhov office vodka poisoning
    if me.distance(anchors.bjarkhov_desk) < 3.5 && mission.bjarkhov_meeting_state >= 2 && !mission.bjarkhov_vodka_poisoned {
        if mission.active_item == Item::PoisonVial {
            mission.prompt = Some("[E] Poison Bjarkhov's vodka with cyanide".to_owned());
            if use_key {
                mission.bjarkhov_vodka_poisoned = true;
                mission.busy = 1.5;
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Poison_Container");
                }
                mission.say("Poisoned Commander Bjarkhov's vodka with cyanide");
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.gun_pull, 0.5);
                }
                return;
            }
        } else {
            mission.prompt = Some("Equip Cyanide Poison [4] to poison vodka".to_owned());
        }
    }

    // Items and places.
    if let Some(i) = anchors.tnt.iter().position(|(p, taken)| !taken && p.distance(me) < REACH) {
        mission.prompt = Some("[E] Pick up dynamite".to_owned());
        if use_key {
            anchors.tnt[i].1 = true;
            mission.tnt += 1;
            mission.say("Picked up dynamite");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.gun_pull, 0.4);
            }
        }
        return;
    }
    if let Some(i) = anchors.bomb_sites.iter().position(|(p, placed)| !placed && p.xz().distance(me.xz()) < 2.5 && (p.y - me.y).abs() < 2.0) {
        if mission.tnt == 0 {
            mission.prompt = Some("You need dynamite (there is some on board)".to_owned());
        } else {
            mission.prompt = Some(format!("[E] {}", text.0.get("PlaceBomb").unwrap_or("Place bomb")));
            if use_key {
                mission.tnt -= 1;
                anchors.bomb_sites[i].1 = true;
                mission.sub_bombs_placed += 1;
                mission.busy = 1.5;
                if let Ok(mut anim) = player_anim.get_mut(player.model()) {
                    anim.play_once("Place_Bomb_Under");
                }
                match mission.sub_bombs_placed {
                    1 => mission.say(text.0.get("1SubBombPlaced").unwrap_or("1st of 3 bombs has been placed on submarine.")),
                    2 => mission.say(text.0.get("2SubBombPlaced").unwrap_or("2nd of 3 bombs placed on submarine.")),
                    _ => {
                        mission.say(text.0.get("AllSubBombsPlaced").unwrap_or("All bombs placed on submarine."));
                        mission.say("Equip Bomb Remote [5] and detonate from safe distance");
                    }
                }
            }
        }
        return;
    }
    let exit_to = anchors.exit - me;
    if exit_to.with_y(0.0).length() < 4.5 && exit_to.y.abs() < 4.0 {
        if mission.objectives[..3].iter().all(|o| o.done) {
            mission.prompt = Some("[E] Board the cargo plane".to_owned());
            if use_key {
                mission.objectives[3].done = true;
                mission.outcome = Some(Outcome::Complete);
            }
        } else {
            mission.prompt = Some(text.0.get("ObjectiveNotComplete").unwrap_or("Objectives not completed").to_owned());
        }
    }
}

pub(super) fn update_dragged_body(
    mut commands: Commands,
    time: Res<Time>,
    world: Res<WorldCollision>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut mission: ResMut<Mission>,
    player: Query<(&GlobalTransform, &Player)>,
    mut actors: Query<(&mut Transform, &Actor)>,
) {
    let Some(drag_entity) = mission.dragging_body else { return };
    let dt = time.delta_secs();
    let Ok((pt, player)) = player.single() else { return };
    let fwd = (pt.rotation() * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
    let drag_pos = pt.translation() - fwd * 0.95;

    if let Ok((mut body_t, _)) = actors.get_mut(drag_entity) {
        let mut target = drag_pos;
        if let Some(h) = world.raycast(target + Vec3::Y * 0.8, Vec3::NEG_Y, 2.0) {
            target.y = target.y + 0.8 - h.distance;
        }
        body_t.translation = target;
        body_t.rotation = pt.rotation();
    }

    if player.speed() > 0.2 {
        mission.drag_timer += dt;
        if mission.drag_timer >= 1.1 {
            mission.drag_timer = 0.0;
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.body_drag, 0.45);
            }
        }
    }
}

/// Silenced pistol or Bomb Remote: left click once the mouse is captured (or [E] for detonator).
#[allow(clippy::too_many_arguments)]
pub(super) fn shoot(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    world: Res<WorldCollision>,
    text: Res<Text>,
    lib: Res<AnimLibrary>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    anchors: Res<Anchors>,
    mut mission: ResMut<Mission>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    camera: Query<&GlobalTransform, With<OrbitCamera>>,
    player: Query<&GlobalTransform, With<Player>>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
) {
    let triggered = mouse.just_pressed(MouseButton::Left)
        || (mission.active_item == Item::BombRemote && keys.just_pressed(KeyCode::KeyE));
    if mission.outcome.is_some() || mission.busy > 0.0 || !triggered {
        return;
    }
    if cursor.single().is_ok_and(|c| c.grab_mode == CursorGrabMode::None) && !keys.just_pressed(KeyCode::KeyE) {
        return; // this click captures the mouse
    }

    if mission.active_item == Item::BombRemote {
        if mission.sub_bombs_placed < 3 {
            mission.say("Need to place all 3 bombs on submarine before detonation");
        } else if !mission.objectives[2].done {
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.sub_explode, 1.0);
            }
            mission.objectives[2].done = true;
            mission.alarm = true;
            mission.say("Submarine destroyed! Objective completed.");
            if let Ok(pt) = player.single() {
                if pt.translation().distance(anchors.sub.0) < 22.0 {
                    mission.health -= 60.0;
                    mission.say("You were caught in the explosion blast!");
                }
            }
            for (e, gt, mut a, mut anim) in &mut actors {
                if a.alive && anchors.bomb_sites.iter().any(|(p, _)| gt.translation().distance(*p) < 20.0) {
                    a.alive = false;
                    let clip = lib.first_of(anim.set(), &["Die_Backward", "Die_Forward"]).unwrap_or_default();
                    anim.play_once(&clip);
                    commands.entity(e).remove::<Gait>();
                }
            }
            if mission.objectives[..3].iter().all(|o| o.done) {
                mission.say(text.0.get("HeadForExit").unwrap_or("Head for your EXIT point").to_owned());
            }
        }
        return;
    }

    if mission.active_item != Item::SilencedPistol {
        return;
    }

    let Ok(cam) = camera.single() else { return };
    let (origin, dir) = (cam.translation(), cam.forward().as_vec3());

    if let Some(sb) = &sound_bank {
        crate::audio::play_sound(&mut commands, &sb.shot_silenced, 0.75);
    }

    let wall = world.raycast(origin, dir, 200.0).map_or(200.0, |h| h.distance);
    // Ray against a capsule-ish target: closest approach to the actor's spine.
    let hit = actors
        .iter()
        .filter(|(_, _, a, _)| a.alive)
        .filter_map(|(e, t, _, _)| {
            let base = t.translation();
            (0..4)
                .filter_map(|k| {
                    let c = base + Vec3::Y * (0.4 + 0.4 * k as f32);
                    let along = (c - origin).dot(dir);
                    (along > 0.0 && along < wall && (origin + dir * along).distance(c) < 0.3).then_some(along)
                })
                .reduce(f32::min)
                .map(|d| (e, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((e, _)) = hit {
        if let Ok((_, _, mut a, mut anim)) = actors.get_mut(e) {
            a.alive = false;
            let clip = lib.first_of(anim.set(), &["Die_Backward", "Die_Forward"]).unwrap_or_default();
            anim.play_once(&clip);
            commands.entity(e).remove::<Gait>();
            let (kind, name) = (a.kind, a.name.clone());
            on_kill(&mut mission, &text, kind, &name);
        }
    }
    // Silenced, but not silent: anyone right there knows it for a shot; further off it is a
    // noise worth checking.
    let Ok(pt) = player.single() else { return };
    let me = pt.translation();
    let mut heard = false;
    for (_, t, mut a, _) in &mut actors {
        if a.alive && a.kind == Kind::Guard && t.translation().distance(me) < GUNSHOT_ALARM {
            a.alerted = true;
            a.last_seen = Some(me);
            heard = true;
        }
    }
    mission.noises.push((me, GUNSHOT_HEARING * 0.5));
    if heard {
        raise_alarm(Some(&mut commands), sound_bank.as_deref(), &mut mission, &text, None);
    }
}
