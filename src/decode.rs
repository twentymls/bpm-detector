//! Decodes an audio file into a mono signal.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::TrackType;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl DecodedAudio {
    pub fn duration_seconds(&self) -> f64 {
        self.samples.len() as f64 / self.sample_rate as f64
    }
}

/// Reads the default audio track of `path` and averages its channels into one mono signal.
pub fn decode_file(path: &Path) -> Result<DecodedAudio, String> {
    let file = File::open(path).map_err(|error| format!("Cannot open the file: {error}"))?;
    if file.metadata().is_ok_and(|metadata| metadata.len() == 0) {
        return Err("The file is empty".to_string());
    }
    let source = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
        hint.with_extension(extension);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|_| "Unsupported or unreadable audio format".to_string())?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| "The file has no audio track".to_string())?;
    let track_id = track.id;
    let codec_parameters = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .ok_or_else(|| "The file has no audio track".to_string())?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_parameters, &AudioDecoderOptions::default())
        .map_err(|_| "Unsupported audio codec".to_string())?;

    let mut mono_samples: Vec<f32> = Vec::new();
    let mut interleaved: Vec<f32> = Vec::new();
    let mut sample_rate: Option<u32> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(format!("Cannot read the audio: {error}")),
        };
        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(format!("Cannot decode the audio: {error}")),
        };

        let channel_count = decoded.spec().channels().count().max(1);
        sample_rate.get_or_insert(decoded.spec().rate());

        interleaved.resize(decoded.samples_interleaved(), 0.0);
        decoded.copy_to_slice_interleaved(&mut interleaved);
        mono_samples.extend(
            interleaved
                .chunks_exact(channel_count)
                .map(|frame| frame.iter().sum::<f32>() / channel_count as f32),
        );
    }

    let sample_rate = sample_rate.ok_or_else(|| "The file contains no audio".to_string())?;
    Ok(DecodedAudio {
        samples: mono_samples,
        sample_rate,
    })
}
