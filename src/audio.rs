//! Sound bank and audio systems for Hitman: Contracts authentic audio.

use bevy::{
    audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume},
    prelude::*,
};

#[derive(Resource)]
pub struct SoundBank {
    pub ambient_music: Handle<AudioSource>,
    pub door_wood_open: Handle<AudioSource>,
    pub door_wood_close: Handle<AudioSource>,
    pub door_metal_open: Handle<AudioSource>,
    pub door_metal_close: Handle<AudioSource>,
    pub shot_silenced: Handle<AudioSource>,
    pub fiber_wire: Handle<AudioSource>,
    pub sub_explode: Handle<AudioSource>,
    pub gun_pull: Handle<AudioSource>,
    pub gun_holster: Handle<AudioSource>,
    pub fuchs_want_soup: Handle<AudioSource>,
    pub fuchs_poisoned: Handle<AudioSource>,
    pub step_snow: [Handle<AudioSource>; 3],
    pub step_steel: [Handle<AudioSource>; 2],
    pub clothes_touch: Handle<AudioSource>,
    pub body_drag: Handle<AudioSource>,
    pub body_drop: Handle<AudioSource>,
    pub osd_hint: Handle<AudioSource>,
    pub osd_warning: Handle<AudioSource>,
    #[allow(dead_code)]
    pub key_pick_lock: Handle<AudioSource>,
    pub bjarkhov_welcome: Handle<AudioSource>,
    pub bjarkhov_drink_vodka: Handle<AudioSource>,
    pub base_alarm: Handle<AudioSource>,
}

pub fn play_sound(commands: &mut Commands, handle: &Handle<AudioSource>, volume: f32) {
    commands.spawn((
        AudioPlayer::new(handle.clone()),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
    ));
}

pub fn setup_audio(mut commands: Commands, mut audio_assets: ResMut<Assets<AudioSource>>) {
    let mut load = |name: &str| -> Handle<AudioSource> {
        let path = format!("data/audio/{name}.ogg");
        match std::fs::read(&path) {
            Ok(bytes) => audio_assets.add(AudioSource {
                bytes: bytes.into(),
            }),
            Err(err) => {
                warn!("Failed to load audio {path}: {err}");
                audio_assets.add(AudioSource {
                    bytes: vec![].into(),
                })
            }
        }
    };

    let bank = SoundBank {
        ambient_music: load("Music_Winter&Night"),
        door_wood_open: load("Door_WoodOpen_01"),
        door_wood_close: load("Door_WoodClosed_01"),
        door_metal_open: load("Door_MetalOpen_01"),
        door_metal_close: load("Door_MetalClosed_01"),
        shot_silenced: load("W_HardballerSD_Shot_01"),
        fiber_wire: load("W_FiberWire_Tighten_01"),
        sub_explode: load("C02-1_Sub_Explode"),
        gun_pull: load("Gun_Pull_01"),
        gun_holster: load("Gun_Holster_01"),
        fuchs_want_soup: load("02-1_FOS_01Fuchs"),
        fuchs_poisoned: load("02-1_FS_01Fuchs"),
        step_snow: [load("Step_Snow_01"), load("Step_Snow_02"), load("Step_Snow_03")],
        step_steel: [load("Step_Steel_01"), load("Step_Steel_02")],
        clothes_touch: load("Clothes_Touch"),
        body_drag: load("Body_Drag"),
        body_drop: load("Body_Drop"),
        osd_hint: load("OSD_Hint"),
        osd_warning: load("OSD_Warning"),
        key_pick_lock: load("Key_PickLock"),
        bjarkhov_welcome: load("Bjarkhov_Welcome"),
        bjarkhov_drink_vodka: load("Bjarkhov_DrinkVodka"),
        base_alarm: load("Base_Alarm"),
    };

    // Spawn ambient Siberia soundtrack looping
    commands.spawn((
        AudioPlayer::new(bank.ambient_music.clone()),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(0.30)),
    ));

    commands.insert_resource(bank);
    info!("Glacier audio bank initialized with authentic Contracts sound effects and ambient soundtrack");
}
