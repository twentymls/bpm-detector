//! Accuracy checks on synthetic drum tracks whose beats sit at exact (sub-sample) times.
//! Run with `cargo test --release`.

use std::f64::consts::PI;
use std::ops::Range;

use bpm_detector::tempo::{Confidence, Stability, estimate_tempo};

const PRECISION_BPM: f64 = 0.01;

struct NoiseSource(u64);

impl NoiseSource {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

struct TrackBuilder {
    samples: Vec<f64>,
    sample_rate: u32,
    noise: NoiseSource,
}

impl TrackBuilder {
    fn new(duration_seconds: f64, sample_rate: u32) -> Self {
        let mut noise = NoiseSource(0x9E37_79B9_7F4A_7C15);
        let length = (duration_seconds * sample_rate as f64) as usize;
        let samples = (0..length).map(|_| 0.003 * noise.next()).collect();
        Self {
            samples,
            sample_rate,
            noise,
        }
    }

    /// Adds `voice(t)` for t in [0, length) seconds after `start_seconds`, at sub-sample precision.
    fn add_event(
        &mut self,
        start_seconds: f64,
        length_seconds: f64,
        mut voice: impl FnMut(f64, &mut NoiseSource) -> f64,
    ) {
        let sample_rate = self.sample_rate as f64;
        let first = (start_seconds * sample_rate).ceil() as usize;
        let last =
            (((start_seconds + length_seconds) * sample_rate) as usize).min(self.samples.len());
        for index in first..last {
            let time = index as f64 / sample_rate - start_seconds;
            self.samples[index] += voice(time, &mut self.noise);
        }
    }

    fn kick(&mut self, start_seconds: f64) {
        self.add_event(start_seconds, 0.6, |time, _| {
            let phase = 2.0 * PI * (50.0 * time + 70.0 * 0.03 * (1.0 - (-time / 0.03).exp()));
            0.8 * phase.sin() * (-time / 0.15).exp()
        });
    }

    fn snare(&mut self, start_seconds: f64) {
        self.add_event(start_seconds, 0.4, |time, noise| {
            0.35 * noise.next() * (-time / 0.08).exp()
                + 0.25 * (2.0 * PI * 180.0 * time).sin() * (-time / 0.05).exp()
        });
    }

    fn hat(&mut self, start_seconds: f64) {
        let mut previous = 0.0;
        self.add_event(start_seconds, 0.1, move |time, noise| {
            let current = noise.next();
            let high_passed = current - previous;
            previous = current;
            0.15 * high_passed * (-time / 0.02).exp()
        });
    }

    /// Four-on-the-floor kick, hats on every off-beat, optionally a snare on beats 2 and 4.
    fn four_on_the_floor(&mut self, beat_times: &[f64], muted_beats: Range<usize>, backbeat: bool) {
        for (beat, window) in beat_times.windows(2).enumerate() {
            if muted_beats.contains(&beat) {
                continue;
            }
            let (start, next) = (window[0], window[1]);
            self.kick(start);
            if backbeat && beat % 2 == 1 {
                self.snare(start);
            }
            self.hat((start + next) / 2.0);
        }
    }

    /// Kick on beats 1 and 3, snare on 2 and 4, sixteenth-note hats.
    fn sixteenth_groove(&mut self, beat_times: &[f64]) {
        for (beat, window) in beat_times.windows(2).enumerate() {
            let (start, next) = (window[0], window[1]);
            if beat % 2 == 0 {
                self.kick(start);
            } else {
                self.snare(start);
            }
            for sixteenth in 0..4 {
                self.hat(start + (next - start) * sixteenth as f64 / 4.0);
            }
        }
    }

    fn build(self) -> Vec<f32> {
        self.samples
            .into_iter()
            .map(|sample| sample as f32)
            .collect()
    }
}

fn steady_beats(bpm: f64, first_beat_seconds: f64, duration_seconds: f64) -> Vec<f64> {
    let period = 60.0 / bpm;
    (0..)
        .map(|beat| first_beat_seconds + beat as f64 * period)
        .take_while(|time| *time < duration_seconds)
        .collect()
}

fn accelerating_beats(start_bpm: f64, end_bpm: f64, duration_seconds: f64) -> Vec<f64> {
    let mut beat_times = vec![0.0];
    while *beat_times.last().unwrap() < duration_seconds {
        let progress = beat_times.last().unwrap() / duration_seconds;
        let bpm = start_bpm + (end_bpm - start_bpm) * progress;
        beat_times.push(beat_times.last().unwrap() + 60.0 / bpm);
    }
    beat_times
}

fn steady_track(bpm: f64, sample_rate: u32, duration_seconds: f64) -> Vec<f32> {
    let mut track = TrackBuilder::new(duration_seconds, sample_rate);
    track.four_on_the_floor(&steady_beats(bpm, 0.237, duration_seconds), 0..0, true);
    track.build()
}

fn assert_precise(samples: &[f32], sample_rate: u32, expected_bpm: f64) {
    let estimate = estimate_tempo(samples, sample_rate).expect("tempo");
    println!("expected {expected_bpm}: {estimate:?}");
    assert!(
        (estimate.bpm - expected_bpm).abs() < PRECISION_BPM,
        "expected {expected_bpm} BPM, got {:.4}",
        estimate.bpm
    );
    assert_eq!(estimate.stability, Stability::Steady);
    assert_eq!(estimate.confidence, Confidence::High);
}

#[test]
fn finds_128_bpm_at_44_1_khz() {
    assert_precise(&steady_track(128.0, 44_100, 180.0), 44_100, 128.0);
}

#[test]
fn finds_fractional_127_5_bpm_at_48_khz() {
    assert_precise(&steady_track(127.5, 48_000, 120.0), 48_000, 127.5);
}

#[test]
fn finds_130_bpm_at_96_khz() {
    assert_precise(&steady_track(130.0, 96_000, 90.0), 96_000, 130.0);
}

#[test]
fn finds_fast_174_3_bpm() {
    let mut track = TrackBuilder::new(150.0, 44_100);
    track.four_on_the_floor(&steady_beats(174.3, 0.237, 150.0), 0..0, false);
    assert_precise(&track.build(), 44_100, 174.3);
}

/// A snare on 2 and 4 that dominates the mix makes the two-beat cycle the strongest pulse, so a
/// fast track reads as half time. The tempo is still exact; the app shows the double alongside.
#[test]
fn reads_a_fast_track_with_a_dominant_backbeat_as_half_time() {
    let estimate = estimate_tempo(&steady_track(174.3, 44_100, 150.0), 44_100).expect("tempo");
    assert!(
        (estimate.bpm - 174.3 / 2.0).abs() < PRECISION_BPM / 2.0,
        "got {:.4}",
        estimate.bpm
    );
}

#[test]
fn finds_slow_85_bpm() {
    assert_precise(&steady_track(85.0, 44_100, 120.0), 44_100, 85.0);
}

#[test]
fn finds_99_9_bpm_with_sixteenth_hats() {
    let mut track = TrackBuilder::new(150.0, 44_100);
    track.sixteenth_groove(&steady_beats(99.9, 0.05, 150.0));
    assert_precise(&track.build(), 44_100, 99.9);
}

#[test]
fn stays_precise_across_a_breakdown_without_drums() {
    let mut track = TrackBuilder::new(180.0, 44_100);
    track.four_on_the_floor(&steady_beats(126.0, 0.4, 180.0), 160..224, true);
    assert_precise(&track.build(), 44_100, 126.0);
}

#[test]
fn handles_a_short_loop() {
    let estimate = estimate_tempo(&steady_track(128.0, 44_100, 12.0), 44_100).expect("tempo");
    println!("short loop: {estimate:?}");
    assert!(
        (estimate.bpm - 128.0).abs() < 0.05,
        "got {:.4}",
        estimate.bpm
    );
    assert_eq!(estimate.stability, Stability::Unknown);
}

#[test]
fn flags_a_tempo_that_speeds_up() {
    let mut track = TrackBuilder::new(120.0, 44_100);
    track.four_on_the_floor(&accelerating_beats(120.0, 124.0, 120.0), 0..0, true);
    let estimate = estimate_tempo(&track.build(), 44_100).expect("tempo");
    println!("accelerating: {estimate:?}");
    assert!(matches!(estimate.stability, Stability::Variable { .. }));
    assert_ne!(estimate.confidence, Confidence::High);
}

#[test]
fn rejects_audio_shorter_than_six_seconds() {
    assert!(estimate_tempo(&steady_track(128.0, 44_100, 4.0), 44_100).is_err());
}
