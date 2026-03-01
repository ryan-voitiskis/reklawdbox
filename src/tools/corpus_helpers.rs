use std::collections::HashSet;

use crate::corpus;

pub(super) struct CorpusConsultation {
    pub(super) consulted_documents: Vec<String>,
    pub(super) manifest_status: String,
    pub(super) warning: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) struct CorpusQuerySpec {
    pub(super) topic: Option<&'static str>,
    pub(super) mode: Option<&'static str>,
    pub(super) doc_type: Option<&'static str>,
    pub(super) search_text: Option<&'static str>,
    pub(super) limit: usize,
}

pub(super) fn unique_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut deduped_paths = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            deduped_paths.push(path);
        }
    }
    deduped_paths
}

pub(super) fn fallback_corpus_consultation(
    fallback_paths: &[&str],
    manifest_status: &str,
    warning: Option<String>,
) -> CorpusConsultation {
    CorpusConsultation {
        consulted_documents: unique_paths(fallback_paths.iter().map(|p| (*p).to_string())),
        manifest_status: manifest_status.to_string(),
        warning,
    }
}

pub(super) fn consult_manifest_first_docs(
    query_specs: &[CorpusQuerySpec],
    fallback_paths: &[&str],
) -> CorpusConsultation {
    match corpus::rekordbox_index() {
        Ok(index) => {
            let mut paths = Vec::new();
            for query_spec in query_specs {
                let query = corpus::CorpusQuery {
                    topic: query_spec.topic,
                    mode: query_spec.mode,
                    doc_type: query_spec.doc_type,
                    search_text: query_spec.search_text,
                    limit: Some(query_spec.limit),
                };
                paths.extend(index.matched_paths(query));
            }

            let paths = unique_paths(paths);
            if paths.is_empty() {
                return fallback_corpus_consultation(
                    fallback_paths,
                    "empty",
                    Some(
                        "Corpus retrieval returned no matching documents; used fallback references."
                            .to_string(),
                    ),
                );
            }

            CorpusConsultation {
                consulted_documents: paths,
                manifest_status: "ok".to_string(),
                warning: None,
            }
        }
        Err(e) => fallback_corpus_consultation(
            fallback_paths,
            "unavailable",
            Some(format!(
                "Corpus retrieval failed; used fallback references: {e}"
            )),
        ),
    }
}

pub(super) fn consult_xml_workflow_docs() -> CorpusConsultation {
    consult_manifest_first_docs(
        &[
            CorpusQuerySpec {
                topic: Some("xml"),
                mode: Some("export"),
                doc_type: Some("reference"),
                search_text: Some("xml import export"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("xml"),
                mode: Some("common"),
                doc_type: Some("guide"),
                search_text: Some("xml format"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("xml"),
                mode: Some("common"),
                doc_type: Some("reference"),
                search_text: Some("developer integration"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("library"),
                mode: Some("common"),
                doc_type: Some("faq"),
                search_text: Some("xml"),
                limit: 2,
            },
        ],
        &[
            "docs/rekordbox/reference/xml-import-export.md",
            "docs/rekordbox/guides/xml-format-spec.md",
            "docs/rekordbox/reference/developer-integration.md",
            "docs/rekordbox/manual/31-preferences.md",
            "docs/rekordbox/faq/library-and-collection.md",
        ],
    )
}

pub(super) fn consult_genre_workflow_docs() -> CorpusConsultation {
    consult_manifest_first_docs(
        &[
            CorpusQuerySpec {
                topic: Some("genre"),
                mode: Some("common"),
                doc_type: Some("manual"),
                search_text: Some("genre"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("metadata"),
                mode: Some("common"),
                doc_type: Some("reference"),
                search_text: Some("genre metadata"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("library"),
                mode: Some("common"),
                doc_type: Some("faq"),
                search_text: Some("genre"),
                limit: 3,
            },
            CorpusQuerySpec {
                topic: Some("collection"),
                mode: Some("common"),
                doc_type: Some("manual"),
                search_text: Some("search genre"),
                limit: 3,
            },
        ],
        &[
            "docs/rekordbox/manual/06-searching.md",
            "docs/rekordbox/faq/library-and-collection.md",
            "docs/rekordbox/reference/glossary.md",
            "docs/rekordbox/reference/developer-integration.md",
        ],
    )
}

pub(super) fn attach_corpus_provenance(
    result: &mut serde_json::Value,
    consultation: CorpusConsultation,
) {
    result["consulted_documents"] = serde_json::json!(consultation.consulted_documents);
    result["manifest_status"] = serde_json::json!(consultation.manifest_status);
    if let Some(warning) = consultation.warning {
        result["corpus_warning"] = serde_json::json!(warning);
    }
}
