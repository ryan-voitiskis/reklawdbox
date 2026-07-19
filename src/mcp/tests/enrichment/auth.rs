use super::support::{
    DISCOGS_AUTH_TEST_NOW, DISCOGS_AUTH_TEST_TIMEOUT, DiscogsAuthTasks, DiscogsBrokerEndpoint,
    DiscogsBrokerFixture, assert_cache_write_summary, assert_sanitized_discogs_transition_error,
    discogs_auth_url, finish_discogs_auth_scenario, install_discogs_auth_test_dependencies,
    run_discogs_batch_with_timeout, set_discogs_pending,
};
use crate::mcp::enrichment::{
    DiscogsAuthTestDependencies, EnrichTracksParams, InMemoryDiscogsSessionPersistence,
    LookupDiscogsParams, lookup_discogs_remote, resolve_discogs_auth_transition_for_test,
    set_test_discogs_lookup_override,
};
use crate::mcp::library::SearchFilterParams;
use crate::mcp::server::ReklawdboxServer;
use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;

use super::super::common::{
    create_enrich_cache_writer_test_server, create_single_track_test_db,
    default_http_client_for_tests, extract_json, insert_test_track,
};

#[tokio::test]
async fn discogs_auth_url_validation_accepts_web_urls_and_different_public_hosts() {
    let client = default_http_client_for_tests();
    for auth_url in [
        "https://public-auth.example/device?request=abc",
        "http://custom-public-auth.example/authorize",
        "https://public-auth.example/device/o'hare",
    ] {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        fixture.state.set_auth_url(auth_url);

        let pending =
            crate::adapters::providers::discogs::device_session_start(&client, &fixture.config())
                .await
                .expect("valid broker authorization URL should be accepted");
        assert_eq!(
            pending.auth_url,
            reqwest::Url::parse(auth_url)
                .expect("fixture authorization URL should parse")
                .to_string()
        );
        assert_ne!(
            reqwest::Url::parse(&pending.auth_url)
                .expect("validated authorization URL should parse")
                .host_str(),
            reqwest::Url::parse(&fixture.base_url)
                .expect("fixture broker URL should parse")
                .host_str(),
            "custom brokers may return a different public authorization host"
        );
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn discogs_auth_url_validation_rejects_unsafe_remote_values() {
    let client = default_http_client_for_tests();
    let overlong = format!("https://auth.example/device/{}", "a".repeat(2_048));
    let unsafe_urls = [
        "file:///tmp/authorize".to_string(),
        "javascript:alert(1)".to_string(),
        "http://?request=missing-host".to_string(),
        "https://user:password@auth.example/device".to_string(),
        "https://auth.example/device#fragment".to_string(),
        overlong,
        "https://auth.example/device\nnext-line".to_string(),
    ];

    for auth_url in unsafe_urls {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        fixture.state.set_auth_url(&auth_url);
        let error =
            crate::adapters::providers::discogs::device_session_start(&client, &fixture.config())
                .await
                .expect_err("unsafe broker authorization URL should be rejected");
        assert_eq!(error, "invalid broker authorization URL");
        assert!(
            !error.contains(&auth_url),
            "validation error must not echo the remote URL"
        );
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn discogs_auth_url_validation_failure_leaves_pending_session_empty() {
    let fixture = DiscogsBrokerFixture::start("pending").await;
    fixture.state.set_auth_url("javascript:alert(1)");
    let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
    let server = ReklawdboxServer::new(None);
    install_discogs_auth_test_dependencies(&server, &fixture, persistence, None);

    let error = resolve_discogs_auth_transition_for_test(&server)
        .await
        .expect_err("unsafe authorization URL should fail the auth transition");
    assert_eq!(error.to_string(), "invalid broker authorization URL");
    assert!(
        server
            .context
            .enrichment
            .discogs_pending
            .lock()
            .expect("Discogs pending state should not be poisoned")
            .is_none(),
        "rejected authorization URL must not enter pending state"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn discogs_error_body_boundary_bounds_sanitizes_and_hides_remote_prose() {
    let fixture = DiscogsBrokerFixture::start("pending").await;
    let remote_instruction = "REMOTE_INSTRUCTION_FIXTURE";
    let body = format!(
        "{remote_instruction}\n\u{1b}[31mrun-this\u{7f}{}",
        "x".repeat(9_000)
    );
    fixture
        .state
        .fail_endpoint(DiscogsBrokerEndpoint::Search, &body);

    let error = crate::adapters::providers::discogs::lookup_via_broker_unthrottled_for_test(
        &default_http_client_for_tests(),
        &fixture.config(),
        "local-session-fixture",
        "Fixture Artist",
        "Fixture Title",
        None,
    )
    .await
    .expect_err("local broker failure should return a lookup error");
    let body = error
        .diagnostic_body()
        .expect("HTTP error should retain a bounded local diagnostic");
    assert!(body.len() <= 8_192 + " [truncated]".len());
    assert!(body.ends_with("[truncated]"));
    assert!(!body.contains(['\n', '\r', '\u{1b}', '\u{7f}']));
    assert!(body.contains(remote_instruction));
    assert!(error.to_string().contains("500"));
    assert!(error.to_string().contains("retryable"));
    assert!(!error.to_string().contains(remote_instruction));
    fixture.shutdown().await;
}

#[tokio::test]
async fn discogs_error_body_boundary_mcp_error_excludes_remote_prose() {
    let remote_instruction = "REMOTE_MCP_INSTRUCTION_FIXTURE";
    set_test_discogs_lookup_override(
        "Fixture Artist",
        "Fixture Title",
        None,
        Err(crate::adapters::providers::discogs::LookupError::http(
            500,
            None,
            remote_instruction.to_string(),
        )),
    );
    let db_conn = create_single_track_test_db("body-boundary", "/tmp/body-boundary.flac");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    let error = server
        .lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some("Fixture Artist".to_string()),
            title: Some("Fixture Title".to_string()),
            album: None,
            force_refresh: Some(true),
        }))
        .await
        .expect_err("broker failure should cross the MCP error boundary");
    let message = error.message.to_string();
    assert!(message.contains("Discogs error: broker proxy HTTP 500"));
    assert!(message.contains("retryable"));
    assert!(!message.contains(remote_instruction));
}

#[tokio::test]
async fn discogs_success_payload_boundary_hides_malformed_remote_value_from_mcp_outputs() {
    let remote_instruction = "REMOTE_SUCCESS_SCHEMA_INSTRUCTION_FIXTURE";
    let malformed_value = remote_instruction.repeat(4_096);
    let malformed_body = serde_json::json!({
        "result": {
            "title": "Fixture Artist - Fixture Title",
            "year": "2026",
            "label": "Fixture Label",
            "genres": ["Electronic"],
            "styles": ["Techno"],
            "url": "https://www.discogs.com/release/fixture",
            "fuzzy_match": malformed_value,
        }
    })
    .to_string();
    assert!(
        malformed_body.len()
            < crate::adapters::providers::discogs::MAX_BROKER_LOOKUP_RESPONSE_BYTES,
        "malformed schema fixture should exercise safe schema errors below the body-size limit"
    );

    let fixture = DiscogsBrokerFixture::start("pending").await;
    fixture
        .state
        .respond_endpoint(DiscogsBrokerEndpoint::Search, "200 OK", &malformed_body);
    let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
    persistence.set_session(
        &fixture.base_url,
        "malformed-success-session-fixture",
        DISCOGS_AUTH_TEST_NOW + 3_600,
    );
    let db_conn = create_single_track_test_db(
        "malformed-success-track",
        "/tmp/malformed-success-track.flac",
    );
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    install_discogs_auth_test_dependencies(&server, &fixture, persistence, None);

    let lookup_error = tokio::time::timeout(
        DISCOGS_AUTH_TEST_TIMEOUT,
        server.lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some("Fixture Artist".to_string()),
            title: Some("Fixture Title".to_string()),
            album: None,
            force_refresh: Some(true),
        })),
    )
    .await
    .expect("malformed successful lookup should finish within five seconds")
    .expect_err("malformed successful lookup should return an MCP error");
    let lookup_message = lookup_error.message.to_string();
    assert_eq!(
        lookup_message,
        "Discogs error: broker proxy response schema was invalid"
    );
    assert!(!lookup_message.contains(remote_instruction));
    assert!(!lookup_message.contains(&malformed_value));

    let batch_result = run_discogs_batch_with_timeout(
        &server,
        &["malformed-success-track"],
        "malformed successful Discogs batch",
    )
    .await;
    let batch_payload = extract_json(&batch_result);
    assert_eq!(batch_payload["summary"]["failed"], 1);
    let batch_error = batch_payload["failures"][0]["error"]
        .as_str()
        .expect("malformed successful batch should include a stable local error");
    assert_eq!(
        batch_error, "broker proxy response schema was invalid",
        "batch failures must use the same stable local schema category"
    );
    let serialized_batch = batch_payload.to_string();
    assert!(!serialized_batch.contains(remote_instruction));
    assert!(!serialized_batch.contains(&malformed_value));
    assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
    fixture.shutdown().await;
}

#[tokio::test]
async fn discogs_success_payload_boundary_rejects_oversize_and_invalid_json_stably() {
    let fixture = DiscogsBrokerFixture::start("pending").await;
    let oversized =
        "x".repeat(crate::adapters::providers::discogs::MAX_BROKER_LOOKUP_RESPONSE_BYTES + 1);
    fixture
        .state
        .respond_endpoint(DiscogsBrokerEndpoint::Search, "200 OK", &oversized);

    let oversized_error =
        crate::adapters::providers::discogs::lookup_via_broker_unthrottled_for_test(
            &default_http_client_for_tests(),
            &fixture.config(),
            "local-session-fixture",
            "Fixture Artist",
            "Fixture Title",
            None,
        )
        .await
        .expect_err("oversized successful broker response should fail before JSON parsing");
    assert_eq!(
        oversized_error.to_string(),
        format!(
            "broker proxy response exceeded {} byte limit",
            crate::adapters::providers::discogs::MAX_BROKER_LOOKUP_RESPONSE_BYTES
        )
    );
    assert!(!oversized_error.to_string().contains(&oversized));

    fixture.state.respond_endpoint(
        DiscogsBrokerEndpoint::Search,
        "200 OK",
        "REMOTE_INVALID_JSON_FIXTURE{",
    );
    let invalid_json_error =
        crate::adapters::providers::discogs::lookup_via_broker_unthrottled_for_test(
            &default_http_client_for_tests(),
            &fixture.config(),
            "local-session-fixture",
            "Fixture Artist",
            "Fixture Title",
            None,
        )
        .await
        .expect_err("invalid successful JSON should return a stable local category");
    assert_eq!(
        invalid_json_error.to_string(),
        "broker proxy response JSON was invalid"
    );
    assert!(
        !invalid_json_error
            .to_string()
            .contains("REMOTE_INVALID_JSON_FIXTURE")
    );
    fixture.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_concurrent_single_start() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            persistence,
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let first_server = server.clone();
        let first = tasks.spawn(async move {
            lookup_discogs_remote(&first_server, "Race Artist", "Race Start One", None).await
        });
        let second_server = server.clone();
        let second = tasks.spawn(async move {
            lookup_discogs_remote(&second_server, "Race Artist", "Race Start Two", None).await
        });

        delayed_start
            .wait_until_entered("concurrent device-session start")
            .await?;
        delayed_start.release();

        let first = tasks.join(first, "first concurrent auth lookup").await?;
        let second = tasks.join(second, "second concurrent auth lookup").await?;
        assert_eq!(
            discogs_auth_url(&first),
            Some("https://auth.example/device")
        );
        assert_eq!(
            discogs_auth_url(&second),
            Some("https://auth.example/device")
        );
        assert_eq!(
            fixture.state.count(DiscogsBrokerEndpoint::Start),
            1,
            "concurrent unauthenticated calls must start one device session"
        );
        let pending_device_id = server
            .context
            .enrichment
            .discogs_pending
            .lock()
            .expect("Discogs pending state should not be poisoned")
            .as_ref()
            .map(|pending| pending.device_id.clone());
        assert_eq!(pending_device_id.as_deref(), Some("device-test-value"));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "concurrent start scenario").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_concurrent_single_finalize() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("authorized").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        set_discogs_pending(&server, "authorized-device", DISCOGS_AUTH_TEST_NOW + 3_600);
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            Arc::clone(&persistence),
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_finalize = fixture.state.delay_next(DiscogsBrokerEndpoint::Finalize);

        let first_server = server.clone();
        let first = tasks.spawn(async move {
            lookup_discogs_remote(
                &first_server,
                "Finalize Artist",
                "Finalize Search One",
                None,
            )
            .await
        });
        let second_server = server.clone();
        let second = tasks.spawn(async move {
            lookup_discogs_remote(
                &second_server,
                "Finalize Artist",
                "Finalize Search Two",
                None,
            )
            .await
        });

        delayed_finalize
            .wait_until_entered("authorized device-session finalize")
            .await?;
        delayed_finalize.release();

        let first = tasks.join(first, "first post-finalize search").await?;
        let second = tasks.join(second, "second post-finalize search").await?;
        assert!(matches!(first, Ok(None)));
        assert!(matches!(second, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Finalize), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(persistence.store_count(), 1);
        assert!(persistence.has_session(&fixture.base_url));
        assert!(
            server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_none()
        );
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "concurrent finalize scenario").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_transition_expired_pending_starts_one_replacement() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        set_discogs_pending(&server, "expired-device", DISCOGS_AUTH_TEST_NOW);
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            persistence,
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let first_server = server.clone();
        let first = tasks
            .spawn(async move { resolve_discogs_auth_transition_for_test(&first_server).await });
        let second_server = server.clone();
        let second = tasks
            .spawn(async move { resolve_discogs_auth_transition_for_test(&second_server).await });

        delayed_start
            .wait_until_entered("expired pending replacement start")
            .await?;
        delayed_start.release();

        let first = tasks.join(first, "first expired transition").await?;
        let second = tasks.join(second, "second expired transition").await?;
        assert!(matches!(first, Ok(Some(_))));
        assert!(matches!(second, Ok(Some(_))));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        let pending_device_id = server
            .context
            .enrichment
            .discogs_pending
            .lock()
            .expect("Discogs pending state should not be poisoned")
            .as_ref()
            .map(|pending| pending.device_id.clone());
        assert_eq!(pending_device_id.as_deref(), Some("device-test-value"));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "expired pending scenario").await;
}

#[tokio::test]
async fn discogs_auth_transition_persistence_errors_keep_recoverable_state() {
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("authorized").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.fail_next_store();
        let server = ReklawdboxServer::new(None);
        set_discogs_pending(&server, "persistence-device", DISCOGS_AUTH_TEST_NOW + 3_600);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);

        let first = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(
            first,
            Err(crate::adapters::providers::discogs::LookupError::Message(_))
        ));
        assert!(
            server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_some()
        );
        assert!(!persistence.has_session(&fixture.base_url));

        let second = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(second, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Finalize), 2);
        assert_eq!(persistence.store_count(), 1);
        assert!(persistence.has_session(&fixture.base_url));
        assert!(
            server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_none()
        );

        persistence.set_session(
            &fixture.base_url,
            "expired-session-fixture",
            DISCOGS_AUTH_TEST_NOW,
        );
        persistence.fail_next_clear();
        let clear_failure = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(
            clear_failure,
            Err(crate::adapters::providers::discogs::LookupError::Message(_))
        ));
        assert!(persistence.has_session(&fixture.base_url));
        fixture.shutdown().await;
    };

    tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario)
        .await
        .expect("persistence error scenario should finish within five seconds");
}

#[tokio::test]
async fn discogs_auth_transition_errors_are_sanitized() {
    let scenario = async {
        let start_fixture = DiscogsBrokerFixture::start("pending").await;
        start_fixture.state.fail_endpoint(
            DiscogsBrokerEndpoint::Start,
            r#"{"pending_token":"fixture-private-start","broker_credential":"fixture-private-broker"}"#,
        );
        let start_persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let start_server = ReklawdboxServer::new(None);
        start_server
            .set_discogs_auth_test_dependencies(DiscogsAuthTestDependencies::new(
                crate::adapters::providers::discogs::BrokerConfig {
                    base_url: start_fixture.base_url.clone(),
                    broker_token: Some("fixture-private-broker".to_string()),
                },
                DISCOGS_AUTH_TEST_NOW,
                Arc::clone(&start_persistence),
            ))
            .expect("start-error dependencies should install");
        let start_result = resolve_discogs_auth_transition_for_test(&start_server).await;
        assert_sanitized_discogs_transition_error(
            start_result,
            "Discogs authentication start failed. Retry the lookup to start authorization.",
        );
        assert_eq!(start_fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert!(!start_persistence.has_session(&start_fixture.base_url));
        start_fixture.shutdown().await;

        let status_fixture = DiscogsBrokerFixture::start("pending").await;
        status_fixture.state.fail_endpoint(
            DiscogsBrokerEndpoint::Status,
            r#"{"pending_token":"fixture-private-status","response_header":"fixture-private-header"}"#,
        );
        let status_persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let status_server = ReklawdboxServer::new(None);
        set_discogs_pending(
            &status_server,
            "status-error-device",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        install_discogs_auth_test_dependencies(
            &status_server,
            &status_fixture,
            status_persistence,
            None,
        );
        let status_result = resolve_discogs_auth_transition_for_test(&status_server).await;
        assert_sanitized_discogs_transition_error(
            status_result,
            "Discogs authentication status check failed. Retry the lookup to continue authorization.",
        );
        assert_eq!(status_fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        assert!(
            status_server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_some()
        );
        status_fixture.shutdown().await;

        let finalize_fixture = DiscogsBrokerFixture::start("authorized").await;
        finalize_fixture.state.fail_endpoint(
            DiscogsBrokerEndpoint::Finalize,
            r#"{"session_token":"fixture-private-session","pending_token":"fixture-private-finalize"}"#,
        );
        let finalize_persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let finalize_server = ReklawdboxServer::new(None);
        set_discogs_pending(
            &finalize_server,
            "finalize-error-device",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        install_discogs_auth_test_dependencies(
            &finalize_server,
            &finalize_fixture,
            Arc::clone(&finalize_persistence),
            None,
        );
        let finalize_result = resolve_discogs_auth_transition_for_test(&finalize_server).await;
        assert_sanitized_discogs_transition_error(
            finalize_result,
            "Discogs authentication finalization failed. Retry the lookup to complete authorization.",
        );
        assert_eq!(
            finalize_fixture
                .state
                .count(DiscogsBrokerEndpoint::Finalize),
            1
        );
        assert!(!finalize_persistence.has_session(&finalize_fixture.base_url));
        assert!(
            finalize_server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_some()
        );
        finalize_fixture.shutdown().await;

        let closed_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("closed-endpoint listener should bind");
        let closed_address = closed_listener
            .local_addr()
            .expect("closed-endpoint listener should have an address");
        drop(closed_listener);
        let connection_server = ReklawdboxServer::new(None);
        connection_server
            .set_discogs_auth_test_dependencies(DiscogsAuthTestDependencies::new(
                crate::adapters::providers::discogs::BrokerConfig {
                    base_url: format!("http://{closed_address}"),
                    broker_token: None,
                },
                DISCOGS_AUTH_TEST_NOW,
                Arc::new(InMemoryDiscogsSessionPersistence::default()),
            ))
            .expect("connection-error dependencies should install");
        let connection_result = resolve_discogs_auth_transition_for_test(&connection_server).await;
        assert_sanitized_discogs_transition_error(
            connection_result,
            "Discogs authentication start failed. Retry the lookup to start authorization.",
        );
    };

    tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario)
        .await
        .expect("sanitized transition-error scenario should finish within five seconds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_transition_cancelled_waiter_reuses_committed_state() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let owner_server = server.clone();
        let owner = tasks
            .spawn(async move { resolve_discogs_auth_transition_for_test(&owner_server).await });
        delayed_start
            .wait_until_entered("owner device-session start")
            .await?;

        let (waiter_entered_tx, waiter_entered_rx) = tokio::sync::oneshot::channel();
        let waiter_server = server.clone();
        let waiter = tasks.spawn(async move {
            let _ = waiter_entered_tx.send(());
            resolve_discogs_auth_transition_for_test(&waiter_server).await
        });
        tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, waiter_entered_rx)
            .await
            .map_err(|_| "waiting lookup should enter within five seconds".to_string())?
            .map_err(|_| "waiting lookup entry notification should arrive".to_string())?;
        tasks
            .abort_and_reap(waiter, "cancelled auth-lock waiter")
            .await?;

        delayed_start.release();
        let owner = tasks.join(owner, "auth transition owner").await?;
        assert!(matches!(owner, Ok(Some(_))));

        let next = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(next, Ok(Some(_))));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "cancelled waiter scenario").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_rejected_token_compare_clear_and_searches_run_outside_lock() {
    let mut concurrent_search_tasks = DiscogsAuthTasks::default();
    let concurrent_searches = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.set_session(
            &fixture.base_url,
            "concurrent-search-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_search = fixture.state.delay_next(DiscogsBrokerEndpoint::Search);

        let first_server = server.clone();
        let first = concurrent_search_tasks.spawn(async move {
            lookup_discogs_remote(&first_server, "Search Artist", "First Search", None).await
        });
        delayed_search
            .wait_until_entered("first ordinary Discogs search")
            .await?;

        let second_server = server.clone();
        let second = concurrent_search_tasks.spawn(async move {
            lookup_discogs_remote(&second_server, "Search Artist", "Second Search", None).await
        });
        fixture
            .state
            .wait_for_count(
                DiscogsBrokerEndpoint::Search,
                2,
                "second ordinary Discogs search while the first is delayed",
            )
            .await?;
        delayed_search.release();

        let first = concurrent_search_tasks
            .join(first, "first ordinary Discogs search")
            .await?;
        let second = concurrent_search_tasks
            .join(second, "second ordinary Discogs search")
            .await?;
        assert!(matches!(first, Ok(None)));
        assert!(matches!(second, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(persistence.clear_count(), 0);
        fixture.shutdown().await;
        Ok::<(), String>(())
    };
    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, concurrent_searches).await;
    finish_discogs_auth_scenario(
        &mut concurrent_search_tasks,
        outcome,
        "ordinary searches outside the auth lock",
    )
    .await;

    let mut replacement_tasks = DiscogsAuthTasks::default();
    let replacement_survives = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.set_session(
            &fixture.base_url,
            "rejected-old-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        fixture.state.reject_token("rejected-old-session-fixture");
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_search = fixture.state.delay_next(DiscogsBrokerEndpoint::Search);

        let lookup_server = server.clone();
        let lookup = replacement_tasks.spawn(async move {
            lookup_discogs_remote(&lookup_server, "Race Artist", "Replacement Wins", None).await
        });
        delayed_search
            .wait_until_entered("rejected old-token search")
            .await?;
        persistence.set_session(
            &fixture.base_url,
            "new-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        delayed_search.release();

        let result = replacement_tasks
            .join(lookup, "replacement-token retry")
            .await?;
        assert!(matches!(result, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(persistence.clear_count(), 0);
        assert!(persistence.session_matches(&fixture.base_url, "new-session-fixture"));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };
    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, replacement_survives).await;
    finish_discogs_auth_scenario(
        &mut replacement_tasks,
        outcome,
        "new persisted session surviving stale rejection",
    )
    .await;

    let mut bounded_retry_tasks = DiscogsAuthTasks::default();
    let retry_is_bounded = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.set_session(
            &fixture.base_url,
            "first-rejected-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        fixture.state.reject_token("first-rejected-session-fixture");
        fixture
            .state
            .reject_token("second-rejected-session-fixture");
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_search = fixture.state.delay_next(DiscogsBrokerEndpoint::Search);

        let lookup_server = server.clone();
        let lookup = bounded_retry_tasks.spawn(async move {
            lookup_discogs_remote(&lookup_server, "Race Artist", "Bounded Retry", None).await
        });
        delayed_search
            .wait_until_entered("first rejected search in bounded retry")
            .await?;
        persistence.set_session(
            &fixture.base_url,
            "second-rejected-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        delayed_search.release();

        let result = bounded_retry_tasks
            .join(lookup, "bounded rejected-token retry")
            .await?;
        assert!(matches!(
            result,
            Err(crate::adapters::providers::discogs::LookupError::AuthRequired(_))
        ));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 0);
        assert_eq!(persistence.clear_count(), 1);
        assert!(!persistence.has_session(&fixture.base_url));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };
    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, retry_is_bounded).await;
    finish_discogs_auth_scenario(
        &mut bounded_retry_tasks,
        outcome,
        "rejected-token recovery bounded to one retry",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrich_tracks_discogs_auth_concurrency_starts_one_session_without_cache_writes() {
    let mut batch_tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let db_conn = create_single_track_test_db(
            "discogs-auth-batch-one",
            "/tmp/discogs-auth-batch-one.flac",
        );
        insert_test_track(
            &db_conn,
            "discogs-auth-batch-two",
            "Concurrent Auth Two",
            "g1",
            "/tmp/discogs-auth-batch-two.flac",
        );
        let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            Arc::clone(&persistence),
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let batch_server = server.clone();
        let batch = batch_tasks.spawn(async move {
            batch_server
                .enrich_tracks(Parameters(EnrichTracksParams {
                    filters: SearchFilterParams::default(),
                    track_ids: Some(vec![
                        "discogs-auth-batch-one".to_string(),
                        "discogs-auth-batch-two".to_string(),
                    ]),
                    playlist_id: None,
                    max_tracks: Some(2),
                    offset: None,
                    providers: Some(vec![
                        crate::application::enrichment::model::EnrichmentProvider::Discogs,
                    ]),
                    skip_cached: Some(false),
                    force_refresh: Some(true),
                    concurrency: Some(2),
                }))
                .await
        });

        delayed_start
            .wait_until_entered("concurrent enrichment device-session start")
            .await?;
        delayed_start.release();
        let result = batch_tasks
            .join(batch, "concurrent Discogs enrichment batch")
            .await
            .map_err(|error| format!("concurrent Discogs enrichment task failed: {error}"))?
            .map_err(|error| {
                format!("concurrent Discogs enrichment should return a batch payload: {error:?}")
            })?;
        let payload = extract_json(&result);

        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        assert_eq!(payload["summary"]["tracks_total"], 2);
        assert_eq!(payload["summary"]["total"], 2);
        assert_eq!(payload["summary"]["enriched"], 0);
        assert_eq!(payload["summary"]["cached"], 0);
        assert_eq!(payload["summary"]["skipped"], 0);
        assert_eq!(payload["summary"]["failed"], 2);
        assert_cache_write_summary(&payload, 0, 0, 0);

        let failures = payload["failures"]
            .as_array()
            .expect("concurrent Discogs failures should be an array");
        assert_eq!(failures.len(), 2);
        let errors = failures
            .iter()
            .map(|failure| {
                assert_eq!(failure["provider"], "discogs");
                failure["error"]
                    .as_str()
                    .expect("Discogs auth failure should include an error")
            })
            .collect::<Vec<_>>();
        assert!(
            errors
                .iter()
                .all(|error| error.contains("Auth URL: https://auth.example/device"))
        );
        assert_eq!(errors[0], errors[1]);

        let serialized = payload.to_string();
        assert!(!serialized.contains("pending-test-value"));
        assert!(!serialized.contains("session-test-value"));
        assert!(!persistence.has_session(&fixture.base_url));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(
        &mut batch_tasks,
        outcome,
        "concurrent Discogs enrichment scenario",
    )
    .await;
}
