//! Prints the tempo of each file given on the command line.
//! Usage: cargo run --release --example analyze -- <file>...

use std::path::Path;
use std::time::Instant;

use bpm_detector::decode::decode_file;
use bpm_detector::tempo::estimate_tempo;

fn main() {
    for argument in std::env::args().skip(1) {
        let started = Instant::now();
        let audio = match decode_file(Path::new(&argument)) {
            Ok(audio) => audio,
            Err(error) => {
                println!("{argument}: {error}");
                continue;
            }
        };
        let decoded_in = started.elapsed();
        match estimate_tempo(&audio.samples, audio.sample_rate) {
            Ok(estimate) => println!(
                "{argument}: {:.5} BPM | {:?} | {:?} | periodicity {:.3} | {:.1}s @ {} Hz | decode {:.2}s, total {:.2}s",
                estimate.bpm,
                estimate.confidence,
                estimate.stability,
                estimate.periodicity,
                audio.duration_seconds(),
                audio.sample_rate,
                decoded_in.as_secs_f64(),
                started.elapsed().as_secs_f64()
            ),
            Err(error) => println!("{argument}: {error}"),
        }
    }
}
