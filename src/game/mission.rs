//! Mission scripting: the Bjarkhov meeting, Fuchs's soup, radiation and death.

use crate::{
    character::{AnimLibrary, Animator, Gait},
    player::Player,
};

use super::*;

pub(super) const RADIATION_DPS: f32 = 6.0;

pub(super) fn bjarkhov_meeting(
    mut commands: Commands,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    anchors: Res<Anchors>,
    text: Res<Text>,
    mut mission: ResMut<Mission>,
    player: Query<&GlobalTransform, With<Player>>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
) {
    if mission.outcome.is_some() {
        return;
    }
    let Ok(pt) = player.single() else { return };
    let me = pt.translation();

    // Gangway greeting if disguised as Fuchs
    if mission.outfit == Outfit::Fuchs && mission.bjarkhov_meeting_state == 0 {
        if me.distance(anchors.ship_gangway) < 7.0 {
            mission.bjarkhov_meeting_state = 1;
            mission.say("Icebreaker Guard: 'Welcome aboard Mr. Fuchs. Commander Bjarkhov is waiting for you in his office.'");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.osd_hint, 0.6);
            }
        }
    }

    // Entering Bjarkhov's office
    if me.distance(anchors.bjarkhov_office) < 5.0 && mission.bjarkhov_meeting_state < 2 {
        if mission.outfit == Outfit::Fuchs {
            mission.bjarkhov_meeting_state = 2;
            mission.say("Commander Bjarkhov: 'Ah, Fabian! Good to see you... Come in, let us discuss business over some vodka.'");
            if let Some(sb) = &sound_bank {
                crate::audio::play_sound(&mut commands, &sb.bjarkhov_welcome, 0.9);
            }
            for (_, _, a, mut anim) in &mut actors {
                if a.name.contains("Bjarkhov") && a.alive {
                    anim.play("Bjarkov_Turn_toPour", 1.0);
                }
            }
        }
    }

    // Bjarkhov drinks poisoned vodka
    if mission.bjarkhov_vodka_poisoned && mission.bjarkhov_meeting_state == 2 {
        mission.bjarkhov_meeting_state = 3;
        for (e, _, mut a, mut anim) in &mut actors {
            if a.name.contains("Bjarkhov") && a.alive {
                a.alive = false;
                anim.play_once("Die_Throat_Cut_Alt_01");
                commands.entity(e).remove::<Gait>();
                let name = a.name.clone();
                let kind = a.kind;
                on_kill(&mut mission, &text, kind, &name);
                mission.say("Commander Bjarkhov succumbed to cyanide poisoning");
                if let Some(sb) = &sound_bank {
                    crate::audio::play_sound(&mut commands, &sb.bjarkhov_drink_vodka, 0.85);
                }
                break;
            }
        }
    }
}

pub(super) fn hazards(time: Res<Time>, anchors: Res<Anchors>, mut mission: ResMut<Mission>, player: Query<&GlobalTransform, With<Player>>) {
    if mission.outcome.is_some() {
        return;
    }
    let Ok(pt) = player.single() else { return };
    let (centre, radius) = anchors.sub;
    if mission.outfit != Outfit::BombWorker && {
        // Inside the hull footprint, from the bilge up to deck level.
        let p = pt.translation();
        p.xz().distance(centre.xz()) < radius && p.y < centre.y + 3.0
    } {
        mission.health -= RADIATION_DPS * time.delta_secs();
        mission.say("Radiation! You need a radiation suit to be on the submarine");
    }
    if mission.health <= 0.0 {
        mission.health = 0.0;
        mission.outcome = Some(Outcome::Failed);
    }
}

pub(super) fn soup_poison_timer(
    mut commands: Commands,
    time: Res<Time>,
    lib: Res<AnimLibrary>,
    text: Res<Text>,
    sound_bank: Option<Res<crate::audio::SoundBank>>,
    mut mission: ResMut<Mission>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
) {
    let Some(t) = mission.soup_timer.as_mut() else { return };
    let prev = *t;
    *t -= time.delta_secs();
    if prev > 8.0 && *t <= 8.0 {
        if let Some(sb) = &sound_bank {
            crate::audio::play_sound(&mut commands, &sb.fuchs_want_soup, 0.9);
        }
        mission.say(text.0.get("02-1_FOS_01Fuchs").unwrap_or("Can I have some soup down here?!! Russisches Schwein."));
    } else if prev > 4.0 && *t <= 4.0 {
        mission.say("Fabian Fuchs eats the poisoned soup...");
    } else if *t <= 0.0 {
        mission.soup_timer = None;
        if let Some(sb) = &sound_bank {
            crate::audio::play_sound(&mut commands, &sb.fuchs_poisoned, 0.9);
        }
        mission.say("Fabian Fuchs succumbed to cyanide poison");
        for (e, _, mut a, mut anim) in &mut actors {
            if a.name.contains("FabianFuchs") && a.alive {
                a.alive = false;
                let clip = lib.first_of(anim.set(), &["Die_Forward", "Die_Backward"]).unwrap_or_default();
                anim.play_once(&clip);
                commands.entity(e).remove::<Gait>();
                let (kind, name) = (a.kind, a.name.clone());
                on_kill(&mut mission, &text, kind, &name);
            }
        }
    }
}
