//! Sound bank and audio systems for Hitman: Contracts authentic audio.

use bevy::{
    audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume},
    prelude::*,
};

#[derive(Resource)]
pub struct SoundBank {
    /// Jesper Kyd's score, one loop per `music::Cue`.
    pub music: [Handle<AudioSource>; crate::music::CUES],
    pub wind: Handle<AudioSource>,
    pub fog_horn: Handle<AudioSource>,
    pub announcements: [Handle<AudioSource>; 5],
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
    pub rifle_shot: Handle<AudioSource>,
    pub rifle_clip_out: Handle<AudioSource>,
    pub rifle_clip_in: Handle<AudioSource>,
    pub rifle_chamber: Handle<AudioSource>,
    pub pistol_clip_out: Handle<AudioSource>,
    pub pistol_clip_in: Handle<AudioSource>,
    pub pistol_chamber: Handle<AudioSource>,
    pub empty_click: Handle<AudioSource>,
    pub pickup_gun: Handle<AudioSource>,
    /// Bullet impacts by surface, and in flesh.
    pub impacts: std::collections::HashMap<crate::collision::Surface, Vec<Handle<AudioSource>>>,
    pub impact_flesh: Vec<Handle<AudioSource>>,
    /// Soldiers' pain and death cries.
    pub pain_cries: Vec<Handle<AudioSource>>,
    pub death_cries: Vec<Handle<AudioSource>>,
    /// Six variations of footstep per surface.
    pub steps: std::collections::HashMap<crate::collision::Surface, Vec<Handle<AudioSource>>>,
    /// Diana's briefing lines, by `.LOC` key (`flow::DIANA`).
    pub voices: std::collections::HashMap<&'static str, Handle<AudioSource>>,
    /// What the level's sound emitters play, by bank name (`ambience::VOICES`).
    pub ambient: std::collections::HashMap<&'static str, Handle<AudioSource>>,
}

/// The blizzard's looping wind, swelling with gusts (`atmosphere::Gust`).
#[derive(Component)]
pub struct WindLoop;
pub const WIND_VOLUME: f32 = 0.45;

pub fn play_sound(commands: &mut Commands, handle: &Handle<AudioSource>, volume: f32) {
    commands.spawn((
        AudioPlayer::new(handle.clone()),
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
    ));
}

/// Decodes every sound straight from the level's own bank (`<ID>_MAIN.WHD/WAV`, music from
/// `streams.wav`), by the names the game uses. Missing entries are reported and play silence.
pub fn setup_audio(mut commands: Commands, mut audio_assets: ResMut<Assets<AudioSource>>) {
    let id = std::env::args().nth(1).unwrap_or_else(|| "C02-1".to_owned());
    let dir = std::path::Path::new("data/Scenes").join(&id);
    let archive = match hitman_clone::glacier::audio::AudioArchive::load(
        &dir.join(format!("{id}_MAIN.WHD")),
        &dir.join(format!("{id}_MAIN.WAV")),
        std::path::Path::new("data/streams.wav"),
    ) {
        Ok(a) => Some(a),
        Err(err) => {
            warn!("no sound bank for {id}: {err:#}");
            None
        }
    };
    let mut load = |name: &str| -> Handle<AudioSource> {
        let bytes = archive.as_ref().and_then(|a| a.decode(&format!("{name}.wav"))).unwrap_or_else(|| {
            warn!("sound {name} not found in the {id} bank");
            Vec::new()
        });
        audio_assets.add(AudioSource { bytes: bytes.into() })
    };
    let bank = SoundBank {
        music: crate::music::TRACKS.map(&mut load),
        wind: load("Wind_Snow_01"),
        fog_horn: load("Ship_FogHorn_01"),
        // The base PA: news, weather ("Snow squalls continuing…"), notices.
        announcements: ["02-1_LS_01Announcement", "02-1_LS_02Announcement", "02-1_LS_03Announcement", "02-1_LS_06Announcement", "02-1_LS_08Advertisement"]
            .map(&mut load),
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
        clothes_touch: load("Clothes_NormalTouch_01"),
        body_drag: load("Body_Drag_01"),
        body_drop: load("Drop_BodySnow_01"),
        osd_hint: load("OSD_Ingame_Hint_01"),
        osd_warning: load("OSD_Ingame_Warning_01"),
        key_pick_lock: load("Key_PickLock_01"),
        // "So good to meet you at last Fabian… We shall have some vodka."
        bjarkhov_welcome: load("C02-1_BGHM_01CB"),
        // "Nostrovya!"
        bjarkhov_drink_vodka: load("C02-1_B_01CB"),
        // "Alarm. Get to your posts. We have an intruder. Alarm."
        base_alarm: load("02-1_LS_09Alarm"),
        rifle_shot: load("W_AK47_Shot_01"),
        rifle_clip_out: load("W_AK47_ClipOut"),
        rifle_clip_in: load("W_AK47_ClipIn"),
        rifle_chamber: load("W_AK47_Chamber"),
        pistol_clip_out: load("W_HardBaller_ClipOut_01"),
        pistol_clip_in: load("W_HardBaller_ClipIn_01"),
        pistol_chamber: load("W_HardBaller_Chamber_01"),
        empty_click: load("W_Empty_01"),
        pickup_gun: load("PickUp_Gun_01"),
        impacts: {
            use crate::collision::Surface::*;
            [(Snow, "Snow", 6), (Gravel, "Gravel", 6), (Concrete, "Stone", 6), (Steel, "Metal", 7), (Wood, "Wood", 4), (Carpet, "Cloth", 3), (Water, "Water", 4), (Dirt, "Gravel", 6)]
                .into_iter()
                .map(|(s, name, n)| (s, (1..=n).map(|i| load(&format!("Impact_{name}_{i:02}"))).collect()))
                .collect()
        },
        impact_flesh: (1..=5).map(|i| load(&format!("Impact_Flesh_{i:02}"))).collect(),
        pain_cries: (1..=5).map(|i| load(&format!("Pain_E_M1_{i:02}"))).collect(),
        death_cries: (1..=5).map(|i| load(&format!("Die_E_M1_{i:02}"))).collect(),
        steps: {
            use crate::collision::Surface::*;
            [(Snow, "SnowNew"), (Gravel, "Gravel"), (Concrete, "Concrete"), (Steel, "Steel"), (Wood, "WoodenFloor"), (Carpet, "Carpet"), (Water, "Water"), (Dirt, "Dirt")]
                .into_iter()
                .map(|(s, name)| (s, (1..=6).map(|i| load(&format!("Step_{name}_{i:02}"))).collect()))
                .collect()
        },
        voices: crate::flow::DIANA.iter().map(|&k| (k, load(k))).collect(),
        ambient: crate::ambience::VOICES.iter().map(|v| (v.sound, load(v.sound))).collect(),
    };

    // The blizzard never stops.
    commands.spawn((AudioPlayer::new(bank.wind.clone()), PlaybackSettings::LOOP.with_volume(Volume::Linear(WIND_VOLUME)), WindLoop));
    commands.insert_resource(bank);
    commands.insert_resource(Ambience { next_pa: 40.0, next_horn: 25.0, pa_index: 0 });
}

/// Timers for the base's background life: PA announcements and the icebreaker's fog horn.
#[derive(Resource)]
pub struct Ambience {
    next_pa: f32,
    next_horn: f32,
    pa_index: usize,
}

pub fn ambience(mut commands: Commands, time: Res<Time>, bank: Option<Res<SoundBank>>, mut amb: Option<ResMut<Ambience>>) {
    let (Some(bank), Some(amb)) = (bank, amb.as_mut()) else { return };
    let dt = time.delta_secs();
    amb.next_pa -= dt;
    amb.next_horn -= dt;
    if amb.next_pa <= 0.0 {
        play_sound(&mut commands, &bank.announcements[amb.pa_index % bank.announcements.len()], 0.35);
        amb.pa_index += 1;
        amb.next_pa = 75.0;
    }
    if amb.next_horn <= 0.0 {
        play_sound(&mut commands, &bank.fog_horn, 0.25);
        amb.next_horn = 55.0;
    }
}
