//! The level's own soundscape: the sound emitters placed in the GMS (`SOUND_CLASS`), each
//! playing the bank sound its `.SND` record names, positioned in 3D around the camera.
//!
//! The placement data has no script for when one-shots fire (wolves, ice, rats are driven by
//! `.sdl` code we don't run), so those play at random intervals; hums, ventilation, boiling
//! water, the flag and the wind beds loop while within earshot. Sounds the scripts trigger
//! themselves (impacts, scope clicks, the fog horn) are left to the game code.

use crate::{atmosphere::Gust, audio::SoundBank};
use bevy::{
    audio::{AudioSinkPlayback, PlaybackSettings, SpatialAudioSink, SpatialScale, Volume},
    prelude::*,
};
use hitman_clone::glacier::{gms::SOUND_CLASS, level::Level};

/// How each emitted sound behaves.
pub struct Voice {
    pub sound: &'static str,
    /// Distance (m) within which it plays at full volume; it falls off with the square beyond.
    reference: f32,
    /// Beyond this it isn't played at all.
    range: f32,
    volume: f32,
    /// One-shot every so many seconds (min, max) instead of looping.
    every: Option<(f32, f32)>,
    /// Swells with the squalls.
    gusty: bool,
}

const fn looped(sound: &'static str, reference: f32, range: f32, volume: f32, gusty: bool) -> Voice {
    Voice { sound, reference, range, volume, every: None, gusty }
}

const fn now_and_then(sound: &'static str, reference: f32, range: f32, volume: f32, every: (f32, f32)) -> Voice {
    Voice { sound, reference, range, volume, every: Some(every), gusty: false }
}

pub const VOICES: [Voice; 13] = [
    looped("Light_FluoresentHumm_01", 1.0, 7.0, 0.35, false),
    looped("Flag_Wind_01", 4.0, 35.0, 0.8, true),
    looped("Ventilation_Air_01", 3.0, 22.0, 0.5, false),
    looped("Ventilation_Air_02", 3.0, 22.0, 0.5, false),
    looped("Kitchen_Boiling_01", 1.5, 10.0, 0.45, false),
    looped("CallSignal6", 1.5, 10.0, 0.25, false),
    looped("Wind_Desert_01", 8.0, 60.0, 0.4, true),
    looped("Wind_Soft_01", 8.0, 60.0, 0.35, true),
    looped("Wind_Soft_03", 8.0, 60.0, 0.35, true),
    looped("Wind_Snow_01", 8.0, 60.0, 0.3, true),
    now_and_then("Wolf_Yelling_01", 40.0, 400.0, 0.6, (25.0, 70.0)),
    now_and_then("Ice_Squeak_03", 4.0, 30.0, 0.5, (6.0, 20.0)),
    now_and_then("Rat_Squeak_02", 1.5, 12.0, 0.4, (10.0, 30.0)),
];

pub struct AmbiencePlugin;

impl Plugin for AmbiencePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (play_emitters, swell_with_gusts));
    }
}

#[derive(Component)]
pub struct Emitter {
    voice: usize,
    /// Seconds to the next one-shot.
    wait: f32,
    /// The entity playing the loop, while within earshot.
    playing: Option<Entity>,
}

/// The playing loop of a gusty emitter, and its base volume.
#[derive(Component)]
struct Gusty(f32);

/// Places an emitter at every sound entity whose sound we know how to play.
pub fn spawn(level: &Level, positions: &[Vec3], commands: &mut Commands) {
    let mut count = 0;
    for (i, e) in level.entities.iter().enumerate() {
        if e.type_id != SOUND_CLASS {
            continue;
        }
        let Some(name) = e.sound.and_then(|r| level.sound_name(r)) else { continue };
        // Some records are sound sets; these are their members in the bank.
        let name = match name {
            "Wolf_TheCoolOnes" => "Wolf_Yelling_01",
            "Rats_TheCoolOnes" => "Rat_Squeak_02",
            "Machine_CallSignal6" => "CallSignal6",
            other => other,
        };
        let Some(voice) = VOICES.iter().position(|v| v.sound == name) else { continue };
        let wait = VOICES[voice].every.map_or(0.0, |(lo, hi)| lo + (hi - lo) * (i as f32 * 0.618).fract());
        commands.spawn((Transform::from_translation(positions[i]), Emitter { voice, wait, playing: None }, Name::new(e.name.clone())));
        count += 1;
    }
    info!("ambience: {count} sound emitters");
}

fn play_emitters(
    mut commands: Commands,
    time: Res<Time>,
    bank: Option<Res<SoundBank>>,
    camera: Query<&GlobalTransform, With<crate::player::OrbitCamera>>,
    mut emitters: Query<(Entity, &GlobalTransform, &mut Emitter)>,
) {
    let (Some(bank), Ok(cam)) = (bank, camera.single()) else { return };
    let ear = cam.translation();
    let dt = time.delta_secs();
    for (entity, at, mut emitter) in &mut emitters {
        let voice = &VOICES[emitter.voice];
        let Some(handle) = bank.ambient.get(voice.sound) else { continue };
        let dist = at.translation().distance(ear);
        let settings = |s: PlaybackSettings| {
            s.with_spatial(true).with_spatial_scale(SpatialScale::new(1.0 / voice.reference)).with_volume(Volume::Linear(voice.volume))
        };
        match voice.every {
            Some((lo, hi)) => {
                emitter.wait -= dt;
                if emitter.wait <= 0.0 {
                    emitter.wait = lo + (hi - lo) * rand01(time.elapsed_secs() + entity.to_bits() as f32 * 0.001);
                    if dist < voice.range {
                        let one_shot = commands.spawn((AudioPlayer::new(handle.clone()), settings(PlaybackSettings::DESPAWN), Transform::default())).id();
                        commands.entity(entity).add_child(one_shot);
                    }
                }
            }
            None => {
                if dist < voice.range && emitter.playing.is_none() {
                    let mut player = commands.spawn((AudioPlayer::new(handle.clone()), settings(PlaybackSettings::LOOP), Transform::default()));
                    if voice.gusty {
                        player.insert(Gusty(voice.volume));
                    }
                    let player = player.id();
                    commands.entity(entity).add_child(player);
                    emitter.playing = Some(player);
                } else if dist > voice.range * 1.2 {
                    if let Some(player) = emitter.playing.take() {
                        commands.entity(player).despawn();
                    }
                }
            }
        }
    }
}

fn swell_with_gusts(gust: Res<Gust>, mut sinks: Query<(&Gusty, &mut SpatialAudioSink)>) {
    for (g, mut sink) in &mut sinks {
        sink.set_volume(Volume::Linear(g.0 * (0.6 + 0.9 * gust.strength)));
    }
}

/// Cheap hash of `x` to 0..1.
fn rand01(x: f32) -> f32 {
    ((x * 12.9898).sin() * 43_758.547).fract().abs()
}
