//! Tempo estimation tuned for precision on music with a steady tempo.
//!
//! 1. Onset envelope: log-band spectral flux every ~2.7-2.9 ms.
//! 2. Coarse tempo: strongest autocorrelation peak between 60 and 200 BPM, weighted by a
//!    log-normal preference centred on `PREFERRED_BPM` (picks between half and double tempo).
//! 3. Refinement: the autocorrelation peak is tracked at 1, 2, 4, 8, ... beats, up to half the
//!    track, stopping where the peak collapses. The beat period is then the least-squares fit of
//!    the peaks at every whole number of beats. The autocorrelation is evaluated on a grid eight
//!    times finer than the envelope (exact band-limited interpolation), so the error on the
//!    period shrinks with the number of beats spanned.
//! 4. Stability: the refinement runs again on each half of the track; a mismatch means the
//!    tempo changes and the result is an average.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

const TARGET_WINDOW_SECONDS: f64 = 0.0232;
const HOP_DIVISOR: usize = 8;
const FLUX_LAG_DIVISOR: usize = 4;
const BANDS_PER_OCTAVE: f64 = 6.0;
const LOWEST_BAND_HZ: f64 = 40.0;
const HIGHEST_BAND_HZ: f64 = 16_000.0;
const DYNAMIC_RANGE_DB: f32 = 80.0;
const DETREND_SECONDS: f64 = 2.0;

const MAX_OVERSAMPLING: usize = 8;
const MAX_FINE_POINTS: usize = 1 << 23;

const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 200.0;
const PREFERRED_BPM: f64 = 130.0;
const PREFERENCE_WIDTH_OCTAVES: f64 = 1.0;

/// A refinement stage is rejected when its peak falls below this share of the best peak so far.
const PEAK_COLLAPSE_RATIO: f64 = 0.5;

const MIN_DURATION_SECONDS: f64 = 6.0;
const STABILITY_MIN_SECONDS: f64 = 30.0;
const STABILITY_TOLERANCE_BPM: f64 = 0.05;
const RELIABLE_HALF_CORRELATION: f64 = 0.1;
const HIGH_CONFIDENCE_CORRELATION: f64 = 0.3;
const MEDIUM_CONFIDENCE_CORRELATION: f64 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stability {
    Steady,
    Variable {
        first_half_bpm: f64,
        second_half_bpm: f64,
    },
    /// The track is too short, or one half has no clear beat to compare.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TempoEstimate {
    pub bpm: f64,
    pub confidence: Confidence,
    pub stability: Stability,
    /// Normalized autocorrelation at the longest lag used by the refinement (1.0 = identical).
    pub periodicity: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisError {
    TooShort,
    NoBeat,
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalysisError::TooShort => write!(formatter, "The audio is too short (6 s minimum)"),
            AnalysisError::NoBeat => write!(formatter, "No clear beat found"),
        }
    }
}

pub fn estimate_tempo(samples: &[f32], sample_rate: u32) -> Result<TempoEstimate, AnalysisError> {
    let duration_seconds = samples.len() as f64 / sample_rate as f64;
    if duration_seconds < MIN_DURATION_SECONDS {
        return Err(AnalysisError::TooShort);
    }

    let envelope = OnsetEnvelope::compute(samples, sample_rate);
    let detrend_frames = (DETREND_SECONDS * envelope.frame_rate).round() as usize;
    let detrended = remove_local_mean(&envelope.values, detrend_frames);

    let autocorrelation = Autocorrelation::new(&detrended);
    let coarse_period =
        coarse_period(&autocorrelation, envelope.frame_rate).ok_or(AnalysisError::NoBeat)?;
    let refinement = refine_period(&autocorrelation, coarse_period).ok_or(AnalysisError::NoBeat)?;
    let bpm = 60.0 * envelope.frame_rate / refinement.period;

    let stability = if duration_seconds >= STABILITY_MIN_SECONDS {
        compare_halves(&detrended, refinement.period, envelope.frame_rate)
    } else {
        Stability::Unknown
    };

    let confidence = match (refinement.correlation, stability) {
        (correlation, Stability::Variable { .. })
            if correlation >= MEDIUM_CONFIDENCE_CORRELATION =>
        {
            Confidence::Medium
        }
        (correlation, _) if correlation >= HIGH_CONFIDENCE_CORRELATION => Confidence::High,
        (correlation, _) if correlation >= MEDIUM_CONFIDENCE_CORRELATION => Confidence::Medium,
        _ => Confidence::Low,
    };

    Ok(TempoEstimate {
        bpm,
        confidence,
        stability,
        periodicity: refinement.correlation,
    })
}

struct OnsetEnvelope {
    values: Vec<f64>,
    frame_rate: f64,
}

impl OnsetEnvelope {
    fn compute(samples: &[f32], sample_rate: u32) -> Self {
        let sample_rate_hz = sample_rate as f64;
        let window_size = 1usize << (sample_rate_hz * TARGET_WINDOW_SECONDS).log2().round() as u32;
        let hop_size = window_size / HOP_DIVISOR;
        let frame_rate = sample_rate_hz / hop_size as f64;
        if samples.len() < window_size {
            return Self {
                values: Vec::new(),
                frame_rate,
            };
        }
        let frame_count = 1 + (samples.len() - window_size) / hop_size;

        let window: Vec<f32> = (0..window_size)
            .map(|index| {
                let phase = 2.0 * std::f64::consts::PI * index as f64 / window_size as f64;
                (0.5 - 0.5 * phase.cos()) as f32
            })
            .collect();
        let (band_of_bin, band_count) = log_bands(window_size, sample_rate_hz);

        let fft = FftPlanner::<f32>::new().plan_fft_forward(window_size);
        let mut buffer = vec![Complex::new(0.0f32, 0.0); window_size];
        let mut scratch = vec![Complex::new(0.0f32, 0.0); fft.get_inplace_scratch_len()];
        let mut band_energies = vec![0.0f64; band_count];
        let mut band_levels_db = vec![0.0f32; frame_count * band_count];

        for frame in 0..frame_count {
            let start = frame * hop_size;
            for (slot, (sample, weight)) in buffer
                .iter_mut()
                .zip(samples[start..start + window_size].iter().zip(&window))
            {
                *slot = Complex::new(sample * weight, 0.0);
            }
            fft.process_with_scratch(&mut buffer, &mut scratch);

            band_energies.fill(0.0);
            for (bin, band) in band_of_bin.iter().enumerate() {
                if let Some(band) = band {
                    band_energies[*band] += buffer[bin].norm_sqr() as f64;
                }
            }
            let frame_levels = &mut band_levels_db[frame * band_count..(frame + 1) * band_count];
            for (level, energy) in frame_levels.iter_mut().zip(&band_energies) {
                *level = (10.0 * (energy + 1e-12).log10()) as f32;
            }
        }

        let loudest_db = band_levels_db.iter().copied().fold(f32::MIN, f32::max);
        let floor_db = loudest_db - DYNAMIC_RANGE_DB;
        for level in band_levels_db.iter_mut() {
            *level = level.max(floor_db);
        }

        let flux_lag = (window_size / FLUX_LAG_DIVISOR / hop_size).max(1);
        let mut values = vec![0.0f64; frame_count];
        for frame in flux_lag..frame_count {
            let current = &band_levels_db[frame * band_count..(frame + 1) * band_count];
            let previous = &band_levels_db
                [(frame - flux_lag) * band_count..(frame - flux_lag + 1) * band_count];
            let rise: f64 = current
                .iter()
                .zip(previous)
                .map(|(now, before)| (now - before).max(0.0) as f64)
                .sum();
            values[frame] = rise / band_count as f64;
        }

        Self { values, frame_rate }
    }
}

/// Maps each FFT bin to a logarithmic band (`BANDS_PER_OCTAVE` per octave), dropping empty bands.
fn log_bands(window_size: usize, sample_rate_hz: f64) -> (Vec<Option<usize>>, usize) {
    let highest_hz = HIGHEST_BAND_HZ.min(sample_rate_hz / 2.0);
    let raw_band_of_bin: Vec<Option<usize>> = (0..=window_size / 2)
        .map(|bin| {
            let frequency = bin as f64 * sample_rate_hz / window_size as f64;
            (frequency >= LOWEST_BAND_HZ && frequency < highest_hz)
                .then(|| (BANDS_PER_OCTAVE * (frequency / LOWEST_BAND_HZ).log2()).floor() as usize)
        })
        .collect();

    let mut compact_index = std::collections::BTreeMap::new();
    for band in raw_band_of_bin.iter().flatten() {
        let next_index = compact_index.len();
        compact_index.entry(*band).or_insert(next_index);
    }
    let band_of_bin = raw_band_of_bin
        .iter()
        .map(|band| band.map(|band| compact_index[&band]))
        .collect();
    (band_of_bin, compact_index.len())
}

fn remove_local_mean(values: &[f64], window_frames: usize) -> Vec<f64> {
    let half_window = window_frames / 2;
    let mut prefix_sums = Vec::with_capacity(values.len() + 1);
    prefix_sums.push(0.0);
    for value in values {
        prefix_sums.push(prefix_sums.last().unwrap() + value);
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let start = index.saturating_sub(half_window);
            let end = (index + half_window + 1).min(values.len());
            value - (prefix_sums[end] - prefix_sums[start]) / (end - start) as f64
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Peak {
    lag: f64,
    correlation: f64,
}

/// Linear autocorrelation of the envelope, band-limited-interpolated onto a grid
/// `oversampling` times finer than the envelope frames.
struct Autocorrelation {
    fine_sums: Vec<f64>,
    oversampling: usize,
    length: usize,
    energy_per_frame: f64,
}

impl Autocorrelation {
    fn new(signal: &[f64]) -> Self {
        let length = signal.len().max(1);
        let fft_size = (2 * length).next_power_of_two();
        let mut oversampling = MAX_OVERSAMPLING;
        while oversampling > 1 && fft_size * oversampling > MAX_FINE_POINTS {
            oversampling /= 2;
        }
        let fine_size = fft_size * oversampling;
        let mut planner = FftPlanner::<f64>::new();

        let mut spectrum: Vec<Complex<f64>> = signal
            .iter()
            .map(|value| Complex::new(*value, 0.0))
            .chain(std::iter::repeat(Complex::new(0.0, 0.0)))
            .take(fft_size)
            .collect();
        planner.plan_fft_forward(fft_size).process(&mut spectrum);

        // Zero-pad the power spectrum (keeping its symmetry) to interpolate the correlation.
        let half = fft_size / 2;
        let mut fine = vec![Complex::new(0.0, 0.0); fine_size];
        for bin in 0..half {
            fine[bin] = Complex::new(spectrum[bin].norm_sqr(), 0.0);
        }
        for bin in half + 1..fft_size {
            fine[fine_size - fft_size + bin] = Complex::new(spectrum[bin].norm_sqr(), 0.0);
        }
        let nyquist = spectrum[half].norm_sqr() / 2.0;
        fine[half] = Complex::new(nyquist, 0.0);
        fine[fine_size - half] = Complex::new(nyquist, 0.0);
        planner.plan_fft_inverse(fine_size).process(&mut fine);

        let fine_sums: Vec<f64> = fine[..length * oversampling]
            .iter()
            .map(|value| value.re / fft_size as f64)
            .collect();
        let energy_per_frame = fine_sums[0] / length as f64;

        Self {
            fine_sums,
            oversampling,
            length,
            energy_per_frame,
        }
    }

    fn normalized_fine(&self, index: usize) -> f64 {
        let lag = index as f64 / self.oversampling as f64;
        (self.fine_sums[index] / (self.length as f64 - lag)) / self.energy_per_frame
    }

    /// Correlation at an integer lag, per overlapping frame, relative to lag zero.
    fn normalized_at(&self, lag: usize) -> f64 {
        self.normalized_fine(lag * self.oversampling)
    }

    /// Highest peak strictly inside `[center - half_width, center + half_width]` (in frames).
    fn peak_near(&self, center: f64, half_width: f64) -> Option<Peak> {
        let scale = self.oversampling as f64;
        let lowest = ((center - half_width) * scale).ceil().max(1.0) as usize;
        let highest =
            (((center + half_width) * scale).floor() as usize).min(self.fine_sums.len() - 2);
        if lowest + 2 > highest {
            return None;
        }
        let best = (lowest..=highest).max_by(|left, right| {
            self.normalized_fine(*left)
                .total_cmp(&self.normalized_fine(*right))
        })?;
        if best == lowest || best == highest {
            return None;
        }
        let (previous, value, next) = (
            self.normalized_fine(best - 1),
            self.normalized_fine(best),
            self.normalized_fine(best + 1),
        );
        let offset = parabolic_peak_offset(previous, value, next);
        Some(Peak {
            lag: (best as f64 + offset) / scale,
            correlation: value - 0.25 * (previous - next) * offset,
        })
    }
}

fn preferred_tempo_weight(bpm: f64) -> f64 {
    let octaves = (bpm / PREFERRED_BPM).log2() / PREFERENCE_WIDTH_OCTAVES;
    (-0.5 * octaves * octaves).exp()
}

fn coarse_period(autocorrelation: &Autocorrelation, frame_rate: f64) -> Option<f64> {
    let lag_for_bpm = |bpm: f64| 60.0 * frame_rate / bpm;
    let first_lag = (lag_for_bpm(MAX_BPM * 1.05).floor() as usize).max(2);
    let last_lag = (lag_for_bpm(MIN_BPM / 1.05).ceil() as usize).min(autocorrelation.length / 2);
    if first_lag + 2 >= last_lag {
        return None;
    }

    let mut best: Option<(f64, f64)> = None;
    for lag in first_lag..last_lag {
        let previous = autocorrelation.normalized_at(lag - 1);
        let value = autocorrelation.normalized_at(lag);
        let next = autocorrelation.normalized_at(lag + 1);
        if value <= 0.0 || value < previous || value <= next {
            continue;
        }
        let peak_lag = lag as f64 + parabolic_peak_offset(previous, value, next);
        let bpm = 60.0 * frame_rate / peak_lag;
        if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            continue;
        }
        let score = value * preferred_tempo_weight(bpm);
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, peak_lag));
        }
    }
    best.map(|(_, lag)| lag)
}

fn parabolic_peak_offset(previous: f64, value: f64, next: f64) -> f64 {
    let curvature = previous - 2.0 * value + next;
    if curvature >= 0.0 {
        return 0.0;
    }
    (0.5 * (previous - next) / curvature).clamp(-0.5, 0.5)
}

struct Refinement {
    period: f64,
    correlation: f64,
}

fn refine_period(autocorrelation: &Autocorrelation, start_period: f64) -> Option<Refinement> {
    let longest_lag = autocorrelation.length as f64 / 2.0;
    let search_half_width = |period: f64| (period / 8.0).max(2.0);

    // Lock onto the peaks at 1, 2, 4, 8, ... beats while they stay clear.
    let mut period = start_period;
    let mut correlation = 0.0;
    let mut strongest_correlation: f64 = 0.0;
    let mut longest_beats = 0usize;
    let mut beats = 1usize;
    loop {
        let half_width = search_half_width(period);
        if beats as f64 * period + half_width >= longest_lag {
            break;
        }
        let Some(peak) = autocorrelation.peak_near(beats as f64 * period, half_width) else {
            break;
        };
        if peak.correlation < PEAK_COLLAPSE_RATIO * strongest_correlation {
            break;
        }
        strongest_correlation = strongest_correlation.max(peak.correlation);
        period = peak.lag / beats as f64;
        correlation = peak.correlation;
        longest_beats = beats;
        beats *= 2;
    }
    if longest_beats == 0 {
        return None;
    }

    // Least-squares fit through the peaks at every whole number of beats up to that span.
    let half_width = search_half_width(period);
    let (weighted_products, weighted_squares) = (1..=longest_beats)
        .filter_map(|beats| {
            let peak = autocorrelation.peak_near(beats as f64 * period, half_width)?;
            (peak.correlation >= PEAK_COLLAPSE_RATIO * strongest_correlation)
                .then_some((beats as f64, peak))
        })
        .fold((0.0, 0.0), |(products, squares), (beats, peak)| {
            (
                products + peak.correlation * beats * peak.lag,
                squares + peak.correlation * beats * beats,
            )
        });
    if weighted_squares > 0.0 {
        period = weighted_products / weighted_squares;
    }
    Some(Refinement {
        period,
        correlation,
    })
}

fn compare_halves(detrended: &[f64], period: f64, frame_rate: f64) -> Stability {
    let middle = detrended.len() / 2;
    let halves = [&detrended[..middle], &detrended[middle..]]
        .map(|half| refine_period(&Autocorrelation::new(half), period));
    let [Some(first_half), Some(second_half)] = halves else {
        return Stability::Unknown;
    };
    if first_half.correlation < RELIABLE_HALF_CORRELATION
        || second_half.correlation < RELIABLE_HALF_CORRELATION
    {
        return Stability::Unknown;
    }

    let first_half_bpm = 60.0 * frame_rate / first_half.period;
    let second_half_bpm = 60.0 * frame_rate / second_half.period;
    if (first_half_bpm - second_half_bpm).abs() > STABILITY_TOLERANCE_BPM {
        Stability::Variable {
            first_half_bpm,
            second_half_bpm,
        }
    } else {
        Stability::Steady
    }
}
