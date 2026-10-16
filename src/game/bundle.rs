//! 47's suit bundle: changing out of his suit leaves it folded on the floor (the original's
//! `CB_Hitman2` / `Bundle_01`), and he can put it back on there.

use super::*;
use crate::{
    character::{self, Animator, CharacterAssets},
    player::Player,
};
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;

/// The wardrobe key of 47's own suit model.
pub const SUIT: &str = "Hitman2_Suit";

/// The folded-suit mesh: its parts and how to set it on the floor.
#[derive(Resource)]
pub struct SuitBundleModel {
    pub parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    pub transform: Transform,
}

#[derive(Component)]
pub(super) struct SuitBundle;

/// Leaves the suit where he changed out of it; E there puts it back on.
#[allow(clippy::too_many_arguments)]
pub(super) fn suit_bundle(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    model: Option<Res<SuitBundleModel>>,
    wardrobe: Res<Wardrobe>,
    textures: Res<Textures>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut mission: ResMut<Mission>,
    mut assets: (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>, ResMut<Assets<Image>>, ResMut<Assets<SkinnedMeshInverseBindposes>>),
    mut was: Local<Option<Outfit>>,
    player: Query<(&GlobalTransform, &Player)>,
    mut animators: Query<&mut Animator>,
    bundles: Query<(Entity, &GlobalTransform), With<SuitBundle>>,
) {
    let Ok((pt, p)) = player.single() else { return };
    let me = pt.translation();
    // Changed out of the suit: it stays here, folded.
    if *was == Some(Outfit::Suit) && mission.outfit != Outfit::Suit {
        if let Some(m) = &model {
            // At his side (in front of him is usually the body he took the clothes from).
            let right = (pt.rotation() * Vec3::X).with_y(0.0).normalize_or_zero();
            let at = me + right * 0.6 + Vec3::Y * 0.02;
            let bundle = commands.spawn((SuitBundle, Transform::from_translation(at).with_rotation(pt.rotation() * m.transform.rotation).with_scale(m.transform.scale), Visibility::Inherited, Name::new("47's suit"))).id();
            for (mesh, material) in &m.parts {
                let part = commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()))).id();
                commands.entity(bundle).add_child(part);
            }
        }
    }
    *was = Some(mission.outfit);
    if mission.outfit == Outfit::Suit || mission.busy > 0.0 || mission.prompt.is_some() || mission.outcome.is_some() {
        return;
    }
    let Some((e, _)) = bundles.iter().find(|(_, t)| t.translation().distance(me) < 1.4) else { return };
    mission.prompt = Some("[E] Put on your suit".to_owned());
    if !keys.just_pressed(KeyCode::KeyE) {
        return;
    }
    let (Some(suit), Some(root)) = (wardrobe.0.get(SUIT), p.model) else { return };
    let mut cast = CharacterAssets { meshes: &mut assets.0, materials: &mut assets.1, images: &mut assets.2, bindposes: &mut assets.3, textures: &textures.0 };
    commands.entity(root).despawn_children().remove::<character::Rig>();
    character::attach(&mut commands, root, suit, &mut cast);
    if let Ok(mut anim) = animators.get_mut(root) {
        anim.play_once("Change_Clothes");
    }
    commands.entity(e).despawn();
    mission.busy = 2.0;
    mission.outfit = Outfit::Suit;
    mission.disguise_unit = None;
    *was = Some(Outfit::Suit);
    if let Some(sb) = &sound_bank {
        crate::audio::play_sound(&mut commands, &sb.clothes_touch, 0.7);
    }
}
