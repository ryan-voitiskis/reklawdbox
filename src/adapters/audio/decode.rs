use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

use super::AudioError;

/// Content roots to try when a `djmdContent.FolderPath` is DB-relative
/// (e.g. `/contents_3915412406/...`) rather than absolute.
///
/// Rekordbox stores these paths relative to wherever the collection's audio
/// lives, which is not necessarily under the rekordbox app directory — a
/// Dropbox- or NAS-backed library is common. `REKORDBOX_MEDIA_ROOTS`
/// (colon-separated, like `PATH`) lets the user point at those locations;
/// the default preserves the previous behaviour.
fn media_roots() -> Vec<String> {
    if let Ok(roots) = std::env::var("REKORDBOX_MEDIA_ROOTS") {
        return roots
            .split(':')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(|r| r.trim_end_matches('/').to_string())
            .collect();
    }
    match std::env::var("HOME") {
        Ok(home) => vec![format!("{home}/Library/Pioneer/rekordbox/share")],
        Err(_) => Vec::new(),
    }
}

pub(crate) fn resolve_audio_path(raw_path: &str) -> Result<String, AudioError> {
    resolve_audio_path_in(raw_path, &media_roots())
}

/// Core resolution, with the content roots injected so tests need not mutate
/// process-global environment variables.
fn resolve_audio_path_in(raw_path: &str, roots: &[String]) -> Result<String, AudioError> {
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

    // DB-relative path: try each configured content root.
    for candidate in [raw_path, decoded.as_str()] {
        if !candidate.starts_with('/') {
            continue;
        }
        for root in roots {
            let joined = format!("{root}{candidate}");
            if std::fs::metadata(&joined).is_ok() {
                return Ok(joined);
            }
        }
    }

    Err(AudioError::Io(format!(
        "File not found (tried raw, decoded, and {} media root(s)): {raw_path}",
        roots.len()
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

#[cfg(test)]
mod media_root_tests {
    use super::*;

    #[test]
    fn resolves_db_relative_path_via_media_root() {
        let root = std::env::temp_dir().join("rbx_media_root_test");
        let dir = root.join("contents_1/sub");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("track.mp3");
        std::fs::write(&file, b"x").unwrap();
        let roots = vec![root.to_str().unwrap().to_string()];

        assert_eq!(
            resolve_audio_path_in("/contents_1/sub/track.mp3", &roots).unwrap(),
            file.to_str().unwrap()
        );
        // An absolute, existing path still wins untouched.
        assert_eq!(
            resolve_audio_path_in(file.to_str().unwrap(), &roots).unwrap(),
            file.to_str().unwrap()
        );
        // A path under no root still fails.
        assert!(resolve_audio_path_in("/contents_1/nope.mp3", &roots).is_err());
        // No roots configured => previous behaviour (raw/decoded only).
        assert!(resolve_audio_path_in("/contents_1/sub/track.mp3", &[]).is_err());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn multiple_roots_are_tried_in_order() {
        let base = std::env::temp_dir().join("rbx_media_multi");
        let second = base.join("b");
        std::fs::create_dir_all(second.join("contents_2")).unwrap();
        let file = second.join("contents_2/x.mp3");
        std::fs::write(&file, b"x").unwrap();

        let roots = vec![
            base.join("a").to_str().unwrap().to_string(),
            second.to_str().unwrap().to_string(),
        ];
        assert_eq!(
            resolve_audio_path_in("/contents_2/x.mp3", &roots).unwrap(),
            file.to_str().unwrap()
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn media_roots_parses_colon_separated_list() {
        // Pure string parsing, no filesystem or env mutation.
        let parsed: Vec<String> = "/a/:/b:  :/c/"
            .split(':')
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(|r| r.trim_end_matches('/').to_string())
            .collect();
        assert_eq!(parsed, vec!["/a", "/b", "/c"]);
    }
}
