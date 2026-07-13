//! CLI-facing analysis cache protocol and compatibility vocabulary.

pub(crate) use crate::application::analysis::batch::{
    BatchFailure as CliBatchFailure, CacheWriteRequest, CacheWriterReport,
    MAX_CONSECUTIVE_CACHE_WRITE_FAILURES, batch_outcome as cli_batch_outcome,
    persist_analysis_cache_write as persist_cli_cache_message, send_cache_message,
    serialize_cache_payload, task_join_error_summary,
};
#[cfg(test)]
pub(crate) use crate::application::analysis::identity::{
    CacheProbe, file_mtime_unix, is_cache_fresh,
};
pub(crate) use crate::application::analysis::identity::{
    cache_probe_for_path, cache_status_for_track,
};
pub(crate) use crate::application::analysis::model::AnalysisCacheWrite as CliCacheWriteMsg;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::runtime::test_support::{TEST_WATCHDOG, TaskGuard, bounded};
    use crate::{audio, store, store::CachedAudioAnalysis};
    use serde::ser::Error as _;

    fn cached(file_size: i64, file_mtime: i64) -> CachedAudioAnalysis {
        CachedAudioAnalysis {
            file_path: "/tmp/a.flac".to_string(),
            analyzer: "stratum-dsp".to_string(),
            file_size,
            file_mtime,
            analysis_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
            input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
            features_json: "{}".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn cache_message() -> CliCacheWriteMsg {
        CliCacheWriteMsg {
            file_path: "/tmp/test.flac".to_string(),
            analyzer: "stratum-dsp".to_string(),
            file_size: 1,
            file_mtime: 2,
            analyzer_version: "v1".to_string(),
            input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
            features_json: "{}".to_string(),
        }
    }

    struct FailingSerialize;

    impl serde::Serialize for FailingSerialize {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(S::Error::custom("boom"))
        }
    }

    fn open_temp_store_with_probe() -> (tempfile::TempDir, rusqlite::Connection, CacheProbe) {
        let dir = tempfile::tempdir().expect("temp dir");

        let audio_path = dir.path().join("track.wav");
        std::fs::write(&audio_path, b"not-a-real-audio-file").expect("write audio fixture");

        let metadata = std::fs::metadata(&audio_path).expect("metadata");
        let cache_key = audio_path.to_string_lossy().to_string();
        let file_size = metadata.len() as i64;
        let file_mtime = file_mtime_unix(&metadata);
        let probe = CacheProbe {
            cache_key,
            file_size,
            file_mtime,
            stratum_input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
        };

        let store_path = dir.path().join("cache.sqlite3");
        let conn = store::open(store_path.to_str().expect("utf-8 path")).expect("open store");
        (dir, conn, probe)
    }

    #[test]
    fn cache_is_fresh_only_when_version_and_file_identity_match() {
        let entry = cached(123, 456);
        let v = audio::STRATUM_SCHEMA_VERSION;
        assert!(is_cache_fresh(Some(&entry), v, 123, 456, "hmm:v1"));
        assert!(!is_cache_fresh(
            Some(&entry),
            "outdated",
            123,
            456,
            "hmm:v1"
        ));
        assert!(
            !is_cache_fresh(Some(&entry), v, 999, 456, "hmm:v1"),
            "mismatched file size must be stale"
        );
        assert!(
            !is_cache_fresh(Some(&entry), v, 123, 999, "hmm:v1"),
            "mismatched file mtime must be stale"
        );
    }

    #[test]
    fn missing_cache_is_not_fresh() {
        assert!(!is_cache_fresh(
            None,
            audio::STRATUM_SCHEMA_VERSION,
            123,
            456,
            "hmm:v1"
        ));
    }

    #[test]
    fn file_mtime_unix_returns_non_negative_timestamp_for_real_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("x.txt");
        std::fs::write(&path, "a").expect("write");
        let metadata = std::fs::metadata(path).expect("metadata");
        assert!(file_mtime_unix(&metadata) >= 0);
    }

    #[test]
    fn cache_status_skips_track_when_both_fresh_entries_exist() {
        let (_dir, conn, probe) = open_temp_store_with_probe();

        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "stratum-dsp",
            probe.file_size,
            probe.file_mtime,
            audio::STRATUM_SCHEMA_VERSION,
            &probe.stratum_input_fingerprint,
            "{}",
        )
        .expect("set stratum");
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "essentia",
            probe.file_size,
            probe.file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .expect("set essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).expect("cache status");
        assert!(has_stratum);
        assert!(has_essentia);
    }

    #[test]
    fn cache_status_detects_outdated_schema_version() {
        let (_dir, conn, probe) = open_temp_store_with_probe();

        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "stratum-dsp",
            probe.file_size,
            probe.file_mtime,
            "outdated",
            &probe.stratum_input_fingerprint,
            "{}",
        )
        .expect("set stale stratum");
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "essentia",
            probe.file_size,
            probe.file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .expect("set fresh essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).expect("cache status");
        assert!(!has_stratum, "outdated stratum cache must be re-analyzed");
        assert!(has_essentia, "fresh essentia cache should still be skipped");
    }

    #[test]
    fn cache_status_detects_stale_file_identity() {
        let (_dir, conn, probe) = open_temp_store_with_probe();

        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "stratum-dsp",
            probe.file_size + 1,
            probe.file_mtime,
            audio::STRATUM_SCHEMA_VERSION,
            &probe.stratum_input_fingerprint,
            "{}",
        )
        .expect("set stale-size stratum");
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            "essentia",
            probe.file_size,
            probe.file_mtime + 1,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .expect("set stale-mtime essentia");

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).expect("cache status");
        assert!(!has_stratum, "stale stratum cache size must be re-analyzed");
        assert!(
            !has_essentia,
            "stale essentia cache mtime must be re-analyzed"
        );
    }

    #[test]
    fn cache_status_grid_change_marks_only_stratum_pending_then_write_restores_freshness() {
        let (_dir, conn, mut probe) = open_temp_store_with_probe();
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            audio::ANALYZER_STRATUM,
            probe.file_size,
            probe.file_mtime,
            audio::STRATUM_SCHEMA_VERSION,
            "grid:v1:before",
            "{}",
        )
        .unwrap();
        store::set_audio_analysis_with_fingerprint(
            &conn,
            &probe.cache_key,
            audio::ANALYZER_ESSENTIA,
            probe.file_size,
            probe.file_mtime,
            audio::ESSENTIA_SCHEMA_VERSION,
            "",
            "{}",
        )
        .unwrap();
        probe.stratum_input_fingerprint = "grid:v1:after".to_string();

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).unwrap();
        assert!(!has_stratum);
        assert!(has_essentia);

        persist_cli_cache_message(
            &conn,
            &CliCacheWriteMsg {
                file_path: probe.cache_key.clone(),
                analyzer: audio::ANALYZER_STRATUM.to_string(),
                file_size: probe.file_size,
                file_mtime: probe.file_mtime,
                analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
                input_fingerprint: probe.stratum_input_fingerprint.clone(),
                features_json: "{}".to_string(),
            },
        )
        .unwrap();

        let (has_stratum, has_essentia) =
            cache_status_for_track(&conn, Some(&probe), true, true).unwrap();
        assert!(has_stratum);
        assert!(has_essentia);
    }

    #[test]
    fn serialize_cache_payload_reports_errors() {
        let err = serialize_cache_payload(&FailingSerialize, "test payload")
            .expect_err("failing serializer should bubble up");
        assert!(err.contains("test payload cache serialization failed"));
        assert!(err.contains("boom"));
    }

    #[tokio::test]
    async fn send_cache_message_reports_closed_channel() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            drop(rx);
            let result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "closed-channel send",
            )
            .await
            .expect("bounded send");
            drop(tx);
            let err = result.expect_err("closed cache channel should surface an error");
            assert!(err.contains("analysis cache queue send failed"));
        })
        .await
        .expect("closed-channel test watchdog expired");
    }

    #[tokio::test]
    async fn send_cache_message_waits_for_success_acknowledgement() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            let writer = TaskGuard::new(tokio::spawn(async move {
                let request = bounded(rx.recv(), "success writer receive")
                    .await?
                    .ok_or_else(|| "success writer channel closed".to_string())?;
                request.acknowledgement.send(Ok(())).ok();
                Ok::<(), String>(())
            }));

            let send_result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "success acknowledged send",
            )
            .await;
            drop(tx);
            let writer_result = writer.join("success writer join").await;
            assert_eq!(send_result.expect("bounded send"), Ok(()));
            assert_eq!(writer_result.expect("guarded writer join"), Ok(()));
        })
        .await
        .expect("success-ack test watchdog expired");
    }

    #[tokio::test]
    async fn send_cache_message_reports_write_acknowledgement_error() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            let writer = TaskGuard::new(tokio::spawn(async move {
                let request = bounded(rx.recv(), "error writer receive")
                    .await?
                    .ok_or_else(|| "error writer channel closed".to_string())?;
                request
                    .acknowledgement
                    .send(Err("sqlite write rejected".to_string()))
                    .ok();
                Ok::<(), String>(())
            }));

            let send_result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "error acknowledged send",
            )
            .await;
            drop(tx);
            let writer_result = writer.join("error writer join").await;
            let err = send_result
                .expect("bounded send")
                .expect_err("write error must reach producer");
            assert_eq!(err, "sqlite write rejected");
            assert_eq!(writer_result.expect("guarded writer join"), Ok(()));
        })
        .await
        .expect("write-error test watchdog expired");
    }

    #[tokio::test]
    async fn send_cache_message_reports_canceled_acknowledgement() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(1);
            let writer = TaskGuard::new(tokio::spawn(async move {
                let request = bounded(rx.recv(), "canceled writer receive")
                    .await?
                    .ok_or_else(|| "canceled writer channel closed".to_string())?;
                drop(request.acknowledgement);
                Ok::<(), String>(())
            }));

            let send_result = bounded(
                send_cache_message(&tx, cache_message(), "analysis"),
                "canceled acknowledged send",
            )
            .await;
            drop(tx);
            let writer_result = writer.join("canceled writer join").await;
            let err = send_result
                .expect("bounded send")
                .expect_err("dropped acknowledgement must reach producer");
            assert!(err.contains("analysis cache acknowledgement canceled"));
            assert_eq!(writer_result.expect("guarded writer join"), Ok(()));
        })
        .await
        .expect("canceled-ack test watchdog expired");
    }

    #[test]
    fn cli_batch_outcome_accepts_only_complete_success() {
        assert!(cli_batch_outcome("test", 0, 0, 0, 0, false, vec![]).is_ok());
        for (failures, joins, writes, incomplete, cancelled) in [
            (1, 0, 0, 0, false),
            (0, 1, 0, 0, false),
            (0, 0, 1, 0, false),
            (0, 0, 0, 1, false),
            (0, 0, 0, 0, true),
            (1, 1, 1, 1, true),
        ] {
            assert!(
                cli_batch_outcome(
                    "test",
                    failures,
                    joins,
                    writes,
                    incomplete,
                    cancelled,
                    vec!["stable summary".to_string()],
                )
                .is_err()
            );
        }

        let combined =
            cli_batch_outcome("test", 1, 2, 3, 4, true, vec!["stable summary".to_string()])
                .expect_err("combined failures");
        assert_eq!(
            combined,
            super::CliBatchFailure {
                command: "test",
                track_or_provider_failures: 1,
                worker_join_failures: 2,
                writer_failures: 3,
                incomplete: 4,
                user_cancelled: true,
                error_summaries: vec!["stable summary".to_string()],
            }
        );
    }
}
