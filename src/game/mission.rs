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
    mut scene: ResMut<crate::cutscene::Cutscene>,
    player: Query<(Entity, &GlobalTransform), With<Player>>,
    mut actors: Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>,
) {
    use crate::cutscene::Line;
    if mission.outcome.is_some() {
        return;
    }
    let Ok((hitman, pt)) = player.single() else { return };
    let me = pt.translation();
    // The nearest living guard within `r` of 47, to speak for the base.
    let guard_near = |actors: &Query<(Entity, &GlobalTransform, &mut Actor, &mut Animator)>, r: f32| {
        actors
            .iter()
            .filter(|(_, _, a, _)| a.alive && a.kind == Kind::Guard)
            .map(|(e, t, _, _)| (e, t.translation().distance(me)))
            .filter(|(_, d)| *d < r)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(e, _)| e)
    };
    let line = |key, speaker, listener, clip| Line { key, speaker, listener, clip };

    // At the gate, as Fuchs: a guard waves him through (the original's scene, its voices).
    if mission.outfit == Outfit::Fuchs && mission.bjarkhov_meeting_state == 0 && me.distance(anchors.ship_gangway) < 7.0 {
        mission.bjarkhov_meeting_state = 1;
        if let Some(g) = guard_near(&actors, 12.0) {
            scene.play(
                "gate",
                vec![
                    line("C02-1_GIHMI_01G", g, hitman, None),
                    line("C02-1_GIHMI_02H", hitman, g, None),
                    line("C02-1_GIHMI_03G", g, hitman, None),
                ],
            );
        }
    }

    // On deck: the frisk. A gun on him is found ("What the .. Alarm !"); the wire isn't.
    if mission.outfit == Outfit::Fuchs && mission.frisk_state == 0 && anchors.frisk.iter().any(|f| f.distance(me) < 3.0) {
        if let Some(g) = guard_near(&actors, 15.0) {
            mission.frisk_state = 1;
            mission.frisker = Some(g);
            scene.play(
                "frisk",
                vec![
                    line("C02-1_GPHMD_01Guard", g, hitman, None),
                    line("C02-1_GPHMD_01HM", hitman, g, None),
                    line("All_R_FS_Frisk01M1", g, hitman, Some("Frisk_Search")),
                ],
            );
        }
    }
    if scene.finished("frisk") {
        let g = mission.frisker.unwrap_or(hitman);
        let armed = mission.carrying_pistol || mission.rifle.is_some();
        if armed {
            scene.play("frisk_bad", vec![line("All_R_FS_FriskBADM1", g, hitman, Some("Frisk_Search_Gun"))]);
        } else {
            mission.frisk_state = 2;
            scene.play("frisk_ok", vec![line("All_R_FS_FriskOKM1", g, hitman, Some("Frisk_Search_Ok"))]);
        }
    }
    if scene.finished("frisk_bad") {
        mission.frisk_state = 2;
        if let Some(g) = mission.frisker {
            if let Ok((_, _, mut a, _)) = actors.get_mut(g) {
                a.alerted = true;
                a.last_seen = Some(me);
                a.lost_for = 0.0;
            }
        }
        raise_alarm(Some(&mut commands), sound_bank.as_deref(), &mut mission, &text, Some(text.0.get("WeaponFound").unwrap_or("A weapon was found on you")));
    }

    // Bjarkhov's office, as Fuchs: the greeting scene, then he turns to pour the vodka.
    if me.distance(anchors.bjarkhov_office) < 5.0 && mission.bjarkhov_meeting_state < 2 && mission.outfit == Outfit::Fuchs {
        mission.bjarkhov_meeting_state = 2;
        if let Some(b) = actors.iter().find(|(_, _, a, _)| a.name.contains("Bjarkhov") && a.alive).map(|(e, _, _, _)| e) {
            scene.play(
                "bjarkhov",
                vec![
                    line("C02-1_BGHM_01CB", b, hitman, None),
                    line("C02-1_BGHM_02HM", hitman, b, None),
                    line("C02-1_BGHM_03CB", b, hitman, None),
                ],
            );
        }
    }
    if scene.finished("bjarkhov") {
        for (_, _, a, mut anim) in &mut actors {
            if a.name.contains("Bjarkhov") && a.alive {
                anim.play("Bjarkov_Turn_toPour", 1.0);
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
        mission.warn("Radiation! You need a radiation suit to be on the submarine");
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
        mission.speak(text.0.get("02-1_FOS_01Fuchs").unwrap_or("Can I have some soup down here?!! Russisches Schwein."));
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
