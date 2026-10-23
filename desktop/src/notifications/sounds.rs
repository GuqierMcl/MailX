//! MailX's own notification sounds, synthesized at play time: a few short sine
//! tones with soft envelopes. No audio files ship with the app, so there is no
//! licensing to track and nothing to decode.
//!
//! The system notification of a mail with one of these sounds is shown silent,
//! so the two never play together.
use std::{f32::consts::TAU, num::NonZero, time::Duration};

use mailx_core::NotificationSound;

const RATE: u32 = 44_100;
/// Well below full scale: these play over whatever the user is listening to.
const VOLUME: f32 = 0.22;

/// One partial of a note: frequency, start, length and relative strength.
/// `glide` is the frequency it ends on, as a multiple of `hz`.
struct Tone {
    hz: f32,
    start: f32,
    length: f32,
    gain: f32,
    glide: f32,
}

const fn tone(hz: f32, start: f32, length: f32, gain: f32) -> Tone {
    slide(hz, hz, start, length, gain)
}

/// A tone whose pitch moves evenly from `from` to `to`.
const fn slide(from: f32, to: f32, start: f32, length: f32, gain: f32) -> Tone {
    Tone {
        hz: from,
        start,
        length,
        gain,
        glide: to / from,
    }
}

// E6 then A6, the second a little longer: an "arrived" lift.
const CHIME: &[Tone] = &[
    tone(1318.5, 0.0, 0.16, 1.0),
    tone(2637.0, 0.0, 0.10, 0.18),
    tone(1760.0, 0.11, 0.32, 1.0),
    tone(3520.0, 0.11, 0.18, 0.15),
];
// One bell-like C7 with an inharmonic overtone, decaying.
const DING: &[Tone] = &[
    tone(2093.0, 0.0, 0.55, 1.0),
    tone(2093.0 * 2.76, 0.0, 0.18, 0.22),
];
// A quick upward glide, built from three overlapping short notes.
const DROPLET: &[Tone] = &[
    tone(880.0, 0.0, 0.06, 0.7),
    tone(1174.7, 0.04, 0.07, 0.85),
    tone(1568.0, 0.08, 0.16, 1.0),
];

// Two rising pops, like bubbles surfacing.
const BUBBLE: &[Tone] = &[
    slide(620.0, 1400.0, 0.0, 0.08, 0.9),
    slide(820.0, 1850.0, 0.10, 0.09, 1.0),
];
// G5 then C6 with the bright fourth partial of a struck wooden bar.
const MARIMBA: &[Tone] = &[
    tone(784.0, 0.0, 0.30, 1.0),
    tone(784.0 * 4.0, 0.0, 0.04, 0.25),
    tone(1046.5, 0.12, 0.40, 1.0),
    tone(1046.5 * 4.0, 0.12, 0.05, 0.25),
];
// A fast C major arpeggio up to C7, each note a little brighter.
const SPARKLE: &[Tone] = &[
    tone(1046.5, 0.0, 0.18, 0.6),
    tone(1318.5, 0.05, 0.18, 0.7),
    tone(1568.0, 0.10, 0.20, 0.85),
    tone(2093.0, 0.15, 0.30, 1.0),
];
// Two soft low taps, a polite knock.
const KNOCK: &[Tone] = &[
    tone(520.0, 0.0, 0.07, 1.0),
    tone(1300.0, 0.0, 0.02, 0.3),
    tone(520.0, 0.13, 0.08, 0.9),
    tone(1300.0, 0.13, 0.02, 0.25),
];
// C6, E6 and G6 together, ringing out.
const CHORD: &[Tone] = &[
    tone(1046.5, 0.0, 0.60, 1.0),
    tone(1318.5, 0.0, 0.55, 0.8),
    tone(1568.0, 0.0, 0.50, 0.7),
];

fn tones(sound: NotificationSound) -> &'static [Tone] {
    match sound {
        NotificationSound::Chime => CHIME,
        NotificationSound::Ding => DING,
        NotificationSound::Droplet => DROPLET,
        NotificationSound::Bubble => BUBBLE,
        NotificationSound::Marimba => MARIMBA,
        NotificationSound::Sparkle => SPARKLE,
        NotificationSound::Knock => KNOCK,
        NotificationSound::Chord => CHORD,
        NotificationSound::System | NotificationSound::None => &[],
    }
}

/// Mono samples of a built-in sound; empty for the system sound and none.
pub(crate) fn samples(sound: NotificationSound) -> Vec<f32> {
    let tones = tones(sound);
    let end = tones
        .iter()
        .map(|tone| tone.start + tone.length)
        .fold(0.0, f32::max);
    let count = (end * RATE as f32).ceil() as usize;
    let mut out = vec![0.0f32; count];
    for tone in tones {
        let first = (tone.start * RATE as f32) as usize;
        let length = (tone.length * RATE as f32) as usize;
        // 5 ms attack against clicks, then an exponential decay to silence.
        let attack = (0.005 * RATE as f32) as usize;
        for index in 0..length {
            let Some(slot) = out.get_mut(first + index) else {
                break;
            };
            let t = index as f32 / RATE as f32;
            let progress = index as f32 / length as f32;
            // Phase of a linear sweep: the integral of the moving frequency.
            let phase = tone.hz * (t + (tone.glide - 1.0) * t * progress / 2.0);
            let rise = (index as f32 / attack as f32).min(1.0);
            let decay = (-5.0 * progress).exp() * (1.0 - progress);
            *slot += (TAU * phase).sin() * tone.gain * rise * decay;
        }
    }
    let peak = out
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    if peak > 0.0 {
        out.iter_mut().for_each(|sample| *sample *= VOLUME / peak);
    }
    out
}

/// Plays a built-in sound on the default output device, on a thread of its
/// own. Without an output device (or with the system / no sound) nothing
/// happens: a missing sound is never worth an error.
pub(crate) fn play(sound: NotificationSound) {
    let data = samples(sound);
    if data.is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("notification-sound".into())
        .spawn(move || {
            let Ok(mut sink) = rodio::DeviceSinkBuilder::open_default_sink() else {
                return;
            };
            sink.log_on_drop(false);
            let length = Duration::from_secs_f32(data.len() as f32 / RATE as f32);
            sink.mixer().add(rodio::buffer::SamplesBuffer::new(
                NonZero::<u16>::MIN,
                NonZero::new(RATE).expect("non-zero rate"),
                data,
            ));
            // The device closes when the sink drops; keep it until the sound ends.
            std::thread::sleep(length + Duration::from_millis(150));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: [NotificationSound; 8] = [
        NotificationSound::Chime,
        NotificationSound::Ding,
        NotificationSound::Droplet,
        NotificationSound::Bubble,
        NotificationSound::Marimba,
        NotificationSound::Sparkle,
        NotificationSound::Knock,
        NotificationSound::Chord,
    ];

    #[test]
    fn every_built_in_sound_is_its_own() {
        let all: Vec<Vec<f32>> = OWN.iter().map(|sound| samples(*sound)).collect();
        for (index, one) in all.iter().enumerate() {
            assert!(
                all[index + 1..].iter().all(|other| other != one),
                "{:?}",
                OWN[index]
            );
        }
    }

    #[test]
    fn a_slide_ends_on_its_target_pitch() {
        // Count zero crossings in the last tenth of a 1 s slide from 400 to 800 Hz.
        let tone = slide(400.0, 800.0, 0.0, 1.0, 1.0);
        let (from, to) = ((0.9 * RATE as f32) as usize, RATE as usize);
        let crossings = (from..to)
            .filter(|&index| {
                let at = |index: usize| {
                    let t = index as f32 / RATE as f32;
                    (TAU * tone.hz * (t + (tone.glide - 1.0) * t * t / 2.0)).sin()
                };
                at(index).signum() != at(index + 1).signum()
            })
            .count();
        // About 780 Hz on average over that span: two crossings per cycle.
        assert!((150..=160).contains(&crossings), "{crossings}");
    }

    #[test]
    fn built_in_sounds_are_short_quiet_and_end_in_silence() {
        for sound in OWN {
            let data = samples(sound);
            let seconds = data.len() as f32 / RATE as f32;
            assert!((0.1..=0.8).contains(&seconds), "{sound:?}: {seconds}s");
            let peak = data.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            assert!((peak - VOLUME).abs() < 1e-4, "{sound:?}: peak {peak}");
            // No click at either end.
            assert!(data[0].abs() < 0.01, "{sound:?} starts with a click");
            assert!(
                data.last().unwrap().abs() < 0.01,
                "{sound:?} ends with a click"
            );
        }
    }

    #[test]
    fn the_system_sound_and_none_synthesize_nothing() {
        assert!(samples(NotificationSound::System).is_empty());
        assert!(samples(NotificationSound::None).is_empty());
    }
}
