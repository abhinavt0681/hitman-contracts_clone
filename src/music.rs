//! Adaptive score: Jesper Kyd's own loops from the level bank, crossfaded by what is happening.
//!
//! Calm night while unseen, the submarine theme near the hull, suspense while a guard grows
//! suspicious or is out looking, the intense cue while 47 is being hunted, and the death or title loops when it ends.
//! Calm and submarine pause while silent so they resume where they left off; the others start
//! from the top each time they come in.

use crate::{
    audio::SoundBank,
    game::{Actor, Anchors, Kind, Mind, Mission, Outcome},
    player::Player,
};
use bevy::{
    audio::{AudioSink, AudioSinkPlayback, PlaybackSettings, Volume},
    prelude::*,
};

pub const CUES: usize = 6;
/// Bank names, indexed by `Cue`.
pub const TRACKS: [&str; CUES] =
    ["Music_Winter&Night", "Music_Submarine", "Music_Suspense3", "Music_Sus1_High_Intense", "Music_Death_Loop", "Music_Main_Title_No_Intro"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cue {
    Calm,
    Submarine,
    Suspense,
    Alarm,
    Death,
    Complete,
}

impl Cue {
    fn level(self) -> f32 {
        match self {
            Cue::Alarm | Cue::Death => 0.40,
            _ => 0.30,
        }
    }
    /// Whether the loop keeps its place while silent.
    fn resumes(self) -> bool {
        matches!(self, Cue::Calm | Cue::Submarine)
    }
}

/// Suspicion above this brings in the suspense cue…
const SUSPENSE_ON: f32 = 0.35;
/// …and it stays until suspicion has been below this for `SUSPENSE_HOLD` seconds.
const SUSPENSE_OFF: f32 = 0.15;
const SUSPENSE_HOLD: f32 = 8.0;
/// Seconds for a full fade in or out.
const FADE: f32 = 2.5;
/// Extra distance beyond the hull radius that still counts as "at the submarine".
const SUB_MARGIN: f32 = 10.0;

pub struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Score>().add_systems(Update, (choose_cue, crossfade).chain());
    }
}

#[derive(Resource)]
struct Score {
    cue: Cue,
    calm_for: f32,
}

impl Default for Score {
    fn default() -> Self {
        Score { cue: Cue::Calm, calm_for: 0.0 }
    }
}

#[derive(Component)]
struct Voice {
    cue: Cue,
    volume: f32,
}

fn choose_cue(
    time: Res<Time>,
    mut score: ResMut<Score>,
    mission: Option<Res<Mission>>,
    anchors: Option<Res<Anchors>>,
    actors: Query<&Actor>,
    player: Query<&GlobalTransform, With<Player>>,
) {
    let Some(mission) = mission else { return };
    let guards = || actors.iter().filter(|a| a.alive && a.kind == Kind::Guard);
    let suspicion = guards().map(|a| a.suspicion).fold(0.0, f32::max);
    // Hunted: someone is after 47 right now (the alarm itself stays raised for good).
    let hunted = guards().any(|a| a.alerted);
    let searching = guards().any(|a| a.mind != Mind::Calm);
    if suspicion < SUSPENSE_OFF && !searching {
        score.calm_for += time.delta_secs();
    } else {
        score.calm_for = 0.0;
    }
    let near_sub = match (anchors, player.single()) {
        (Some(a), Ok(p)) => p.translation().xz().distance(a.sub.0.xz()) < a.sub.1 + SUB_MARGIN,
        _ => false,
    };
    let cue = match mission.outcome {
        Some(Outcome::Failed) => Cue::Death,
        Some(Outcome::Complete) => Cue::Complete,
        None if hunted => Cue::Alarm,
        None if suspicion > SUSPENSE_ON || searching || (score.cue == Cue::Suspense && score.calm_for < SUSPENSE_HOLD) => Cue::Suspense,
        None if near_sub => Cue::Submarine,
        None => Cue::Calm,
    };
    if cue != score.cue {
        info!("music: {:?} -> {cue:?}", score.cue);
        score.cue = cue;
    }
}

fn crossfade(
    mut commands: Commands,
    time: Res<Time>,
    score: Res<Score>,
    bank: Option<Res<SoundBank>>,
    mut voices: Query<(Entity, &mut Voice, Option<&mut AudioSink>)>,
) {
    let Some(bank) = bank else { return };
    // Death cuts in hard; everything else breathes.
    let rate = if score.cue == Cue::Death { 3.0 } else { 1.0 } / FADE * time.delta_secs();
    let mut current_playing = false;
    for (entity, mut voice, sink) in &mut voices {
        let active = voice.cue == score.cue;
        current_playing |= active;
        let target = if active { voice.cue.level() } else { 0.0 };
        let step = rate * voice.cue.level().max(0.3);
        voice.volume = if voice.volume < target { (voice.volume + step).min(target) } else { (voice.volume - step).max(target) };
        let Some(mut sink) = sink else { continue };
        sink.set_volume(Volume::Linear(voice.volume));
        if voice.volume > 0.0 {
            if sink.is_paused() {
                sink.play();
            }
        } else if voice.cue.resumes() {
            if !sink.is_paused() {
                sink.pause();
            }
        } else {
            debug!("music: {:?} faded out", voice.cue);
            commands.entity(entity).despawn();
        }
    }
    if !current_playing {
        debug!("music: start {:?}", score.cue);
        commands.spawn((
            AudioPlayer::new(bank.music[score.cue as usize].clone()),
            PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
            Voice { cue: score.cue, volume: 0.0 },
            Name::new(TRACKS[score.cue as usize]),
        ));
    }
}
