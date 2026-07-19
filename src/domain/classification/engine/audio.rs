use super::super::taxonomy::GenreFamily;
use super::super::{AudioFeatures, taxonomy as genre};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EnergyBucket {
    NonDancefloor,
    LowEnergy,
    Dancefloor,
    HighEnergy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum CharFlag {
    Ambient,
    Atmospheric,
    Broken,
    Irregular,
    Fast,
    Slow,
    /// `key_confidence` in `(0.0, 0.1)`: no clear tonal centre.
    Atonal,
    /// `decay_mid_tau > 200ms`: lingering mid-band decay/reverb tail.
    LongTail,
    /// `loudness_range < 1 LU` on a full-length track: compressed club master.
    Compressed,
}

pub(super) struct AudioProfile {
    pub(super) bucket: Option<EnergyBucket>,
    pub(super) flags: Vec<CharFlag>,
    pub(super) bpm: f64,
    pub(super) centroid: Option<f64>,
    pub(super) rhythm_regularity: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BpmContext {
    pub(super) effective_bpm: f64,
    pub(super) fallback: Option<BpmFallback>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct BpmFallback {
    pub(super) rekordbox_bpm: f64,
    pub(super) detector_bpm: f64,
}

/// Spectral centroid threshold: sub-bass dominated (ambient, drone, dub techno).
const CENTROID_VERY_LOW: f64 = 600.0;
/// Spectral centroid threshold: dark timbre (dub techno, deep techno).
const CENTROID_DARK: f64 = 1200.0;
/// Mid-band decay threshold for reverb-heavy long-tail material.
const LONG_TAIL_DECAY_MS: f64 = 200.0;
/// Loudness-range threshold for heavily mastered club tracks.
const COMPRESSED_LOUDNESS_RANGE_LU: f64 = 1.0;
/// Short clips can report artificially narrow loudness range.
const COMPRESSED_MIN_DURATION_SECONDS: f64 = 60.0;
/// Maximum relative difference for Stratum/Essentia BPM detector consensus.
const BPM_DETECTOR_CONSENSUS_TOLERANCE: f64 = 0.03;

pub(super) fn profile(audio: &AudioFeatures) -> AudioProfile {
    let bpm = audio.rekordbox_bpm;
    let danceability = audio.danceability.filter(|value| value.is_finite());
    let dynamic_complexity = audio.dynamic_complexity.filter(|value| value.is_finite());
    let rhythm_regularity = audio.rhythm_regularity.filter(|value| value.is_finite());
    let bucket = danceability.map(|danceability| {
        if danceability < 1.0 {
            EnergyBucket::NonDancefloor
        } else if danceability < 1.5 {
            EnergyBucket::LowEnergy
        } else if danceability <= 2.5 {
            EnergyBucket::Dancefloor
        } else {
            EnergyBucket::HighEnergy
        }
    });

    let mut flags = Vec::new();
    if let Some(dc) = dynamic_complexity {
        if dc > 10.0 {
            flags.push(CharFlag::Ambient);
        }
        if dc > 5.0 {
            flags.push(CharFlag::Atmospheric);
        }
    }
    // Caveat: atmospheric + broken/irregular -> lower confidence on rhythm flags.
    // We still set them but the decision tree checks for this combination.
    if let Some(rr) = rhythm_regularity {
        if rr < 0.5 {
            flags.push(CharFlag::Broken);
        } else if rr < 0.8 {
            flags.push(CharFlag::Irregular);
        }
    }
    if bpm > 155.0 {
        flags.push(CharFlag::Fast);
    }
    if bpm < 115.0 {
        flags.push(CharFlag::Slow);
    }
    // `key_confidence == 0.0` is stratum's sentinel for "key detection failed",
    // not atonal music - exclude it so analysis failures aren't relabelled.
    if audio.key_confidence.is_some_and(|kc| kc > 0.0 && kc < 0.1) {
        flags.push(CharFlag::Atonal);
    }
    if audio.decay_mid_tau.is_some_and(|t| t > LONG_TAIL_DECAY_MS) {
        flags.push(CharFlag::LongTail);
    }
    if audio
        .duration_seconds
        .is_some_and(|d| d > COMPRESSED_MIN_DURATION_SECONDS)
        && audio
            .loudness_range
            .is_some_and(|lr| lr < COMPRESSED_LOUDNESS_RANGE_LU)
    {
        flags.push(CharFlag::Compressed);
    }

    AudioProfile {
        bucket,
        flags,
        bpm,
        centroid: audio
            .spectral_centroid_mean
            .filter(|value| value.is_finite()),
        rhythm_regularity,
    }
}

pub(super) fn bpm_context(
    audio: Option<&AudioFeatures>,
    audio_profile: Option<&AudioProfile>,
    fallback_bpm: f64,
) -> BpmContext {
    let default_bpm = audio_profile.map_or(fallback_bpm, |profile| profile.bpm);
    if audio_profile
        .and_then(|profile| profile.bucket)
        .is_none_or(|bucket| bucket < EnergyBucket::Dancefloor)
    {
        return BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        };
    }

    let Some(audio) = audio else {
        return BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        };
    };

    let (Some(false), Some(stratum_bpm), Some(essentia_bpm)) = (
        audio.bpm_agreement,
        audio.stratum_bpm.filter(|bpm| *bpm > 0.0),
        audio.essentia_bpm.filter(|bpm| *bpm > 0.0),
    ) else {
        return BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        };
    };

    let detector_delta = relative_delta(stratum_bpm, essentia_bpm);
    let consensus_bpm = (stratum_bpm + essentia_bpm) / 2.0;
    let rekordbox_delta = relative_delta(default_bpm, consensus_bpm);
    if detector_delta < BPM_DETECTOR_CONSENSUS_TOLERANCE
        && rekordbox_delta > BPM_DETECTOR_CONSENSUS_TOLERANCE
        && !is_near_double_time(default_bpm, consensus_bpm)
    {
        BpmContext {
            effective_bpm: consensus_bpm,
            fallback: Some(BpmFallback {
                rekordbox_bpm: default_bpm,
                detector_bpm: consensus_bpm,
            }),
        }
    } else {
        BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        }
    }
}

pub(super) fn has_flag(profile: &AudioProfile, flag: CharFlag) -> bool {
    profile.flags.contains(&flag)
}

pub(super) fn clearly_favors_family(profile: &AudioProfile, candidate: &str) -> bool {
    let family = genre::genre_family(candidate);
    match family {
        GenreFamily::Downtempo => {
            let very_low_centroid = profile.centroid.is_some_and(|c| c < CENTROID_VERY_LOW);
            (profile.bucket == Some(EnergyBucket::LowEnergy)
                && (has_flag(profile, CharFlag::Atmospheric) || very_low_centroid))
                || (profile.bucket == Some(EnergyBucket::NonDancefloor) && very_low_centroid)
        }
        GenreFamily::Bass => {
            has_flag(profile, CharFlag::Fast)
                || (has_flag(profile, CharFlag::Broken)
                    && profile
                        .bucket
                        .is_some_and(|bucket| bucket >= EnergyBucket::Dancefloor))
        }
        GenreFamily::Techno => {
            let dark_timbre = profile.centroid.is_some_and(|c| c < CENTROID_DARK);
            let long_tail = has_flag(profile, CharFlag::LongTail);
            (profile
                .bucket
                .is_some_and(|bucket| bucket >= EnergyBucket::Dancefloor)
                && profile.rhythm_regularity.is_some()
                && !has_flag(profile, CharFlag::Broken)
                && profile.bpm >= 125.0)
                || (profile.bucket == Some(EnergyBucket::LowEnergy)
                    && profile.rhythm_regularity.is_some()
                    && !has_flag(profile, CharFlag::Broken)
                    && profile.bpm >= 118.0
                    && profile.bpm <= 132.0
                    && (dark_timbre || long_tail))
        }
        GenreFamily::House => {
            profile.bucket == Some(EnergyBucket::Dancefloor)
                && profile.rhythm_regularity.is_some()
                && !has_flag(profile, CharFlag::Broken)
                && !has_flag(profile, CharFlag::Atonal)
                && profile.bpm >= 118.0
                && profile.bpm <= 132.0
        }
        GenreFamily::Hardcore => {
            profile
                .bucket
                .is_some_and(|bucket| bucket >= EnergyBucket::Dancefloor)
                && profile.rhythm_regularity.is_some()
                && !has_flag(profile, CharFlag::Broken)
                && profile.bpm >= 138.0
        }
        _ => false,
    }
}

fn relative_delta(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        return f64::INFINITY;
    }
    (a - b).abs() / a.min(b)
}

fn is_near_double_time(a: f64, b: f64) -> bool {
    if a <= 0.0 || b <= 0.0 {
        return false;
    }
    let ratio = a.max(b) / a.min(b);
    ((ratio - 2.0).abs() / 2.0) < BPM_DETECTOR_CONSENSUS_TOLERANCE
}

impl PartialOrd for EnergyBucket {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EnergyBucket {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |bucket: &EnergyBucket| -> u8 {
            match bucket {
                EnergyBucket::NonDancefloor => 0,
                EnergyBucket::LowEnergy => 1,
                EnergyBucket::Dancefloor => 2,
                EnergyBucket::HighEnergy => 3,
            }
        };
        rank(self).cmp(&rank(other))
    }
}
