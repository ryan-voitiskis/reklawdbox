#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::corpus::{self, CorpusHit, CorpusIndex, CorpusQuery};

    #[derive(Clone, Copy, Debug)]
    struct QuerySpec {
        topic: Option<&'static str>,
        mode: Option<&'static str>,
        doc_type: Option<&'static str>,
        search_text: Option<&'static str>,
        limit: usize,
    }

    #[derive(Clone, Copy, Debug)]
    struct Thresholds {
        min_total_hits: usize,
        min_expected_recall: f32,
        min_prefix_matches: usize,
    }

    #[derive(Clone, Copy, Debug)]
    struct WorkflowEvalCase {
        name: &'static str,
        query_specs: &'static [QuerySpec],
        expected_doc_ids: &'static [&'static str],
        expected_prefix_doc_ids: &'static [&'static str],
        thresholds: Thresholds,
    }

    #[derive(Debug)]
    struct EvalReport {
        workflow: &'static str,
        deterministic: bool,
        total_hits: usize,
        min_total_hits: usize,
        matched_expected: usize,
        expected_total: usize,
        expected_recall: f32,
        min_expected_recall: f32,
        prefix_matches: usize,
        min_prefix_matches: usize,
        observed_paths: Vec<String>,
        expected_paths: Vec<String>,
        expected_prefix_paths: Vec<String>,
    }

    impl EvalReport {
        fn passed(&self) -> bool {
            self.deterministic
                && self.total_hits >= self.min_total_hits
                && self.expected_recall >= self.min_expected_recall
                && self.prefix_matches >= self.min_prefix_matches
        }

        fn render(&self) -> String {
            format!(
                "workflow: {workflow}\n\
                 deterministic: {deterministic}\n\
                 total_hits: {total_hits} (threshold: >= {min_total_hits})\n\
                 expected_recall: {matched_expected}/{expected_total} = {expected_recall:.3} (threshold: >= {min_expected_recall:.3})\n\
                 prefix_matches: {prefix_matches} (threshold: >= {min_prefix_matches})\n\
                 expected_paths: {expected_paths:?}\n\
                 expected_prefix_paths: {expected_prefix_paths:?}\n\
                 observed_paths: {observed_paths:?}",
                workflow = self.workflow,
                deterministic = self.deterministic,
                total_hits = self.total_hits,
                min_total_hits = self.min_total_hits,
                matched_expected = self.matched_expected,
                expected_total = self.expected_total,
                expected_recall = self.expected_recall,
                min_expected_recall = self.min_expected_recall,
                prefix_matches = self.prefix_matches,
                min_prefix_matches = self.min_prefix_matches,
                expected_paths = self.expected_paths,
                expected_prefix_paths = self.expected_prefix_paths,
                observed_paths = self.observed_paths,
            )
        }
    }

    fn run_manifest_first_retrieval(
        index: &CorpusIndex,
        query_specs: &[QuerySpec],
    ) -> Vec<CorpusHit> {
        let mut seen_paths = HashSet::new();
        let mut combined = Vec::new();
        for spec in query_specs {
            let query = CorpusQuery {
                topic: spec.topic,
                mode: spec.mode,
                doc_type: spec.doc_type,
                search_text: spec.search_text,
                limit: Some(spec.limit),
            };
            for hit in index.query(query) {
                if seen_paths.insert(hit.path.clone()) {
                    combined.push(hit);
                }
            }
        }
        combined
    }

    fn repo_path_for_doc_id(index: &CorpusIndex, doc_id: &str) -> String {
        index
            .manifest()
            .documents
            .iter()
            .find(|document| document.id == doc_id)
            .map(|document| format!("{}/{}", corpus::REKORDBOX_DOCS_ROOT, document.path))
            .unwrap_or_else(|| panic!("manifest is missing expected doc id '{doc_id}'"))
    }

    fn evaluate_case(index: &CorpusIndex, case: &WorkflowEvalCase) -> EvalReport {
        let first_run = run_manifest_first_retrieval(index, case.query_specs);
        let second_run = run_manifest_first_retrieval(index, case.query_specs);

        let first_paths: Vec<String> = first_run.iter().map(|hit| hit.path.clone()).collect();
        let second_paths: Vec<String> = second_run.iter().map(|hit| hit.path.clone()).collect();
        let deterministic = first_paths == second_paths;

        let expected_paths: Vec<String> = case
            .expected_doc_ids
            .iter()
            .map(|doc_id| repo_path_for_doc_id(index, doc_id))
            .collect();
        let expected_prefix_paths: Vec<String> = case
            .expected_prefix_doc_ids
            .iter()
            .map(|doc_id| repo_path_for_doc_id(index, doc_id))
            .collect();

        let observed_set: HashSet<&str> = first_paths.iter().map(String::as_str).collect();
        let matched_expected = expected_paths
            .iter()
            .filter(|path| observed_set.contains(path.as_str()))
            .count();
        let expected_total = expected_paths.len();
        let expected_recall = if expected_total == 0 {
            1.0
        } else {
            matched_expected as f32 / expected_total as f32
        };

        let prefix_matches = first_paths
            .iter()
            .zip(expected_prefix_paths.iter())
            .take_while(|(observed, expected)| *observed == *expected)
            .count();

        EvalReport {
            workflow: case.name,
            deterministic,
            total_hits: first_paths.len(),
            min_total_hits: case.thresholds.min_total_hits,
            matched_expected,
            expected_total,
            expected_recall,
            min_expected_recall: case.thresholds.min_expected_recall,
            prefix_matches,
            min_prefix_matches: case.thresholds.min_prefix_matches,
            observed_paths: first_paths,
            expected_paths,
            expected_prefix_paths,
        }
    }

    fn workflow_cases() -> &'static [WorkflowEvalCase] {
        &[
            WorkflowEvalCase {
                name: "ui-understanding",
                query_specs: &[
                    QuerySpec {
                        topic: Some("interface"),
                        mode: Some("common"),
                        doc_type: Some("manual"),
                        search_text: Some("introduction"),
                        limit: 2,
                    },
                    QuerySpec {
                        topic: Some("interface"),
                        mode: Some("common"),
                        doc_type: Some("manual"),
                        search_text: Some("window"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("interface"),
                        mode: Some("export"),
                        doc_type: Some("manual"),
                        search_text: Some("screen"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("interface"),
                        mode: Some("performance"),
                        doc_type: Some("manual"),
                        search_text: Some("window"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("interface"),
                        mode: Some("common"),
                        doc_type: Some("reference"),
                        search_text: Some("glossary interface"),
                        limit: 2,
                    },
                ],
                expected_doc_ids: &[
                    "introduction",
                    "collection-window",
                    "export-mode-screen",
                    "info-window",
                    "preferences",
                    "glossary",
                ],
                expected_prefix_doc_ids: &[
                    "introduction",
                    "collection-window",
                    "export-mode-screen",
                ],
                thresholds: Thresholds {
                    min_total_hits: 5,
                    min_expected_recall: 0.80,
                    min_prefix_matches: 3,
                },
            },
            WorkflowEvalCase {
                name: "xml-import-export",
                query_specs: &[
                    QuerySpec {
                        topic: Some("xml"),
                        mode: Some("export"),
                        doc_type: Some("reference"),
                        search_text: Some("xml import export"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("xml"),
                        mode: Some("common"),
                        doc_type: Some("guide"),
                        search_text: Some("xml format"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("xml"),
                        mode: Some("common"),
                        doc_type: Some("reference"),
                        search_text: Some("developer integration"),
                        limit: 3,
                    },
                ],
                expected_doc_ids: &[
                    "xml-import-export",
                    "xml-format-spec",
                    "developer-integration",
                ],
                expected_prefix_doc_ids: &[
                    "xml-import-export",
                    "xml-format-spec",
                    "developer-integration",
                ],
                thresholds: Thresholds {
                    min_total_hits: 3,
                    min_expected_recall: 1.0,
                    min_prefix_matches: 3,
                },
            },
            WorkflowEvalCase {
                name: "library-management",
                query_specs: &[
                    QuerySpec {
                        topic: Some("library"),
                        mode: Some("common"),
                        doc_type: Some("manual"),
                        search_text: Some("management collection"),
                        limit: 4,
                    },
                    QuerySpec {
                        topic: Some("library"),
                        mode: Some("common"),
                        doc_type: Some("faq"),
                        search_text: Some("library collection"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("library"),
                        mode: Some("common"),
                        doc_type: Some("reference"),
                        search_text: Some("library metadata"),
                        limit: 3,
                    },
                ],
                expected_doc_ids: &[
                    "management",
                    "faq-library-and-collection",
                    "developer-integration",
                    "glossary",
                ],
                expected_prefix_doc_ids: &[
                    "management",
                    "faq-library-and-collection",
                    "developer-integration",
                ],
                thresholds: Thresholds {
                    min_total_hits: 4,
                    min_expected_recall: 1.0,
                    min_prefix_matches: 3,
                },
            },
            WorkflowEvalCase {
                name: "usb-export",
                query_specs: &[
                    QuerySpec {
                        topic: Some("usb"),
                        mode: Some("export"),
                        doc_type: Some("guide"),
                        search_text: Some("usb export"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("usb"),
                        mode: Some("export"),
                        doc_type: Some("manual"),
                        search_text: Some("usb export"),
                        limit: 4,
                    },
                    QuerySpec {
                        topic: Some("devices"),
                        mode: Some("common"),
                        doc_type: Some("faq"),
                        search_text: Some("usb devices"),
                        limit: 3,
                    },
                ],
                expected_doc_ids: &[
                    "usb-export",
                    "device-library-backup",
                    "export-pro-dj-link",
                    "faq-usb-and-devices",
                ],
                expected_prefix_doc_ids: &["usb-export", "device-library-backup"],
                thresholds: Thresholds {
                    min_total_hits: 6,
                    min_expected_recall: 1.0,
                    min_prefix_matches: 2,
                },
            },
            WorkflowEvalCase {
                name: "preferences-settings",
                query_specs: &[
                    QuerySpec {
                        topic: Some("preferences"),
                        mode: Some("export"),
                        doc_type: Some("manual"),
                        search_text: Some("preferences"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("preferences"),
                        mode: Some("performance"),
                        doc_type: Some("manual"),
                        search_text: Some("preferences"),
                        limit: 3,
                    },
                    QuerySpec {
                        topic: Some("preferences"),
                        mode: Some("export"),
                        doc_type: Some("manual"),
                        search_text: Some("menu"),
                        limit: 2,
                    },
                ],
                expected_doc_ids: &["preferences", "menu-list"],
                expected_prefix_doc_ids: &["preferences", "menu-list"],
                thresholds: Thresholds {
                    min_total_hits: 2,
                    min_expected_recall: 1.0,
                    min_prefix_matches: 2,
                },
            },
        ]
    }

    #[test]
    fn task_success_eval_manifest_first_workflows() {
        let index = corpus::rekordbox_index().expect("rekordbox manifest index should load");

        for case in workflow_cases() {
            let report = evaluate_case(index, case);
            assert!(
                report.passed(),
                "task-success eval failed\n{}",
                report.render()
            );
        }
    }
}
