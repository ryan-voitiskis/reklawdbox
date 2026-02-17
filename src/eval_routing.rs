#[cfg(test)]
mod tests {
    use crate::corpus::{CorpusIndex, CorpusQuery};

    const TOP_K: usize = 3;
    const TOP1_ACCURACY_THRESHOLD: f64 = 0.90;
    const SCORE_PASS_RATE_THRESHOLD: f64 = 0.80;
    const TOP1_SCORE_THRESHOLD: i32 = 150;

    #[derive(Debug, Clone, Copy)]
    struct RoutingCase {
        prompt: &'static str,
        topic: Option<&'static str>,
        mode: Option<&'static str>,
        doc_type: Option<&'static str>,
        search_text: &'static str,
        expected_id: &'static str,
        expected_path: &'static str,
        min_top_score: i32,
    }

    #[derive(Debug)]
    struct CaseResult {
        case: RoutingCase,
        top_hit: Option<(String, String, i32)>,
        top_candidates: Vec<String>,
        top1_match: bool,
        score_ok: bool,
    }

    const ROUTING_CASES: &[RoutingCase] = &[
        RoutingCase {
            prompt: "How do I import and export rekordbox XML with the canonical reference format?",
            topic: Some("xml"),
            mode: Some("export"),
            doc_type: Some("reference"),
            search_text: "xml import export",
            expected_id: "xml-import-export",
            expected_path: "docs/rekordbox/reference/xml-import-export.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "Where is the XML format list used by rekordbox import workflows?",
            topic: Some("xml"),
            mode: Some("common"),
            doc_type: Some("guide"),
            search_text: "xml format list",
            expected_id: "xml-format-spec",
            expected_path: "docs/rekordbox/guides/xml-format-spec.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "I need the USB export workflow for playlists and devices.",
            topic: Some("usb"),
            mode: Some("export"),
            doc_type: Some("guide"),
            search_text: "usb export playlists",
            expected_id: "usb-export",
            expected_path: "docs/rekordbox/guides/usb-export.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "Show me the setup guide for PRO DJ LINK with connected equipment.",
            topic: Some("pro-dj-link"),
            mode: Some("export"),
            doc_type: Some("guide"),
            search_text: "setup link equipment",
            expected_id: "pro-dj-link-setup",
            expected_path: "docs/rekordbox/guides/pro-dj-link-setup.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "I want DVS setup instructions for performance mode with compatibility checks.",
            topic: Some("dvs"),
            mode: Some("performance"),
            doc_type: Some("guide"),
            search_text: "dvs setup compatibility",
            expected_id: "dvs-setup",
            expected_path: "docs/rekordbox/guides/dvs-setup.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "How do I configure lighting mode and MIDI controls?",
            topic: Some("lighting"),
            mode: Some("lighting"),
            doc_type: Some("guide"),
            search_text: "lighting mode midi",
            expected_id: "lighting-mode",
            expected_path: "docs/rekordbox/guides/lighting-mode.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "Which guide covers streaming service usage in rekordbox?",
            topic: Some("streaming"),
            mode: Some("common"),
            doc_type: Some("guide"),
            search_text: "streaming service usage",
            expected_id: "streaming-services",
            expected_path: "docs/rekordbox/guides/streaming-services.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "Where is the keyboard shortcut reference for performance browsing?",
            topic: Some("browsing"),
            mode: Some("performance"),
            doc_type: Some("guide"),
            search_text: "keyboard shortcut reference",
            expected_id: "keyboard-shortcuts",
            expected_path: "docs/rekordbox/guides/keyboard-shortcuts.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "Open the preferences window manual section for performance mode.",
            topic: Some("preferences"),
            mode: Some("performance"),
            doc_type: Some("manual"),
            search_text: "preferences window",
            expected_id: "preferences",
            expected_path: "docs/rekordbox/manual/31-preferences.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
        RoutingCase {
            prompt: "I need the manual chapter for collaborative playlists.",
            topic: Some("collaborative-playlists"),
            mode: Some("common"),
            doc_type: Some("manual"),
            search_text: "collaborative playlist",
            expected_id: "collaborative-playlists",
            expected_path: "docs/rekordbox/manual/09-collaborative-playlists.md",
            min_top_score: TOP1_SCORE_THRESHOLD,
        },
    ];

    #[test]
    fn corpus_document_routing_meets_quality_thresholds() {
        let index = CorpusIndex::load_rekordbox().expect("manifest index should load");
        assert!(!ROUTING_CASES.is_empty(), "routing eval must include cases");

        assert_cases_reference_manifest_entries(&index, ROUTING_CASES);

        let results = ROUTING_CASES
            .iter()
            .map(|case| evaluate_case(&index, *case))
            .collect::<Vec<_>>();

        let total = results.len();
        let top1_hits = results.iter().filter(|result| result.top1_match).count();
        let score_pass_hits = results.iter().filter(|result| result.score_ok).count();

        let min_top1_hits = ((total as f64) * TOP1_ACCURACY_THRESHOLD).ceil() as usize;
        let min_score_hits = ((total as f64) * SCORE_PASS_RATE_THRESHOLD).ceil() as usize;

        let report = build_report(
            &results,
            total,
            top1_hits,
            min_top1_hits,
            score_pass_hits,
            min_score_hits,
        );
        println!("{report}");

        assert!(
            top1_hits >= min_top1_hits,
            "routing top-1 threshold failed\n{report}"
        );
        assert!(
            score_pass_hits >= min_score_hits,
            "routing score threshold failed\n{report}"
        );
    }

    fn assert_cases_reference_manifest_entries(index: &CorpusIndex, cases: &[RoutingCase]) {
        for case in cases {
            let expected_manifest_path = case
                .expected_path
                .strip_prefix("docs/rekordbox/")
                .expect("expected paths must be repo-relative docs/rekordbox paths");

            let manifest_doc = index
                .manifest()
                .documents
                .iter()
                .find(|doc| doc.id == case.expected_id)
                .unwrap_or_else(|| {
                    panic!(
                        "expected document id missing in manifest: {}",
                        case.expected_id
                    )
                });

            assert_eq!(
                manifest_doc.path, expected_manifest_path,
                "manifest path drift for expected id {}",
                case.expected_id
            );
        }
    }

    fn evaluate_case(index: &CorpusIndex, case: RoutingCase) -> CaseResult {
        let hits = index.query(CorpusQuery {
            topic: case.topic,
            mode: case.mode,
            doc_type: case.doc_type,
            search_text: Some(case.search_text),
            limit: Some(TOP_K),
        });

        let top_candidates = hits
            .iter()
            .map(|hit| format!("{}:{}@{}", hit.id, hit.score, hit.path))
            .collect::<Vec<_>>();

        let top_hit = hits
            .first()
            .map(|hit| (hit.id.clone(), hit.path.clone(), hit.score));
        let top1_match = matches!(
            top_hit.as_ref(),
            Some((id, path, _)) if id == case.expected_id && path == case.expected_path
        );
        let score_ok =
            matches!(top_hit.as_ref(), Some((_, _, score)) if *score >= case.min_top_score);

        CaseResult {
            case,
            top_hit,
            top_candidates,
            top1_match,
            score_ok,
        }
    }

    fn build_report(
        results: &[CaseResult],
        total: usize,
        top1_hits: usize,
        min_top1_hits: usize,
        score_pass_hits: usize,
        min_score_hits: usize,
    ) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "Routing Eval Summary: top1={top1_hits}/{total} (threshold >= {min_top1_hits}), score={score_pass_hits}/{total} (threshold >= {min_score_hits}), score_floor={TOP1_SCORE_THRESHOLD}"
        ));

        for (idx, result) in results.iter().enumerate() {
            let top = result
                .top_hit
                .as_ref()
                .map(|(id, path, score)| format!("{id}:{score}@{path}"))
                .unwrap_or_else(|| "<none>".to_string());

            lines.push(format!(
                "case {}: top1_match={} score_ok={} expected={}:{} top={} prompt=\"{}\" candidates=[{}]",
                idx + 1,
                result.top1_match,
                result.score_ok,
                result.case.expected_id,
                result.case.expected_path,
                top,
                result.case.prompt,
                result.top_candidates.join(", ")
            ));
        }

        lines.join("\n")
    }
}
