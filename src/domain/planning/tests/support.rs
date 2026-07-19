use crate::domain::planning::*;

pub(super) struct ProfileSpec<'a> {
    id: &'a str,
    key: &'a str,
    bpm: f64,
    energy: f64,
    genre: &'a str,
}

impl<'a> ProfileSpec<'a> {
    pub(super) fn new(id: &'a str, key: &'a str, bpm: f64, energy: f64, genre: &'a str) -> Self {
        Self {
            id,
            key,
            bpm,
            energy,
            genre,
        }
    }
}

#[derive(Default)]
pub(super) struct ProfileAnalysis {
    brightness: Option<f64>,
    rhythm: Option<f64>,
    loudness_range: Option<f64>,
}

impl ProfileAnalysis {
    pub(super) fn measured(brightness: f64, rhythm: f64, loudness_range: f64) -> Self {
        Self {
            brightness: Some(brightness),
            rhythm: Some(rhythm),
            loudness_range: Some(loudness_range),
        }
    }
}

pub(super) fn synth_profile(spec: ProfileSpec<'_>, analysis: ProfileAnalysis) -> TrackProfile {
    let ProfileSpec {
        id,
        key,
        bpm,
        energy,
        genre,
    } = spec;
    TrackProfile {
        track: crate::domain::library::Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: "Eval".to_string(),
            album: String::new(),
            genre: genre.to_string(),
            key: key.to_string(),
            bpm,
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 2025,
            length: 360,
            file_path: format!("/eval/{id}.flac"),
            play_count: 0,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: crate::domain::library::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        },
        camelot_key: parse_camelot_key(key),
        key_display: key.to_string(),
        bpm,
        energy,
        brightness: analysis.brightness,
        rhythm_regularity: analysis.rhythm,
        loudness_range: analysis.loudness_range,
        canonical_genre: Some(genre.to_string()),
        genre_family: genre_family_for(genre),
        timbral: None,
    }
}

pub(super) fn simple_profile(
    id: &str,
    key: &str,
    bpm: f64,
    energy: f64,
    genre: &str,
) -> TrackProfile {
    synth_profile(
        ProfileSpec::new(id, key, bpm, energy, genre),
        ProfileAnalysis::default(),
    )
}

pub(super) fn mixing_policy(
    weights: &PriorityWeights,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
) -> TransitionMixingPolicy<'_> {
    TransitionMixingPolicy {
        weights,
        master_tempo,
        harmonic_style,
    }
}

pub(super) fn transition_moment(
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    genre_run_length: u32,
    play_bpms: Option<(f64, f64)>,
) -> TransitionMoment {
    TransitionMoment {
        from_phase,
        to_phase,
        genre_run_length,
        play_bpms,
    }
}

pub(super) fn sequence_policy<'a>(
    target_track_count: usize,
    energy_phases: &'a [EnergyPhase],
    weights: &'a PriorityWeights,
    harmonic_style: HarmonicMixingStyle,
) -> SequencePolicy<'a> {
    SequencePolicy {
        target_track_count,
        energy_phases,
        mixing: mixing_policy(weights, true, Some(harmonic_style)),
        bpm_drift_pct: 6.0,
        target_bpms: None,
    }
}

pub(super) fn pool_scoring_policy<'a>(
    master_tempo: bool,
    reference_bpm: f64,
    weights: &'a PoolWeights,
    timbral_normalization: Option<&'a TimbralNormalization>,
) -> PoolScoringPolicy<'a> {
    PoolScoringPolicy {
        master_tempo,
        reference_bpm,
        weights,
        timbral_normalization,
    }
}

pub(super) fn pool_discovery_bounds(
    threshold: f64,
    minimum_size: usize,
    maximum_size: usize,
    maximum_results: usize,
) -> PoolDiscoveryBounds {
    PoolDiscoveryBounds::new(threshold, minimum_size, maximum_size, maximum_results)
        .expect("test discovery bounds should be valid")
}
