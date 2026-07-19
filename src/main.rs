mod adapters;
mod application;
mod bootstrap;
mod cli;
mod domain;
mod mcp;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use std::io::IsTerminal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Both functions only set vars not already present, so call order
    // determines priority: shell env > .mcp.json > config.toml.
    bootstrap::environment::load_env_from_mcp_json();
    let cfg = adapters::platform::config::load();
    // SAFETY: same context as load_env_from_mcp_json above.
    unsafe { adapters::platform::config::apply_env(&cfg) };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    domain::classification::taxonomy::init_overrides(cfg.genre_overrides);
    match bootstrap::mode::detect(std::env::args(), std::io::stdin().is_terminal()) {
        bootstrap::mode::LaunchMode::Cli => cli::run().await,
        bootstrap::mode::LaunchMode::McpStdio => {
            let server = mcp::ReklawdboxServer::new(adapters::rekordbox::resolve_db_path());
            let service = server.serve(stdio()).await?;
            service.waiting().await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    #[ignore]
    fn private_rekordbox_real_change_pipeline() {
        let fixture = crate::adapters::rekordbox::test_support::PrivateRekordboxFixture::from_env()
            .expect("private Rekordbox fixture should be configured");
        let conn = fixture
            .open()
            .expect("private Rekordbox fixture should open read-only");
        let params = crate::adapters::rekordbox::SearchParams {
            query: None,
            artist: None,
            genre: None,
            rating_min: None,
            bpm_min: Some(120.0),
            bpm_max: Some(130.0),
            key: None,
            playlist: None,
            has_genre: None,
            has_label: None,
            year_zero: None,
            label: None,
            path: None,
            path_prefix: None,
            added_after: None,
            added_before: None,
            exclude_samples: false,
            limit: Some(5),
            offset: None,
        };
        let tracks = crate::adapters::rekordbox::search_tracks(&conn, &params).unwrap();
        assert!(!tracks.is_empty(), "need tracks for pipeline test");

        let track = &tracks[0];
        let cm = crate::domain::metadata::ChangeManager::new();
        let (staged, total) = cm.stage(vec![crate::domain::metadata::TrackChange {
            track_id: track.id.clone(),
            genre: Some("Deep House".to_string()),
            comments: Some("integration test".to_string()),
            rating: Some(4),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        assert!(staged == 1, "exactly one change should stage");
        assert!(total == 1, "exactly one change should be counted");

        let diffs = cm.preview(&tracks);
        assert!(!diffs.is_empty(), "expected diffs for staged changes");
        let td = &diffs[0];
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "genre" && f.new_value == "Deep House")
        );
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "comments" && f.new_value == "integration test")
        );

        let modified = cm.apply_changes(&tracks);
        let modified_track = modified.iter().find(|t| t.id == track.id).unwrap();
        assert!(
            modified_track.genre == "Deep House",
            "staged genre should apply"
        );
        assert!(
            modified_track.comments == "integration test",
            "staged comments should apply"
        );
        assert!(modified_track.rating == 4, "staged rating should apply");

        let xml = crate::adapters::rekordbox::xml::generate_xml(&modified);
        assert!(xml.contains("Genre=\"Deep House\""));
        assert!(xml.contains("Comments=\"integration test\""));
        assert!(xml.contains("Rating=\"204\""));

        for track in &modified {
            if track.id != modified_track.id {
                let original = tracks
                    .iter()
                    .find(|original| original.id == track.id)
                    .unwrap();
                assert!(
                    track.genre == original.genre,
                    "unstaged genre must remain unchanged"
                );
                assert!(
                    track.comments == original.comments,
                    "unstaged comments must remain unchanged"
                );
                assert!(
                    track.rating == original.rating,
                    "unstaged rating must remain unchanged"
                );
            }
        }
    }
}
