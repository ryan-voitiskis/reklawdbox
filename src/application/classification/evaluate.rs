//! Pure aggregate metrics for classifier benchmarks.
//!
//! This module deliberately accepts already-classified rows. It never reads the
//! library, cache, or training registry, which keeps benchmark leakage controls
//! at the caller boundary and makes every denominator independently testable.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::domain::classification::taxonomy;
use crate::domain::classification::{
    ClassificationConfidence, ClassificationMode, ClassificationResult,
};

#[derive(Debug)]
pub(crate) struct EvaluationCase<'a> {
    pub(crate) truth: &'static str,
    pub(crate) result: &'a ClassificationResult,
    pub(crate) source_stratum: &'a str,
    pub(crate) discogs_match_quality: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RateMetric {
    pub(crate) count: usize,
    pub(crate) denominator: usize,
    pub(crate) percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AccuracyMetrics {
    pub(crate) exact: RateMetric,
    pub(crate) same_family: RateMetric,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConfidencePrecision {
    pub(crate) recommendations: usize,
    pub(crate) exact: usize,
    pub(crate) exact_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ConfusionPair {
    pub(crate) expected: String,
    pub(crate) recommended: String,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct StratumMetrics {
    pub(crate) evaluated: usize,
    pub(crate) recommended: usize,
    pub(crate) exact: usize,
    pub(crate) exact_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct EvaluationSummary {
    pub(crate) evaluated: usize,
    pub(crate) skipped: usize,
    pub(crate) canonical_label_counts: BTreeMap<String, usize>,
    pub(crate) accuracy: AccuracyMetrics,
    pub(crate) abstention: RateMetric,
    pub(crate) manual_review: RateMetric,
    pub(crate) confidence_precision: BTreeMap<String, ConfidencePrecision>,
    pub(crate) confusion_pairs: Vec<ConfusionPair>,
    pub(crate) by_source_stratum: BTreeMap<String, StratumMetrics>,
    pub(crate) by_discogs_match_quality: BTreeMap<String, StratumMetrics>,
    pub(crate) by_classification_mode: BTreeMap<String, StratumMetrics>,
}

fn rate(count: usize, denominator: usize) -> RateMetric {
    RateMetric {
        count,
        denominator,
        percent: percentage(count, denominator),
    }
}

fn percentage(count: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        count as f64 * 100.0 / denominator as f64
    }
}

fn confidence_name(confidence: ClassificationConfidence) -> &'static str {
    match confidence {
        ClassificationConfidence::High => "high",
        ClassificationConfidence::Medium => "medium",
        ClassificationConfidence::Low => "low",
        ClassificationConfidence::Insufficient => "insufficient",
    }
}

fn classification_mode_name(mode: ClassificationMode) -> &'static str {
    match mode {
        ClassificationMode::Full => "full",
        ClassificationMode::Degraded => "degraded",
    }
}

pub(crate) fn evaluate(cases: &[EvaluationCase<'_>], skipped: usize) -> EvaluationSummary {
    let evaluated = cases.len();
    let mut labels = BTreeMap::new();
    let mut exact = 0usize;
    let mut same_family = 0usize;
    let mut recommendations = 0usize;
    let mut abstentions = 0usize;
    let mut manual_review = 0usize;
    let mut confidence: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut confusion: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut sources: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut qualities: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let mut modes: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();

    for case in cases {
        *labels.entry(case.truth.to_string()).or_insert(0) += 1;
        if case.result.review_required() {
            manual_review += 1;
        }

        let source = sources
            .entry(case.source_stratum.to_string())
            .or_insert((0, 0, 0));
        source.0 += 1;
        let quality = qualities
            .entry(case.discogs_match_quality.to_string())
            .or_insert((0, 0, 0));
        quality.0 += 1;
        let mode = modes
            .entry(classification_mode_name(case.result.mode).to_string())
            .or_insert((0, 0, 0));
        mode.0 += 1;

        let Some(recommended) = case.result.genre else {
            abstentions += 1;
            continue;
        };
        recommendations += 1;
        source.1 += 1;
        quality.1 += 1;
        mode.1 += 1;

        let is_exact = recommended.eq_ignore_ascii_case(case.truth);
        if is_exact {
            exact += 1;
            source.2 += 1;
            quality.2 += 1;
            mode.2 += 1;
        } else {
            *confusion
                .entry((case.truth.to_string(), recommended.to_string()))
                .or_insert(0) += 1;
        }
        if taxonomy::genre_family(recommended) == taxonomy::genre_family(case.truth) {
            same_family += 1;
        }

        let tier = confidence
            .entry(confidence_name(case.result.confidence).to_string())
            .or_insert((0, 0));
        tier.0 += 1;
        if is_exact {
            tier.1 += 1;
        }
    }

    let confidence_precision = confidence
        .into_iter()
        .map(|(tier, (recommended, correct))| {
            (
                tier,
                ConfidencePrecision {
                    recommendations: recommended,
                    exact: correct,
                    exact_percent: percentage(correct, recommended),
                },
            )
        })
        .collect();
    let mut confusion_pairs: Vec<_> = confusion
        .into_iter()
        .map(|((expected, recommended), count)| ConfusionPair {
            expected,
            recommended,
            count,
        })
        .collect();
    confusion_pairs.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.expected.cmp(&b.expected))
            .then_with(|| a.recommended.cmp(&b.recommended))
    });

    let to_strata = |values: BTreeMap<String, (usize, usize, usize)>| {
        values
            .into_iter()
            .map(|(name, (rows, recommended, correct))| {
                (
                    name,
                    StratumMetrics {
                        evaluated: rows,
                        recommended,
                        exact: correct,
                        exact_percent: percentage(correct, recommended),
                    },
                )
            })
            .collect()
    };

    EvaluationSummary {
        evaluated,
        skipped,
        canonical_label_counts: labels,
        accuracy: AccuracyMetrics {
            exact: rate(exact, recommendations),
            same_family: rate(same_family, recommendations),
        },
        abstention: rate(abstentions, evaluated),
        manual_review: rate(manual_review, evaluated),
        confidence_precision,
        confusion_pairs,
        by_source_stratum: to_strata(sources),
        by_discogs_match_quality: to_strata(qualities),
        by_classification_mode: to_strata(modes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::classification::{ClassificationAction, ClassificationMode};

    fn result(
        genre: Option<&'static str>,
        confidence: ClassificationConfidence,
    ) -> ClassificationResult {
        ClassificationResult {
            track_id: String::new(),
            artist: String::new(),
            title: String::new(),
            current_genre: String::new(),
            genre,
            confidence,
            action: ClassificationAction::Suggest,
            mode: ClassificationMode::Full,
            degraded_reasons: Vec::new(),
            evidence: Vec::new(),
            candidates: Vec::new(),
            flags: Vec::new(),
            review_hint: None,
        }
    }

    #[test]
    fn metrics_keep_recommendation_and_evaluated_denominators_distinct() {
        let exact = result(Some("Techno"), ClassificationConfidence::High);
        let family = result(Some("Deep Techno"), ClassificationConfidence::Low);
        let abstain = result(None, ClassificationConfidence::Insufficient);
        let cases = [
            EvaluationCase {
                truth: "Techno",
                result: &exact,
                source_stratum: "discogs+audio",
                discogs_match_quality: "exact",
            },
            EvaluationCase {
                truth: "Techno",
                result: &family,
                source_stratum: "discogs",
                discogs_match_quality: "fuzzy",
            },
            EvaluationCase {
                truth: "House",
                result: &abstain,
                source_stratum: "none",
                discogs_match_quality: "not_searched",
            },
        ];

        let summary = evaluate(&cases, 2);
        assert_eq!(summary.evaluated, 3);
        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.accuracy.exact.denominator, 2);
        assert_eq!(summary.accuracy.exact.count, 1);
        assert_eq!(summary.accuracy.same_family.count, 2);
        assert_eq!(summary.abstention.count, 1);
        assert_eq!(summary.abstention.denominator, 3);
        assert_eq!(summary.manual_review.count, 2);
        assert_eq!(summary.confidence_precision["high"].exact_percent, 100.0);
        assert_eq!(summary.confusion_pairs[0].recommended, "Deep Techno");
        assert_eq!(summary.by_classification_mode["full"].evaluated, 3);
    }

    #[test]
    fn zero_rows_are_finite_and_zero() {
        let summary = evaluate(&[], 4);
        assert_eq!(summary.evaluated, 0);
        assert_eq!(summary.skipped, 4);
        assert_eq!(summary.accuracy.exact.percent, 0.0);
        assert_eq!(summary.abstention.percent, 0.0);
        assert!(summary.confusion_pairs.is_empty());
    }

    #[test]
    fn evaluation_stratifies_mode_and_uses_domain_review_policy() {
        let mut degraded = result(Some("Techno"), ClassificationConfidence::High);
        degraded.mode = ClassificationMode::Degraded;
        let cases = [EvaluationCase {
            truth: "Techno",
            result: &degraded,
            source_stratum: "discogs",
            discogs_match_quality: "exact",
        }];

        let summary = evaluate(&cases, 0);
        assert_eq!(summary.manual_review.count, 1);
        assert_eq!(summary.by_classification_mode["degraded"].evaluated, 1);
        assert_eq!(summary.by_classification_mode["degraded"].exact, 1);
    }
}
