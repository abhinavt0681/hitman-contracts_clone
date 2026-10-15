//! Doors: leaves that swing open for 47 and NPCs, with their collision switched off while open.

use crate::{
    collision::WorldCollision,
    player::Player,
};

use super::*;

#[derive(Component, Clone, Debug)]
#[allow(dead_code)]
pub struct DoorLeaf {
    pub source_id: u32,
    pub closed_rotation: Quat,
    pub open_rotation: Quat,
}

#[derive(Component, Clone, Debug)]
#[allow(dead_code)]
pub struct Door {
    pub name: String,
    pub interaction_point: Vec3,
    pub is_open: bool,
    pub progress: f32,
    pub target: f32,
    pub auto_close_timer: f32,
    pub leaves: Vec<Entity>,
    pub collision_sources: Vec<u32>,
}

pub fn update_doors(
    mut commands: Commands,
    time: Res<Time>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut collision: ResMut<WorldCollision>,
    mut doors: Query<&mut Door>,
    mut leaves: Query<(&DoorLeaf, &mut Transform)>,
    actors: Query<(&GlobalTransform, &Actor)>,
    player: Query<&GlobalTransform, With<Player>>,
) {
    let dt = time.delta_secs();
    let player_pos = player.iter().next().map(|gt| gt.translation());

    for mut door in &mut doors {
        // NPCs approaching closed doors automatically open them
        if !door.is_open {
            let door_pos = door.interaction_point;
            for (gt, actor) in &actors {
                if actor.alive && gt.translation().distance(door_pos) < 2.2 {
                    door.is_open = true;
                    door.target = 1.0;
                    door.auto_close_timer = 5.0;
                    if let Some(sb) = &sound_bank {
                        if let Some(ppos) = player_pos {
                            let dist = ppos.distance(door_pos);
                            const MAX_DOOR_HEAR_DIST: f32 = 18.0;
                            if dist < MAX_DOOR_HEAR_DIST {
                                let falloff = (1.0 - dist / MAX_DOOR_HEAR_DIST).powi(2);
                                let vol = (0.70 * falloff).clamp(0.04, 0.70);
                                let is_metal = door.name.contains("Metal") || door.name.contains("DDoor") || door.name.contains("Fence");
                                let snd = if is_metal { &sb.door_metal_open } else { &sb.door_wood_open };
                                crate::audio::play_sound(&mut commands, snd, vol);
                            }
                        }
                    }
                    break;
                }
            }
        }

        // Auto close timer
        if door.is_open && door.auto_close_timer > 0.0 {
            door.auto_close_timer -= dt;
            if door.auto_close_timer <= 0.0 {
                door.target = 0.0;
                door.is_open = false;
                if let Some(sb) = &sound_bank {
                    if let Some(ppos) = player_pos {
                        let door_pos = door.interaction_point;
                        let dist = ppos.distance(door_pos);
                        const MAX_DOOR_HEAR_DIST: f32 = 18.0;
                        if dist < MAX_DOOR_HEAR_DIST {
                            let falloff = (1.0 - dist / MAX_DOOR_HEAR_DIST).powi(2);
                            let vol = (0.70 * falloff).clamp(0.04, 0.70);
                            let is_metal = door.name.contains("Metal") || door.name.contains("DDoor") || door.name.contains("Fence");
                            let snd = if is_metal { &sb.door_metal_close } else { &sb.door_wood_close };
                            crate::audio::play_sound(&mut commands, snd, vol);
                        }
                    }
                }
            }
        }

        // Animate door progress towards target
        if (door.progress - door.target).abs() > 1e-4 {
            let dir = (door.target - door.progress).signum();
            door.progress = (door.progress + dir * 2.5 * dt).clamp(0.0, 1.0);

            if door.progress > 0.05 {
                for &src in &door.collision_sources {
                    collision.disable_source(src);
                }
            } else {
                for &src in &door.collision_sources {
                    collision.enable_source(src);
                }
            }

            let t = door.progress * door.progress * (3.0 - 2.0 * door.progress);
            for &leaf_entity in &door.leaves {
                if let Ok((leaf, mut xform)) = leaves.get_mut(leaf_entity) {
                    xform.rotation = leaf.closed_rotation.slerp(leaf.open_rotation, t);
                }
            }
        }
    }
}
