//! Offline, leakage-resistant evaluation of calibrated genre profiles.
//!
//! This module is test-only. Its opt-in private test reads the live Rekordbox
//! library and Reklawdbox cache through read-only adapters, never loads or
//! persists the user's stored profile registry. The evaluation writes only
//! aggregate results; the separate embedding extractor writes an explicitly
//! requested private manifest outside Git.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::adapters::{audio, rekordbox, state};
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_rekordbox_connection,
};
use crate::domain::classification::{
    AudioBackendStatus, AudioFeatures, ClassificationConfidence, ClassificationMode,
    ClassificationResult, TrackEvidence, broad, engine::classify_track_with_profiles, profiles,
    taxonomy,
};
use crate::domain::library::Track;
use crate::domain::metadata as normalize;

use super::evidence::build_track_evidence;

const EXPERIMENT_ID: &str = "genre-profile-grouped-cv-v2-expanded-corpus";
const AUDIT_EXPERIMENT_ID: &str = "genre-audit-consensus-v1";
const BROAD_EXPERIMENT_ID: &str = "broad-genre-parent-consensus-v1";
const DEVELOPMENT_CORPUS_FINGERPRINT: &str =
    "sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061";
const FOLD_COUNT: usize = 5;
const AUDIT_EXCLUSION_PLAYLISTS: [&str; 8] = [
    "genre_verified",
    "genre_reference_candidates",
    "genre_discovery_blind_v1",
    "genre_discovery_v2_tech_house_batch_01",
    "genre_discovery_v3_tech_house_batch_01",
    "minimal_candidates",
    "minimal_research_candidates_v2",
    "tech_house_research_candidates_v2",
];

struct PreparedRow {
    truth: &'static str,
    track: Track,
    evidence: TrackEvidence,
    artist_keys: Vec<String>,
    release_key: Option<String>,
    related_title_key: Option<String>,
    fold: usize,
}

#[derive(Debug, Clone)]
struct LeakageGroup {
    members: Vec<usize>,
    genre_counts: BTreeMap<&'static str, usize>,
    stable_key: String,
}

#[derive(Debug, Clone)]
struct Prediction {
    truth: &'static str,
    predicted: Option<&'static str>,
    confidence: ClassificationConfidence,
    manual_review: bool,
    fold: usize,
}

#[derive(Debug, Clone)]
struct BroadPrediction {
    truth: &'static str,
    predicted: Option<&'static str>,
    fold: usize,
}

#[derive(Debug, Default, Clone, Serialize)]
struct ExclusionCounts {
    empty_genre: usize,
    unknown_genre: usize,
    unresolved_audio_identity: usize,
    missing_stratum: usize,
    invalid_stratum: usize,
    missing_essentia: usize,
    invalid_essentia: usize,
    incomplete_audio: usize,
    unscorable_audio: usize,
}

#[derive(Debug, Clone, Serialize)]
struct CoverageSummary {
    playlist_rows: usize,
    canonical_rows: usize,
    usable_rows: usize,
    excluded_rows: usize,
    exclusions: ExclusionCounts,
    usable_by_genre: BTreeMap<String, usize>,
    leakage_groups: usize,
    largest_leakage_group: usize,
    rows_by_fold: Vec<usize>,
    groups_by_fold: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct PerGenreMetrics {
    support: usize,
    predicted: usize,
    exact: usize,
    abstentions: usize,
    recall: f64,
    precision: f64,
    f1: f64,
    leading_confusions: Vec<ConfusionMetric>,
}

#[derive(Debug, Clone, Serialize)]
struct ConfusionMetric {
    recommended: String,
    count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct FoldMetric {
    fold: usize,
    support: usize,
    exact_accuracy: f64,
    macro_f1: f64,
}

#[derive(Debug, Clone, Serialize)]
struct ClassifierMetrics {
    support: usize,
    exact: usize,
    exact_accuracy: f64,
    macro_recall: f64,
    macro_f1: f64,
    same_family_accuracy: f64,
    same_family_confusion_rate: f64,
    abstentions: usize,
    abstention_rate: f64,
    manual_reviews: usize,
    manual_review_rate: f64,
    high_medium_recommendations: usize,
    high_medium_exact: usize,
    high_medium_precision: f64,
    per_genre: BTreeMap<String, PerGenreMetrics>,
    folds: Vec<FoldMetric>,
}

#[derive(Debug, Clone, Serialize)]
struct BroadTargetMetrics {
    support: usize,
    offers: usize,
    correct_offers: usize,
    abstentions: usize,
    offered_precision: f64,
    recall: f64,
    f1: f64,
    leading_confusions: Vec<ConfusionMetric>,
}

#[derive(Debug, Clone, Serialize)]
struct BroadFoldMetric {
    fold: usize,
    eligible_rows: usize,
    offers: usize,
    correct_offers: usize,
    coverage: f64,
    offered_precision: f64,
}

#[derive(Debug, Clone, Serialize)]
struct BroadMetrics {
    eligible_rows: usize,
    offers: usize,
    correct_offers: usize,
    abstentions: usize,
    coverage: f64,
    offered_precision: f64,
    accuracy: f64,
    macro_recall: f64,
    macro_f1: f64,
    per_target: BTreeMap<String, BroadTargetMetrics>,
    folds: Vec<BroadFoldMetric>,
}

#[derive(Debug, Clone, Serialize)]
struct BroadConfigurations {
    unselective_projection: BroadMetrics,
    current_confident_projection: BroadMetrics,
    parent_consensus: BroadMetrics,
}

#[derive(Debug, Clone, Serialize)]
struct BroadPromotionGate {
    offered_precision_at_least_0_90: bool,
    coverage_at_least_0_50: bool,
    every_fold_precision_at_least_0_85: bool,
    supported_target_precision_at_least_0_75: bool,
    precision_improvement_at_least_0_10: bool,
    supported_target_failures: Vec<String>,
    passed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct BroadEvaluationResult {
    experiment_id: &'static str,
    method_status: &'static str,
    corpus_fingerprint: String,
    rule_version: &'static str,
    semantic_sha256: String,
    taxonomy_genres: usize,
    broad_targets: usize,
    usable_rows: usize,
    eligible_rows: usize,
    excluded_unmodeled_truth_rows: usize,
    configurations: BroadConfigurations,
    gate: BroadPromotionGate,
    outcome: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct MetricDeltas {
    exact_accuracy: f64,
    macro_recall: f64,
    macro_f1: f64,
    same_family_accuracy: f64,
    abstention_rate: f64,
}

#[derive(Debug, Clone, Serialize)]
struct PromotionGate {
    macro_f1_improvement: bool,
    macro_recall_improvement: bool,
    exact_accuracy_improvement: bool,
    same_family_non_regression: bool,
    per_genre_recall_non_regression: bool,
    truth_profile_coverage: bool,
    passed: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ProfileEvaluationResult {
    experiment_id: &'static str,
    method_status: &'static str,
    taxonomy_genres: usize,
    classifier_profile_schema_version: &'static str,
    stratum_schema_version: &'static str,
    essentia_schema_version: &'static str,
    corpus_fingerprint: String,
    folds: usize,
    coverage: CoverageSummary,
    baseline: ClassifierMetrics,
    fold_trained_profiles: ClassifierMetrics,
    deltas: MetricDeltas,
    truth_profile_coverage_count: usize,
    truth_profile_coverage_rate: f64,
    genres_losing_more_than_0_15_recall: Vec<String>,
    gate: PromotionGate,
    outcome: &'static str,
}

#[derive(Debug, Serialize)]
struct PrivateEmbeddingManifest {
    experiment_id: &'static str,
    stage: &'static str,
    corpus_fingerprint: String,
    fold_count: usize,
    patches_per_track: usize,
    model_url: &'static str,
    model_sha256: &'static str,
    metadata_url: &'static str,
    metadata_sha256: &'static str,
    rows: Vec<PrivateEmbeddingRow>,
}

#[derive(Debug, Serialize)]
struct PrivateEmbeddingRow {
    row_index: usize,
    file_path: String,
    truth: &'static str,
    fold: usize,
    baseline_recommendation: Option<&'static str>,
    arrangement_dynamic: [Option<f64>; 4],
}

#[derive(Debug, Serialize)]
struct PrivateGenreAuditManifest {
    experiment_id: &'static str,
    stage: &'static str,
    development_corpus_fingerprint: &'static str,
    candidate_corpus_fingerprint: String,
    patches_per_track: usize,
    model_url: &'static str,
    model_sha256: &'static str,
    metadata_url: &'static str,
    metadata_sha256: &'static str,
    library_rows: usize,
    excluded_playlist_rows: usize,
    missing_file_rows: usize,
    candidate_input_rows: usize,
    canonical_candidate_rows: usize,
    usable_rows: usize,
    exclusions: ExclusionCounts,
    exclusion_playlists: Vec<&'static str>,
    rows: Vec<PrivateGenreAuditRow>,
}

#[derive(Debug, Serialize)]
struct PrivateGenreAuditRow {
    row_index: usize,
    track_id: String,
    file_path: String,
    artist: String,
    title: String,
    album: String,
    current_genre: &'static str,
    bpm: f64,
    baseline_recommendation: Option<&'static str>,
    baseline_confidence: ClassificationConfidence,
    arrangement_dynamic: [Option<f64>; 4],
}

struct LoadedRows {
    rows: Vec<PreparedRow>,
    playlist_rows: usize,
    canonical_rows: usize,
    exclusions: ExclusionCounts,
    corpus_fingerprint: String,
}

#[derive(Debug)]
struct UnionFind {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl UnionFind {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
            size: vec![1; len],
        }
    }

    fn find(&mut self, value: usize) -> usize {
        if self.parent[value] != value {
            self.parent[value] = self.find(self.parent[value]);
        }
        self.parent[value]
    }

    fn union(&mut self, left: usize, right: usize) {
        let mut left_root = self.find(left);
        let mut right_root = self.find(right);
        if left_root == right_root {
            return;
        }
        if self.size[left_root] < self.size[right_root] {
            std::mem::swap(&mut left_root, &mut right_root);
        }
        self.parent[right_root] = left_root;
        self.size[left_root] += self.size[right_root];
    }
}

fn fraction(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn safe_f1(precision: f64, recall: f64) -> f64 {
    if precision + recall <= f64::EPSILON {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

fn is_generic_credit(value: &str) -> bool {
    matches!(
        value,
        "" | "va" | "various" | "variousartist" | "variousartists" | "unknown" | "unknownartist"
    )
}

fn credit_keys(raw: &str) -> Vec<String> {
    let mut prepared = raw.to_ascii_lowercase();
    for delimiter in [
        " featuring ",
        " feat. ",
        " feat ",
        " presents ",
        " pres. ",
        " vs. ",
        " vs ",
        " & ",
        " + ",
        " / ",
        ",",
        ";",
    ] {
        prepared = prepared.replace(delimiter, "|");
    }
    let mut keys = BTreeSet::new();
    for part in prepared.split('|') {
        let key = normalize::normalize_for_matching(part);
        if !is_generic_credit(&key) {
            keys.insert(key);
        }
    }
    keys.into_iter().collect()
}

fn base_title(raw: &str) -> String {
    let mut title = raw.trim();
    if let Some(index) = title.find(['(', '[']) {
        title = title[..index].trim();
    }
    let normalized = normalize::normalize_for_matching(title);
    for suffix in [
        "remastered",
        "remaster",
        "originalmix",
        "extendedmix",
        "radioedit",
        "edit",
        "remix",
        "version",
        "mix",
    ] {
        if let Some(stripped) = normalized.strip_suffix(suffix)
            && !stripped.is_empty()
        {
            return stripped.to_string();
        }
    }
    normalized
}

fn leakage_descriptors(track: &Track) -> (Vec<String>, Option<String>, Option<String>) {
    let mut artist_keys = credit_keys(&track.artist);
    artist_keys.extend(credit_keys(&track.remixer));
    artist_keys.sort();
    artist_keys.dedup();

    let album = normalize::normalize_for_matching(&track.album);
    let label = normalize::normalize_for_matching(&track.label);
    let release_key = if album.is_empty() {
        None
    } else {
        Some(format!("{album}:{}:{label}", track.year))
    };
    let related = base_title(&track.title);
    let primary_artist = artist_keys.first().cloned().unwrap_or_default();
    let related_title_key = if primary_artist.is_empty() || related.is_empty() {
        None
    } else {
        Some(format!("{primary_artist}:{related}"))
    };
    (artist_keys, release_key, related_title_key)
}

fn build_leakage_groups(rows: &[PreparedRow]) -> Vec<LeakageGroup> {
    let mut union = UnionFind::new(rows.len());
    let mut first_for_key: HashMap<String, usize> = HashMap::new();

    for (index, row) in rows.iter().enumerate() {
        let keys = row
            .artist_keys
            .iter()
            .map(|key| format!("artist:{key}"))
            .chain(row.release_key.iter().map(|key| format!("release:{key}")))
            .chain(
                row.related_title_key
                    .iter()
                    .map(|key| format!("related:{key}")),
            );
        for key in keys {
            if let Some(previous) = first_for_key.insert(key, index) {
                union.union(index, previous);
            }
        }
    }

    let mut members_by_root: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..rows.len() {
        let root = union.find(index);
        members_by_root.entry(root).or_default().push(index);
    }

    let mut groups = Vec::with_capacity(members_by_root.len());
    for members in members_by_root.into_values() {
        let mut genres = BTreeMap::new();
        let mut stable_parts = Vec::new();
        for &index in &members {
            let row = &rows[index];
            *genres.entry(row.truth).or_insert(0) += 1;
            stable_parts.push(format!(
                "{}:{}:{}:{}",
                row.truth,
                row.artist_keys.join("+"),
                row.release_key.as_deref().unwrap_or(""),
                row.related_title_key.as_deref().unwrap_or("")
            ));
        }
        stable_parts.sort();
        groups.push(LeakageGroup {
            members,
            genre_counts: genres,
            stable_key: format!("{:x}", Sha256::digest(stable_parts.join("\n").as_bytes())),
        });
    }
    groups
}

fn assign_groups_to_folds(groups: &[LeakageGroup], fold_count: usize) -> Vec<usize> {
    assert!(fold_count > 1);
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|left, right| {
        groups[*right]
            .members
            .len()
            .cmp(&groups[*left].members.len())
            .then_with(|| groups[*left].stable_key.cmp(&groups[*right].stable_key))
    });

    let mut total_by_genre: BTreeMap<&'static str, usize> = BTreeMap::new();
    let total_rows: usize = groups.iter().map(|group| group.members.len()).sum();
    for group in groups {
        for (&genre, &count) in &group.genre_counts {
            *total_by_genre.entry(genre).or_insert(0) += count;
        }
    }

    let mut rows_by_fold = vec![0usize; fold_count];
    let mut genre_by_fold = vec![BTreeMap::<&'static str, usize>::new(); fold_count];
    let mut assigned = vec![0usize; groups.len()];
    for group_index in order {
        let group = &groups[group_index];
        let mut candidates: Vec<(f64, usize, usize)> = (0..fold_count)
            .map(|fold| {
                let total_target = total_rows as f64 / fold_count as f64;
                let total_after = rows_by_fold[fold] + group.members.len();
                let mut score = (total_after as f64 / total_target.max(1.0)).powi(2) * 0.25;
                for (&genre, &addition) in &group.genre_counts {
                    let target = total_by_genre[&genre] as f64 / fold_count as f64;
                    let after = genre_by_fold[fold].get(genre).copied().unwrap_or(0) + addition;
                    score += (after as f64 / target.max(1.0)).powi(2);
                }
                (score, rows_by_fold[fold], fold)
            })
            .collect();
        candidates.sort_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        let fold = candidates[0].2;
        assigned[group_index] = fold;
        rows_by_fold[fold] += group.members.len();
        for (&genre, &count) in &group.genre_counts {
            *genre_by_fold[fold].entry(genre).or_insert(0) += count;
        }
    }
    assigned
}

fn apply_fold_assignment(rows: &mut [PreparedRow], groups: &[LeakageGroup], folds: &[usize]) {
    for (group, &fold) in groups.iter().zip(folds) {
        for &index in &group.members {
            rows[index].fold = fold;
        }
    }
}

fn prediction(row: &PreparedRow, result: ClassificationResult) -> Prediction {
    Prediction {
        truth: row.truth,
        predicted: result.genre,
        confidence: result.confidence,
        manual_review: result.review_required(),
        fold: row.fold,
    }
}

fn aggregate_metrics(predictions: &[Prediction], include_folds: bool) -> ClassifierMetrics {
    let support = predictions.len();
    let exact = predictions
        .iter()
        .filter(|row| row.predicted == Some(row.truth))
        .count();
    let same_family = predictions
        .iter()
        .filter(|row| {
            row.predicted.is_some_and(|predicted| {
                taxonomy::genre_family(predicted) == taxonomy::genre_family(row.truth)
            })
        })
        .count();
    let same_family_confusions = predictions
        .iter()
        .filter(|row| {
            row.predicted.is_some_and(|predicted| {
                predicted != row.truth
                    && taxonomy::genre_family(predicted) == taxonomy::genre_family(row.truth)
            })
        })
        .count();
    let abstentions = predictions
        .iter()
        .filter(|row| row.predicted.is_none())
        .count();
    let manual_reviews = predictions.iter().filter(|row| row.manual_review).count();
    let high_medium: Vec<_> = predictions
        .iter()
        .filter(|row| {
            row.predicted.is_some()
                && matches!(
                    row.confidence,
                    ClassificationConfidence::High | ClassificationConfidence::Medium
                )
        })
        .collect();
    let high_medium_exact = high_medium
        .iter()
        .filter(|row| row.predicted == Some(row.truth))
        .count();

    let truth_genres: BTreeSet<_> = predictions.iter().map(|row| row.truth).collect();
    let mut per_genre = BTreeMap::new();
    for truth in truth_genres {
        let genre_rows: Vec<_> = predictions
            .iter()
            .filter(|row| row.truth == truth)
            .collect();
        let genre_support = genre_rows.len();
        let predicted = predictions
            .iter()
            .filter(|row| row.predicted == Some(truth))
            .count();
        let genre_exact = genre_rows
            .iter()
            .filter(|row| row.predicted == Some(truth))
            .count();
        let genre_abstentions = genre_rows
            .iter()
            .filter(|row| row.predicted.is_none())
            .count();
        let recall = fraction(genre_exact, genre_support);
        let precision = fraction(genre_exact, predicted);
        let mut confusions: BTreeMap<String, usize> = BTreeMap::new();
        for row in genre_rows {
            if row.predicted != Some(truth) {
                let label = row.predicted.unwrap_or("<abstain>").to_string();
                *confusions.entry(label).or_insert(0) += 1;
            }
        }
        let mut leading_confusions: Vec<_> = confusions
            .into_iter()
            .map(|(recommended, count)| ConfusionMetric { recommended, count })
            .collect();
        leading_confusions.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.recommended.cmp(&right.recommended))
        });
        leading_confusions.truncate(3);
        per_genre.insert(
            truth.to_string(),
            PerGenreMetrics {
                support: genre_support,
                predicted,
                exact: genre_exact,
                abstentions: genre_abstentions,
                recall,
                precision,
                f1: safe_f1(precision, recall),
                leading_confusions,
            },
        );
    }
    let macro_recall = if per_genre.is_empty() {
        0.0
    } else {
        per_genre.values().map(|genre| genre.recall).sum::<f64>() / per_genre.len() as f64
    };
    let macro_f1 = if per_genre.is_empty() {
        0.0
    } else {
        per_genre.values().map(|genre| genre.f1).sum::<f64>() / per_genre.len() as f64
    };

    let folds = if include_folds {
        (0..FOLD_COUNT)
            .map(|fold| {
                let fold_rows: Vec<_> = predictions
                    .iter()
                    .filter(|row| row.fold == fold)
                    .cloned()
                    .collect();
                let metrics = aggregate_metrics(&fold_rows, false);
                FoldMetric {
                    fold,
                    support: metrics.support,
                    exact_accuracy: metrics.exact_accuracy,
                    macro_f1: metrics.macro_f1,
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    ClassifierMetrics {
        support,
        exact,
        exact_accuracy: fraction(exact, support),
        macro_recall,
        macro_f1,
        same_family_accuracy: fraction(same_family, support),
        same_family_confusion_rate: fraction(same_family_confusions, support),
        abstentions,
        abstention_rate: fraction(abstentions, support),
        manual_reviews,
        manual_review_rate: fraction(manual_reviews, support),
        high_medium_recommendations: high_medium.len(),
        high_medium_exact,
        high_medium_precision: fraction(high_medium_exact, high_medium.len()),
        per_genre,
        folds,
    }
}

fn broad_prediction(
    row: &PreparedRow,
    result: &ClassificationResult,
    selector: fn(&ClassificationResult) -> Option<&'static str>,
) -> Option<BroadPrediction> {
    Some(BroadPrediction {
        truth: broad::broad_genre(row.truth)?,
        predicted: selector(result),
        fold: row.fold,
    })
}

fn aggregate_broad_metrics(predictions: &[BroadPrediction], include_folds: bool) -> BroadMetrics {
    let eligible_rows = predictions.len();
    let offers = predictions
        .iter()
        .filter(|prediction| prediction.predicted.is_some())
        .count();
    let correct_offers = predictions
        .iter()
        .filter(|prediction| prediction.predicted == Some(prediction.truth))
        .count();
    let abstentions = eligible_rows.saturating_sub(offers);

    let truth_targets: BTreeSet<_> = predictions.iter().map(|row| row.truth).collect();
    let mut per_target = BTreeMap::new();
    for truth in truth_targets {
        let truth_rows: Vec<_> = predictions
            .iter()
            .filter(|prediction| prediction.truth == truth)
            .collect();
        let support = truth_rows.len();
        let target_offers = predictions
            .iter()
            .filter(|prediction| prediction.predicted == Some(truth))
            .count();
        let target_correct = truth_rows
            .iter()
            .filter(|prediction| prediction.predicted == Some(truth))
            .count();
        let target_abstentions = truth_rows
            .iter()
            .filter(|prediction| prediction.predicted.is_none())
            .count();
        let offered_precision = fraction(target_correct, target_offers);
        let recall = fraction(target_correct, support);
        let mut confusions = BTreeMap::new();
        for prediction in truth_rows {
            if prediction.predicted != Some(truth) {
                let label = prediction.predicted.unwrap_or("<abstain>").to_string();
                *confusions.entry(label).or_insert(0usize) += 1;
            }
        }
        let mut leading_confusions: Vec<_> = confusions
            .into_iter()
            .map(|(recommended, count)| ConfusionMetric { recommended, count })
            .collect();
        leading_confusions.sort_by(|left, right| {
            right
                .count
                .cmp(&left.count)
                .then_with(|| left.recommended.cmp(&right.recommended))
        });
        leading_confusions.truncate(3);
        per_target.insert(
            truth.to_string(),
            BroadTargetMetrics {
                support,
                offers: target_offers,
                correct_offers: target_correct,
                abstentions: target_abstentions,
                offered_precision,
                recall,
                f1: safe_f1(offered_precision, recall),
                leading_confusions,
            },
        );
    }

    let macro_recall = if per_target.is_empty() {
        0.0
    } else {
        per_target.values().map(|target| target.recall).sum::<f64>() / per_target.len() as f64
    };
    let macro_f1 = if per_target.is_empty() {
        0.0
    } else {
        per_target.values().map(|target| target.f1).sum::<f64>() / per_target.len() as f64
    };
    let folds = if include_folds {
        (0..FOLD_COUNT)
            .map(|fold| {
                let fold_rows: Vec<_> = predictions
                    .iter()
                    .filter(|prediction| prediction.fold == fold)
                    .collect();
                let fold_offers = fold_rows
                    .iter()
                    .filter(|prediction| prediction.predicted.is_some())
                    .count();
                let fold_correct = fold_rows
                    .iter()
                    .filter(|prediction| prediction.predicted == Some(prediction.truth))
                    .count();
                BroadFoldMetric {
                    fold,
                    eligible_rows: fold_rows.len(),
                    offers: fold_offers,
                    correct_offers: fold_correct,
                    coverage: fraction(fold_offers, fold_rows.len()),
                    offered_precision: fraction(fold_correct, fold_offers),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    BroadMetrics {
        eligible_rows,
        offers,
        correct_offers,
        abstentions,
        coverage: fraction(offers, eligible_rows),
        offered_precision: fraction(correct_offers, offers),
        accuracy: fraction(correct_offers, eligible_rows),
        macro_recall,
        macro_f1,
        per_target,
        folds,
    }
}

fn broad_semantic_sha256() -> String {
    let mapping = taxonomy::GENRES
        .iter()
        .map(|genre| {
            format!(
                "{genre}=>{}",
                broad::broad_genre(genre).unwrap_or("<unmodeled>")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{:x}",
        Sha256::digest(format!("{}\n{mapping}", broad::RULE_VERSION).as_bytes())
    )
}

fn broad_gate(unselective: &BroadMetrics, candidate: &BroadMetrics) -> BroadPromotionGate {
    let offered_precision_at_least_0_90 = candidate.offered_precision >= 0.90 - f64::EPSILON;
    let coverage_at_least_0_50 = candidate.coverage >= 0.50 - f64::EPSILON;
    let every_fold_precision_at_least_0_85 = candidate
        .folds
        .iter()
        .all(|fold| fold.offers > 0 && fold.offered_precision >= 0.85 - f64::EPSILON);
    let supported_target_failures: Vec<_> = candidate
        .per_target
        .iter()
        .filter(|(_, target)| {
            target.support >= 10
                && target.offers >= 5
                && target.offered_precision < 0.75 - f64::EPSILON
        })
        .map(|(target, _)| target.clone())
        .collect();
    let supported_target_precision_at_least_0_75 = supported_target_failures.is_empty();
    let precision_improvement_at_least_0_10 =
        candidate.offered_precision >= unselective.offered_precision + 0.10 - f64::EPSILON;
    let passed = offered_precision_at_least_0_90
        && coverage_at_least_0_50
        && every_fold_precision_at_least_0_85
        && supported_target_precision_at_least_0_75
        && precision_improvement_at_least_0_10;
    BroadPromotionGate {
        offered_precision_at_least_0_90,
        coverage_at_least_0_50,
        every_fold_precision_at_least_0_85,
        supported_target_precision_at_least_0_75,
        precision_improvement_at_least_0_10,
        supported_target_failures,
        passed,
    }
}

fn cache_identity_fingerprint(rows: &[PreparedRow]) -> String {
    let mut parts: Vec<_> = rows
        .iter()
        .map(|row| {
            format!(
                "{}:{}:{}:{}:{}",
                row.truth, row.track.id, row.track.file_path, row.track.artist, row.track.album
            )
        })
        .collect();
    parts.sort();
    format!("sha256:{:x}", Sha256::digest(parts.join("\n").as_bytes()))
}

fn load_private_rows() -> Result<LoadedRows, String> {
    let db_path = rekordbox::resolve_db_path()
        .ok_or_else(|| "could not resolve Rekordbox master.db".to_string())?;
    let rekordbox_conn = rekordbox::open(&db_path).map_err(|error| error.to_string())?;
    let playlist = rekordbox::get_playlists(&rekordbox_conn)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|playlist| playlist.name.eq_ignore_ascii_case("genre_verified"))
        .ok_or_else(|| "playlist 'genre_verified' not found".to_string())?;
    let tracks = rekordbox::get_playlist_tracks_unbounded(&rekordbox_conn, &playlist.id, None)
        .map_err(|error| error.to_string())?;
    load_rows_from_tracks(&rekordbox_conn, tracks)
}

fn load_rows_from_tracks(
    rekordbox_conn: &Connection,
    tracks: Vec<Track>,
) -> Result<LoadedRows, String> {
    let playlist_rows = tracks.len();

    let identities = audio_cache_identities_with_rekordbox_connection(
        tracks.iter().map(|track| track.file_path.as_str()),
        rekordbox_conn,
    );
    let store_path = state::resolve_path();
    let store_path = store_path
        .to_str()
        .ok_or_else(|| "internal store path is not valid UTF-8".to_string())?;
    let store = state::open_read_only(store_path).map_err(|error| error.to_string())?;

    let normalized: Vec<_> = tracks
        .iter()
        .zip(identities)
        .map(|(track, identity)| {
            let artist = normalize::normalize_for_matching(&track.artist);
            let title = normalize::normalize_for_matching(&track.title);
            let album = normalize::normalize_for_matching(&track.album);
            (artist, title, album, identity)
        })
        .collect();
    let enrichment_keys: Vec<_> = normalized
        .iter()
        .map(|(artist, title, album, _)| {
            ("discogs", artist.as_str(), title.as_str(), album.as_str())
        })
        .collect();
    let stratum_identities: Vec<_> = normalized
        .iter()
        .filter_map(|(_, _, _, identity)| identity.as_ref()?.as_stratum_store_identity())
        .collect();
    let essentia_identities: Vec<_> = normalized
        .iter()
        .filter_map(|(_, _, _, identity)| {
            identity
                .as_ref()
                .map(AudioCacheIdentity::as_essentia_store_identity)
        })
        .collect();
    let enrichment =
        state::batch_get_enrichment(&store, &enrichment_keys).map_err(|error| error.to_string())?;
    let stratum = state::batch_get_fresh_audio_analysis(
        &store,
        &stratum_identities,
        audio::ANALYZER_STRATUM,
        audio::STRATUM_SCHEMA_VERSION,
    )
    .map_err(|error| error.to_string())?;
    let essentia = state::batch_get_fresh_audio_analysis(
        &store,
        &essentia_identities,
        audio::ANALYZER_ESSENTIA,
        audio::ESSENTIA_SCHEMA_VERSION,
    )
    .map_err(|error| error.to_string())?;

    let mut exclusions = ExclusionCounts::default();
    let mut canonical_rows = 0usize;
    let mut rows = Vec::new();
    for (track, (artist, title, album, identity)) in tracks.into_iter().zip(normalized) {
        if track.genre.trim().is_empty() {
            exclusions.empty_genre += 1;
            continue;
        }
        let Some(truth) = taxonomy::resolve_genre(&track.genre) else {
            exclusions.unknown_genre += 1;
            continue;
        };
        canonical_rows += 1;
        let Some(identity) = identity else {
            exclusions.unresolved_audio_identity += 1;
            continue;
        };
        let enrichment_key = ("discogs".to_string(), artist, title, album);
        let mut evaluation_track = track.clone();
        evaluation_track.genre.clear();
        let evidence = build_track_evidence(
            &evaluation_track,
            enrichment.get(&enrichment_key),
            stratum.get(&identity.cache_key),
            essentia.get(&identity.cache_key),
            &[],
        );
        match evidence.stratum_status {
            AudioBackendStatus::Fresh => {}
            AudioBackendStatus::Missing => exclusions.missing_stratum += 1,
            AudioBackendStatus::Invalid => exclusions.invalid_stratum += 1,
        }
        match evidence.essentia_status {
            AudioBackendStatus::Fresh => {}
            AudioBackendStatus::Missing => exclusions.missing_essentia += 1,
            AudioBackendStatus::Invalid => exclusions.invalid_essentia += 1,
        }
        if evidence.stratum_status != AudioBackendStatus::Fresh
            || evidence.essentia_status != AudioBackendStatus::Fresh
        {
            exclusions.incomplete_audio += 1;
            continue;
        }
        let Some(features) = evidence.audio.as_ref() else {
            exclusions.unscorable_audio += 1;
            continue;
        };
        if !profiles::has_scorable_optional_features(features) {
            exclusions.unscorable_audio += 1;
            continue;
        }
        let (artist_keys, release_key, related_title_key) = leakage_descriptors(&track);
        rows.push(PreparedRow {
            truth,
            track,
            evidence,
            artist_keys,
            release_key,
            related_title_key,
            fold: 0,
        });
    }
    let corpus_fingerprint = cache_identity_fingerprint(&rows);
    Ok(LoadedRows {
        rows,
        playlist_rows,
        canonical_rows,
        exclusions,
        corpus_fingerprint,
    })
}

fn run_private_evaluation() -> Result<ProfileEvaluationResult, String> {
    let LoadedRows {
        mut rows,
        playlist_rows,
        canonical_rows,
        exclusions,
        corpus_fingerprint,
    } = load_private_rows()?;
    if rows.is_empty() {
        return Err("no usable genre_verified rows".to_string());
    }
    let groups = build_leakage_groups(&rows);
    let folds = assign_groups_to_folds(&groups, FOLD_COUNT);
    apply_fold_assignment(&mut rows, &groups, &folds);

    let baseline_predictions: Vec<_> = rows
        .iter()
        .map(|row| prediction(row, classify_track_with_profiles(&row.evidence, None)))
        .collect();
    let mut profile_predictions = Vec::with_capacity(rows.len());
    let mut truth_profile_coverage_count = 0usize;
    for fold in 0..FOLD_COUNT {
        let training: Vec<(&'static str, &AudioFeatures)> = rows
            .iter()
            .filter(|row| row.fold != fold)
            .filter_map(|row| Some((row.truth, row.evidence.audio.as_ref()?)))
            .collect();
        let registry = profiles::calibrate(&training);
        for row in rows.iter().filter(|row| row.fold == fold) {
            let audio = row
                .evidence
                .audio
                .as_ref()
                .expect("usable evaluation row has audio");
            if profiles::can_score_genre(audio, &registry, row.truth) {
                truth_profile_coverage_count += 1;
            }
            profile_predictions.push(prediction(
                row,
                classify_track_with_profiles(&row.evidence, Some(&registry)),
            ));
        }
    }
    profile_predictions.sort_by_key(|row| row.fold);

    let baseline = aggregate_metrics(&baseline_predictions, true);
    let fold_trained_profiles = aggregate_metrics(&profile_predictions, true);
    let deltas = MetricDeltas {
        exact_accuracy: fold_trained_profiles.exact_accuracy - baseline.exact_accuracy,
        macro_recall: fold_trained_profiles.macro_recall - baseline.macro_recall,
        macro_f1: fold_trained_profiles.macro_f1 - baseline.macro_f1,
        same_family_accuracy: fold_trained_profiles.same_family_accuracy
            - baseline.same_family_accuracy,
        abstention_rate: fold_trained_profiles.abstention_rate - baseline.abstention_rate,
    };
    let truth_profile_coverage_rate = fraction(truth_profile_coverage_count, rows.len());
    let genres_losing_more_than_0_15_recall: Vec<_> = baseline
        .per_genre
        .iter()
        .filter_map(|(genre, baseline_genre)| {
            let profile_genre = fold_trained_profiles.per_genre.get(genre)?;
            (baseline_genre.support >= 10
                && profile_genre.recall < baseline_genre.recall - 0.15 - f64::EPSILON)
                .then_some(genre.clone())
        })
        .collect();
    let macro_f1_improvement = deltas.macro_f1 + f64::EPSILON >= 0.05;
    let macro_recall_improvement = deltas.macro_recall + f64::EPSILON >= 0.05;
    let exact_accuracy_improvement = deltas.exact_accuracy + f64::EPSILON >= 0.03;
    let same_family_non_regression = deltas.same_family_accuracy + f64::EPSILON >= -0.02;
    let per_genre_recall_non_regression = genres_losing_more_than_0_15_recall.is_empty();
    let truth_profile_coverage = truth_profile_coverage_rate + f64::EPSILON >= 0.80;
    let passed = macro_f1_improvement
        && macro_recall_improvement
        && exact_accuracy_improvement
        && same_family_non_regression
        && per_genre_recall_non_regression
        && truth_profile_coverage;

    let mut usable_by_genre = BTreeMap::new();
    let mut rows_by_fold = vec![0usize; FOLD_COUNT];
    for row in &rows {
        *usable_by_genre.entry(row.truth.to_string()).or_insert(0) += 1;
        rows_by_fold[row.fold] += 1;
    }
    let mut groups_by_fold = vec![0usize; FOLD_COUNT];
    for fold in folds {
        groups_by_fold[fold] += 1;
    }
    let coverage = CoverageSummary {
        playlist_rows,
        canonical_rows,
        usable_rows: rows.len(),
        excluded_rows: playlist_rows.saturating_sub(rows.len()),
        exclusions,
        usable_by_genre,
        leakage_groups: groups.len(),
        largest_leakage_group: groups
            .iter()
            .map(|group| group.members.len())
            .max()
            .unwrap_or(0),
        rows_by_fold,
        groups_by_fold,
    };
    Ok(ProfileEvaluationResult {
        experiment_id: EXPERIMENT_ID,
        method_status: "pre_registered_expanded_corpus_development_evaluation",
        taxonomy_genres: taxonomy::GENRES.len(),
        classifier_profile_schema_version: profiles::PROFILE_SCHEMA_VERSION,
        stratum_schema_version: audio::STRATUM_SCHEMA_VERSION,
        essentia_schema_version: audio::ESSENTIA_SCHEMA_VERSION,
        corpus_fingerprint,
        folds: FOLD_COUNT,
        coverage,
        baseline,
        fold_trained_profiles,
        deltas,
        truth_profile_coverage_count,
        truth_profile_coverage_rate,
        genres_losing_more_than_0_15_recall,
        gate: PromotionGate {
            macro_f1_improvement,
            macro_recall_improvement,
            exact_accuracy_improvement,
            same_family_non_regression,
            per_genre_recall_non_regression,
            truth_profile_coverage,
            passed,
        },
        outcome: if passed {
            "profile_representation_passed_development_gate"
        } else {
            "profile_representation_failed_development_gate"
        },
    })
}

fn run_private_broad_evaluation() -> Result<BroadEvaluationResult, String> {
    let LoadedRows {
        mut rows,
        corpus_fingerprint,
        ..
    } = load_private_rows()?;
    if rows.is_empty() {
        return Err("no usable genre_verified rows".to_string());
    }
    let usable_rows = rows.len();
    let groups = build_leakage_groups(&rows);
    let folds = assign_groups_to_folds(&groups, FOLD_COUNT);
    apply_fold_assignment(&mut rows, &groups, &folds);

    let results: Vec<_> = rows
        .iter()
        .map(|row| classify_track_with_profiles(&row.evidence, None))
        .collect();
    let build_predictions = |selector: fn(&ClassificationResult) -> Option<&'static str>| {
        rows.iter()
            .zip(&results)
            .filter_map(|(row, result)| broad_prediction(row, result, selector))
            .collect::<Vec<_>>()
    };
    let unselective =
        aggregate_broad_metrics(&build_predictions(broad::unselective_projection), true);
    let confident = aggregate_broad_metrics(&build_predictions(broad::confident_projection), true);
    let candidate = aggregate_broad_metrics(&build_predictions(broad::parent_consensus), true);
    if unselective.eligible_rows != confident.eligible_rows
        || unselective.eligible_rows != candidate.eligible_rows
    {
        return Err("broad configurations evaluated different truth rows".to_string());
    }
    let gate = broad_gate(&unselective, &candidate);
    let broad_targets = taxonomy::GENRES
        .iter()
        .filter_map(|genre| broad::broad_genre(genre))
        .collect::<BTreeSet<_>>()
        .len();
    Ok(BroadEvaluationResult {
        experiment_id: BROAD_EXPERIMENT_ID,
        method_status: "pre_registered_deterministic_development_evaluation",
        corpus_fingerprint,
        rule_version: broad::RULE_VERSION,
        semantic_sha256: broad_semantic_sha256(),
        taxonomy_genres: taxonomy::GENRES.len(),
        broad_targets,
        usable_rows,
        eligible_rows: candidate.eligible_rows,
        excluded_unmodeled_truth_rows: usable_rows.saturating_sub(candidate.eligible_rows),
        configurations: BroadConfigurations {
            unselective_projection: unselective,
            current_confident_projection: confident,
            parent_consensus: candidate,
        },
        outcome: if gate.passed {
            "parent_consensus_passed_development_gate"
        } else {
            "parent_consensus_failed_development_gate"
        },
        gate,
    })
}

fn build_private_embedding_manifest() -> Result<PrivateEmbeddingManifest, String> {
    let LoadedRows {
        mut rows,
        corpus_fingerprint,
        ..
    } = load_private_rows()?;
    if rows.is_empty() {
        return Err("no usable genre_verified rows".to_string());
    }
    let groups = build_leakage_groups(&rows);
    let folds = assign_groups_to_folds(&groups, FOLD_COUNT);
    apply_fold_assignment(&mut rows, &groups, &folds);
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(row_index, row)| {
            let audio = row
                .evidence
                .audio
                .as_ref()
                .expect("usable embedding row has audio");
            let baseline_recommendation = classify_track_with_profiles(&row.evidence, None).genre;
            PrivateEmbeddingRow {
                row_index,
                file_path: row.track.file_path,
                truth: row.truth,
                fold: row.fold,
                baseline_recommendation,
                arrangement_dynamic: [
                    audio.loudness_range,
                    audio.dynamic_complexity,
                    audio.spectral_flux_mean,
                    audio.onset_rate,
                ],
            }
        })
        .collect();
    Ok(PrivateEmbeddingManifest {
        experiment_id: EXPERIMENT_ID,
        stage: "discogs_effnet_v2_expanded_corpus",
        corpus_fingerprint,
        fold_count: FOLD_COUNT,
        patches_per_track: 12,
        model_url: "https://essentia.upf.edu/models/feature-extractors/discogs-effnet/discogs-effnet-bsdynamic-1.onnx",
        model_sha256: "a280825b334797cf677939db8cd5762c0392aedd0ca6415dbc1cd083f045e43c",
        metadata_url: "https://essentia.upf.edu/models/feature-extractors/discogs-effnet/discogs-effnet-bsdynamic-1.json",
        metadata_sha256: "a2e85b2e7372d5f8e0f35bdd6aeae1139f101087d183d0b2fb60b0ea0f01a0ff",
        rows,
    })
}

fn build_private_genre_audit_manifest() -> Result<PrivateGenreAuditManifest, String> {
    let db_path = rekordbox::resolve_db_path()
        .ok_or_else(|| "could not resolve Rekordbox master.db".to_string())?;
    let rekordbox_conn = rekordbox::open(&db_path).map_err(|error| error.to_string())?;
    let playlists = rekordbox::get_playlists(&rekordbox_conn).map_err(|error| error.to_string())?;

    let mut excluded_ids = BTreeSet::new();
    let mut verified_tracks = None;
    for required_name in AUDIT_EXCLUSION_PLAYLISTS {
        let playlist = playlists
            .iter()
            .find(|playlist| playlist.name.eq_ignore_ascii_case(required_name))
            .ok_or_else(|| {
                format!("required audit exclusion playlist '{required_name}' not found")
            })?;
        let tracks = rekordbox::get_playlist_tracks_unbounded(&rekordbox_conn, &playlist.id, None)
            .map_err(|error| error.to_string())?;
        if required_name == "genre_verified" {
            verified_tracks = Some(tracks.clone());
        }
        excluded_ids.extend(tracks.into_iter().map(|track| track.id));
    }

    let verified = load_rows_from_tracks(
        &rekordbox_conn,
        verified_tracks.ok_or_else(|| "genre_verified rows were not loaded".to_string())?,
    )?;
    if verified.corpus_fingerprint != DEVELOPMENT_CORPUS_FINGERPRINT {
        return Err(format!(
            "genre_verified development fingerprint changed: expected {DEVELOPMENT_CORPUS_FINGERPRINT}, found {}",
            verified.corpus_fingerprint
        ));
    }

    let library_tracks = rekordbox::search_tracks_unbounded(
        &rekordbox_conn,
        &rekordbox::SearchParams {
            exclude_samples: true,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let library_rows = library_tracks.len();
    let unexposed: Vec<_> = library_tracks
        .into_iter()
        .filter(|track| !excluded_ids.contains(&track.id))
        .collect();
    let missing_file_rows = unexposed
        .iter()
        .filter(|track| !Path::new(&track.file_path).is_file())
        .count();
    let candidate_tracks: Vec<_> = unexposed
        .into_iter()
        .filter(|track| Path::new(&track.file_path).is_file())
        .collect();
    let candidate_input_rows = candidate_tracks.len();
    let LoadedRows {
        rows,
        canonical_rows,
        exclusions,
        corpus_fingerprint,
        ..
    } = load_rows_from_tracks(&rekordbox_conn, candidate_tracks)?;

    let mut audit_rows = Vec::with_capacity(rows.len());
    for (row_index, row) in rows.into_iter().enumerate() {
        let baseline = classify_track_with_profiles(&row.evidence, None);
        if !baseline.current_genre.is_empty() {
            return Err("audit baseline unexpectedly retained current genre".to_string());
        }
        let audio = row
            .evidence
            .audio
            .as_ref()
            .expect("usable audit row has audio");
        audit_rows.push(PrivateGenreAuditRow {
            row_index,
            track_id: row.track.id.clone(),
            file_path: row.track.file_path.clone(),
            artist: row.track.artist.clone(),
            title: row.track.title.clone(),
            album: row.track.album.clone(),
            current_genre: row.truth,
            bpm: row.track.bpm,
            baseline_recommendation: baseline.genre,
            baseline_confidence: baseline.confidence,
            arrangement_dynamic: [
                audio.loudness_range,
                audio.dynamic_complexity,
                audio.spectral_flux_mean,
                audio.onset_rate,
            ],
        });
    }

    Ok(PrivateGenreAuditManifest {
        experiment_id: AUDIT_EXPERIMENT_ID,
        stage: "whole_library_consensus_audit_v1",
        development_corpus_fingerprint: DEVELOPMENT_CORPUS_FINGERPRINT,
        candidate_corpus_fingerprint: corpus_fingerprint,
        patches_per_track: 12,
        model_url: "https://essentia.upf.edu/models/feature-extractors/discogs-effnet/discogs-effnet-bsdynamic-1.onnx",
        model_sha256: "a280825b334797cf677939db8cd5762c0392aedd0ca6415dbc1cd083f045e43c",
        metadata_url: "https://essentia.upf.edu/models/feature-extractors/discogs-effnet/discogs-effnet-bsdynamic-1.json",
        metadata_sha256: "a2e85b2e7372d5f8e0f35bdd6aeae1139f101087d183d0b2fb60b0ea0f01a0ff",
        library_rows,
        excluded_playlist_rows: excluded_ids.len(),
        missing_file_rows,
        candidate_input_rows,
        canonical_candidate_rows: canonical_rows,
        usable_rows: audit_rows.len(),
        exclusions,
        exclusion_playlists: AUDIT_EXCLUSION_PLAYLISTS.to_vec(),
        rows: audit_rows,
    })
}

fn write_result(path: &Path, result: &ProfileEvaluationResult) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(result).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::classification::{ClassificationAction, ClassificationDegradedReason};
    use crate::domain::library::FileKind;

    fn track(id: &str, artist: &str, album: &str, title: &str, genre: &str) -> Track {
        Track {
            id: id.to_string(),
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            genre: genre.to_string(),
            bpm: 126.0,
            key: String::new(),
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: "Label".to_string(),
            remixer: String::new(),
            year: 2020,
            length: 300,
            file_path: format!("/{id}.flac"),
            play_count: 0,
            bit_rate: 0,
            sample_rate: 44_100,
            file_kind: FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    fn evidence(track: &Track) -> TrackEvidence {
        build_track_evidence(track, None, None, None, &[])
    }

    fn prepared(track: Track, truth: &'static str) -> PreparedRow {
        let (artist_keys, release_key, related_title_key) = leakage_descriptors(&track);
        PreparedRow {
            truth,
            evidence: evidence(&track),
            track,
            artist_keys,
            release_key,
            related_title_key,
            fold: 0,
        }
    }

    fn prediction_row(
        truth: &'static str,
        predicted: Option<&'static str>,
        confidence: ClassificationConfidence,
        fold: usize,
    ) -> Prediction {
        Prediction {
            truth,
            predicted,
            confidence,
            manual_review: matches!(
                confidence,
                ClassificationConfidence::Low | ClassificationConfidence::Insufficient
            ),
            fold,
        }
    }

    #[test]
    fn artist_release_and_related_versions_never_split_groups() {
        let rows = vec![
            prepared(track("a", "Artist", "Release", "Track", "House"), "House"),
            prepared(
                track("b", "Artist", "Other", "Other Track", "Techno"),
                "Techno",
            ),
            prepared(
                track("c", "Guest", "Release", "Different", "House"),
                "House",
            ),
            prepared(
                track("d", "Artist", "Third", "Track (Remix)", "House"),
                "House",
            ),
        ];
        let groups = build_leakage_groups(&rows);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 4);
    }

    #[test]
    fn group_assignment_is_deterministic_and_keeps_members_together() {
        let rows: Vec<_> = (0..20)
            .map(|index| {
                let genre = if index % 2 == 0 { "House" } else { "Techno" };
                prepared(
                    track(
                        &format!("id-{index}"),
                        &format!("artist-{}", index / 2),
                        &format!("album-{}", index / 2),
                        &format!("title-{index}"),
                        genre,
                    ),
                    genre,
                )
            })
            .collect();
        let groups = build_leakage_groups(&rows);
        let first = assign_groups_to_folds(&groups, FOLD_COUNT);
        let second = assign_groups_to_folds(&groups, FOLD_COUNT);
        assert_eq!(first, second);
        assert!(first.iter().copied().collect::<BTreeSet<_>>().len() > 1);
        for (group, fold) in groups.iter().zip(first) {
            assert!(group.members.iter().all(|_| fold < FOLD_COUNT));
        }
    }

    #[test]
    fn metrics_count_abstention_as_recall_and_accuracy_failure() {
        let rows = vec![
            prediction_row("House", Some("House"), ClassificationConfidence::High, 0),
            prediction_row("House", None, ClassificationConfidence::Insufficient, 1),
            prediction_row(
                "Techno",
                Some("Deep Techno"),
                ClassificationConfidence::Low,
                2,
            ),
        ];
        let metrics = aggregate_metrics(&rows, true);
        assert_eq!(metrics.support, 3);
        assert_eq!(metrics.exact, 1);
        assert!((metrics.exact_accuracy - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(metrics.per_genre["House"].abstentions, 1);
        assert!((metrics.per_genre["House"].recall - 0.5).abs() < 1e-12);
        assert_eq!(metrics.per_genre["Techno"].recall, 0.0);
        assert!((metrics.same_family_accuracy - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn evaluation_input_can_remove_current_genre_without_losing_truth() {
        let original = track("a", "Artist", "Release", "Track", "House");
        let truth = taxonomy::resolve_genre(&original.genre).unwrap();
        let mut evaluation_track = original;
        evaluation_track.genre.clear();
        let evidence = evidence(&evaluation_track);
        assert_eq!(truth, "House");
        assert!(evidence.current_genre.is_empty());
    }

    #[test]
    fn fold_training_filter_excludes_every_held_out_row() {
        let rows: Vec<_> = (0..10)
            .map(|index| {
                let mut row = prepared(
                    track(
                        &format!("id-{index}"),
                        &format!("artist-{index}"),
                        &format!("album-{index}"),
                        &format!("title-{index}"),
                        "House",
                    ),
                    "House",
                );
                row.fold = index % FOLD_COUNT;
                row
            })
            .collect();
        for fold in 0..FOLD_COUNT {
            let training_ids: BTreeSet<_> = rows
                .iter()
                .filter(|row| row.fold != fold)
                .map(|row| row.track.id.as_str())
                .collect();
            assert!(
                rows.iter()
                    .filter(|row| row.fold == fold)
                    .all(|row| !training_ids.contains(row.track.id.as_str()))
            );
        }
    }

    #[test]
    fn result_shape_does_not_serialize_track_identity_fields() {
        let result = ProfileEvaluationResult {
            experiment_id: EXPERIMENT_ID,
            method_status: "test",
            taxonomy_genres: 1,
            classifier_profile_schema_version: "1",
            stratum_schema_version: "1",
            essentia_schema_version: "1",
            corpus_fingerprint: "sha256:test".to_string(),
            folds: FOLD_COUNT,
            coverage: CoverageSummary {
                playlist_rows: 0,
                canonical_rows: 0,
                usable_rows: 0,
                excluded_rows: 0,
                exclusions: ExclusionCounts::default(),
                usable_by_genre: BTreeMap::new(),
                leakage_groups: 0,
                largest_leakage_group: 0,
                rows_by_fold: vec![],
                groups_by_fold: vec![],
            },
            baseline: aggregate_metrics(&[], true),
            fold_trained_profiles: aggregate_metrics(&[], true),
            deltas: MetricDeltas {
                exact_accuracy: 0.0,
                macro_recall: 0.0,
                macro_f1: 0.0,
                same_family_accuracy: 0.0,
                abstention_rate: 0.0,
            },
            truth_profile_coverage_count: 0,
            truth_profile_coverage_rate: 0.0,
            genres_losing_more_than_0_15_recall: vec![],
            gate: PromotionGate {
                macro_f1_improvement: false,
                macro_recall_improvement: false,
                exact_accuracy_improvement: false,
                same_family_non_regression: true,
                per_genre_recall_non_regression: true,
                truth_profile_coverage: false,
                passed: false,
            },
            outcome: "test",
        };
        let json = serde_json::to_string(&result).unwrap();
        for forbidden in ["track_id", "file_path", "artist", "album", "title"] {
            assert!(
                !json.contains(forbidden),
                "serialized private field {forbidden}"
            );
        }
    }

    #[test]
    fn broad_metrics_measure_precision_and_coverage_separately() {
        let predictions = vec![
            BroadPrediction {
                truth: "House",
                predicted: Some("House"),
                fold: 0,
            },
            BroadPrediction {
                truth: "House",
                predicted: None,
                fold: 0,
            },
            BroadPrediction {
                truth: "Techno",
                predicted: Some("House"),
                fold: 0,
            },
            BroadPrediction {
                truth: "Techno",
                predicted: Some("Techno"),
                fold: 0,
            },
        ];
        let metrics = aggregate_broad_metrics(&predictions, false);
        assert_eq!(metrics.eligible_rows, 4);
        assert_eq!(metrics.offers, 3);
        assert_eq!(metrics.correct_offers, 2);
        assert_eq!(metrics.abstentions, 1);
        assert!((metrics.coverage - 0.75).abs() < f64::EPSILON);
        assert!((metrics.offered_precision - (2.0 / 3.0)).abs() < f64::EPSILON);
        assert!((metrics.accuracy - 0.5).abs() < f64::EPSILON);
        assert_eq!(metrics.per_target["House"].support, 2);
        assert_eq!(metrics.per_target["House"].offers, 2);
        assert_eq!(metrics.per_target["House"].correct_offers, 1);
    }

    #[test]
    fn broad_result_shape_does_not_serialize_track_identity_fields() {
        let metrics = aggregate_broad_metrics(
            &[BroadPrediction {
                truth: "House",
                predicted: Some("House"),
                fold: 0,
            }],
            true,
        );
        let result = BroadEvaluationResult {
            experiment_id: BROAD_EXPERIMENT_ID,
            method_status: "test",
            corpus_fingerprint: "sha256:test".into(),
            rule_version: broad::RULE_VERSION,
            semantic_sha256: broad_semantic_sha256(),
            taxonomy_genres: taxonomy::GENRES.len(),
            broad_targets: 1,
            usable_rows: 1,
            eligible_rows: 1,
            excluded_unmodeled_truth_rows: 0,
            configurations: BroadConfigurations {
                unselective_projection: metrics.clone(),
                current_confident_projection: metrics.clone(),
                parent_consensus: metrics,
            },
            gate: BroadPromotionGate {
                offered_precision_at_least_0_90: true,
                coverage_at_least_0_50: true,
                every_fold_precision_at_least_0_85: false,
                supported_target_precision_at_least_0_75: true,
                precision_improvement_at_least_0_10: false,
                supported_target_failures: Vec::new(),
                passed: false,
            },
            outcome: "test",
        };
        let json = serde_json::to_string(&result).unwrap();
        for forbidden in ["track_id", "file_path", "artist", "album", "title"] {
            assert!(
                !json.contains(forbidden),
                "serialized private field {forbidden}"
            );
        }
    }

    #[test]
    #[ignore = "requires private Rekordbox library and current audio cache"]
    fn private_grouped_genre_profile_evaluation() {
        let output = std::env::var("REKLAWDBOX_PROFILE_EVALUATION_OUTPUT")
            .expect("set REKLAWDBOX_PROFILE_EVALUATION_OUTPUT");
        let result = run_private_evaluation().expect("private profile evaluation failed");
        write_result(Path::new(&output), &result).expect("write private profile evaluation result");
        println!(
            "PROFILE_EVALUATION_SUMMARY_JSON={}",
            serde_json::json!({
                "output_path": output,
                "experiment_id": result.experiment_id,
                "coverage": result.coverage,
                "deltas": result.deltas,
                "truth_profile_coverage_rate": result.truth_profile_coverage_rate,
                "gate": result.gate,
                "outcome": result.outcome,
            })
        );
    }

    #[test]
    #[ignore = "requires private Rekordbox library and current audio cache"]
    fn private_broad_genre_evaluation() {
        let output = std::env::var("REKLAWDBOX_BROAD_GENRE_EVALUATION_OUTPUT")
            .expect("set REKLAWDBOX_BROAD_GENRE_EVALUATION_OUTPUT");
        let result = run_private_broad_evaluation().expect("private broad genre evaluation failed");
        std::fs::write(
            &output,
            serde_json::to_vec_pretty(&result).expect("serialize broad evaluation result"),
        )
        .expect("write private broad evaluation result");
        println!(
            "BROAD_GENRE_EVALUATION_SUMMARY_JSON={}",
            serde_json::json!({
                "output_path": output,
                "experiment_id": result.experiment_id,
                "corpus_fingerprint": result.corpus_fingerprint,
                "semantic_sha256": result.semantic_sha256,
                "eligible_rows": result.eligible_rows,
                "configurations": result.configurations,
                "gate": result.gate,
                "outcome": result.outcome,
            })
        );
    }

    #[test]
    #[ignore = "requires private Rekordbox library and current audio cache"]
    fn private_discogs_effnet_manifest() {
        let output = std::env::var("REKLAWDBOX_EMBEDDING_MANIFEST_OUTPUT")
            .expect("set REKLAWDBOX_EMBEDDING_MANIFEST_OUTPUT");
        let manifest =
            build_private_embedding_manifest().expect("build private embedding manifest");
        std::fs::write(
            &output,
            serde_json::to_vec_pretty(&manifest).expect("serialize private embedding manifest"),
        )
        .expect("write private embedding manifest");
        println!(
            "EMBEDDING_MANIFEST_SUMMARY_JSON={}",
            serde_json::json!({
                "output_path": output,
                "experiment_id": manifest.experiment_id,
                "stage": manifest.stage,
                "corpus_fingerprint": manifest.corpus_fingerprint,
                "fold_count": manifest.fold_count,
                "patches_per_track": manifest.patches_per_track,
                "rows": manifest.rows.len(),
                "model_sha256": manifest.model_sha256,
                "metadata_sha256": manifest.metadata_sha256,
            })
        );
    }

    #[test]
    #[ignore = "requires private Rekordbox library and current audio cache"]
    fn private_genre_audit_manifest() {
        let output = std::env::var("REKLAWDBOX_GENRE_AUDIT_MANIFEST_OUTPUT")
            .expect("set REKLAWDBOX_GENRE_AUDIT_MANIFEST_OUTPUT");
        let manifest =
            build_private_genre_audit_manifest().expect("build private genre audit manifest");
        std::fs::write(
            &output,
            serde_json::to_vec_pretty(&manifest).expect("serialize private genre audit manifest"),
        )
        .expect("write private genre audit manifest");
        println!(
            "GENRE_AUDIT_MANIFEST_SUMMARY_JSON={}",
            serde_json::json!({
                "output_path": output,
                "experiment_id": manifest.experiment_id,
                "stage": manifest.stage,
                "development_corpus_fingerprint": manifest.development_corpus_fingerprint,
                "candidate_corpus_fingerprint": manifest.candidate_corpus_fingerprint,
                "library_rows": manifest.library_rows,
                "excluded_playlist_rows": manifest.excluded_playlist_rows,
                "missing_file_rows": manifest.missing_file_rows,
                "candidate_input_rows": manifest.candidate_input_rows,
                "canonical_candidate_rows": manifest.canonical_candidate_rows,
                "usable_rows": manifest.usable_rows,
                "exclusions": manifest.exclusions,
            })
        );
    }

    #[allow(dead_code)]
    fn _assert_result_types_remain_constructible(
        _action: ClassificationAction,
        _mode: ClassificationMode,
        _reasons: Vec<ClassificationDegradedReason>,
    ) {
    }
}
