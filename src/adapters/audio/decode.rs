use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

use super::AudioError;

pub(crate) fn resolve_audio_path(raw_path: &str) -> Result<String, AudioError> {
    if std::fs::metadata(raw_path).is_ok() {
        return Ok(raw_path.to_string());
    }

    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|e| AudioError::Parse(format!("Invalid UTF-8 in file path: {e}")))?
        .to_string();

    if std::fs::metadata(&decoded).is_ok() {
        return Ok(decoded);
    }

    Err(AudioError::Io(format!(
        "File not found (tried raw and decoded): {raw_path}"
    )))
}

pub fn decode_to_samples(path: &str) -> Result<(Vec<f32>, u32), AudioError> {
    let file = std::fs::File::open(path)
        .map_err(|e| AudioError::Io(format!("Failed to open audio file '{path}': {e}")))?;
    let media_source_stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
    {
        hint.with_extension(ext);
    }

    let probed = get_probe()
        .format(
            &hint,
            media_source_stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Decode(format!("Failed to probe audio format: {e}")))?;

    let mut format_reader = probed.format;

    let track = format_reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::Decode("No audio track found in file".to_string()))?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| AudioError::Decode("Audio track has no sample rate".to_string()))?;
    let n_frames_hint = track.codec_params.n_frames;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AudioError::Decode(format!("Failed to create decoder: {e}")))?;

    let mut all_samples: Vec<f32> = Vec::with_capacity(n_frames_hint.unwrap_or(0) as usize);
    let mut decode_warning_count: u64 = 0;

    loop {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(AudioError::Decode(format!("Error reading packet: {e}"))),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                decode_warning_count += 1;
                continue;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                match decoder.decode(&packet) {
                    Ok(d) => d,
                    Err(symphonia::core::errors::Error::DecodeError(_)) => {
                        decode_warning_count += 1;
                        continue;
                    }
                    Err(e) => {
                        return Err(AudioError::Decode(format!("Decode error after reset: {e}")));
                    }
                }
            }
            Err(e) => return Err(AudioError::Decode(format!("Decode error: {e}"))),
        };

        let mono = decode_buffer_to_mono(&decoded);
        all_samples.extend_from_slice(&mono);
    }

    if decode_warning_count > 0 {
        tracing::debug!("{path}: {decode_warning_count} decode warnings (frames skipped)");
    }

    if all_samples.is_empty() {
        return Err(AudioError::Decode("Decoded zero audio samples".to_string()));
    }

    Ok((all_samples, sample_rate))
}

fn decode_buffer_to_mono(buf: &AudioBufferRef) -> Vec<f32> {
    match buf {
        AudioBufferRef::F32(b) => downmix_to_mono(b.planes().planes(), |&v| v),
        AudioBufferRef::F64(b) => downmix_to_mono(b.planes().planes(), |&v| v as f32),
        AudioBufferRef::S8(b) => downmix_to_mono(b.planes().planes(), |&v| v as f32 / 128.0),
        AudioBufferRef::S16(b) => downmix_to_mono(b.planes().planes(), |&v| v as f32 / 32768.0),
        AudioBufferRef::S24(b) => {
            downmix_to_mono(b.planes().planes(), |v| v.inner() as f32 / 8388608.0)
        }
        AudioBufferRef::S32(b) => {
            downmix_to_mono(b.planes().planes(), |&v| v as f32 / 2147483648.0)
        }
        AudioBufferRef::U8(b) => {
            downmix_to_mono(b.planes().planes(), |&v| (v as f32 - 128.0) / 128.0)
        }
        AudioBufferRef::U16(b) => {
            downmix_to_mono(b.planes().planes(), |&v| (v as f32 - 32768.0) / 32768.0)
        }
        AudioBufferRef::U24(b) => downmix_to_mono(b.planes().planes(), |v| {
            (v.inner() as f32 - 8388608.0) / 8388608.0
        }),
        AudioBufferRef::U32(b) => downmix_to_mono(b.planes().planes(), |&v| {
            (v as f64 - 2147483648.0) as f32 / 2147483648.0
        }),
    }
}

pub(crate) fn downmix_to_mono<T, F>(planes: &[&[T]], convert: F) -> Vec<f32>
where
    F: Fn(&T) -> f32,
{
    if planes.is_empty() {
        return Vec::new();
    }
    let num_channels = planes.len();
    let num_frames = planes.iter().map(|ch| ch.len()).min().unwrap_or(0);

    if num_channels == 1 {
        return planes[0].iter().take(num_frames).map(&convert).collect();
    }

    let channel_weight = 1.0 / num_channels as f32;
    (0..num_frames)
        .map(|i| {
            let sum: f32 = planes.iter().map(|ch| convert(&ch[i])).sum();
            sum * channel_weight
        })
        .collect()
}
