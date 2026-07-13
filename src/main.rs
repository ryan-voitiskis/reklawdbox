mod adapters;
mod anlz;
mod application;
mod audio;
mod audio_profile;
mod audit;
mod backup;
mod bandcamp;
mod beatport;
mod bootstrap;
mod changes;
mod classify;
mod cli;
mod color;
mod config;
#[cfg(test)]
mod corpus;
mod db;
mod discogs;
mod domain;
#[cfg(test)]
mod eval_routing;
#[cfg(test)]
mod eval_tasks;
mod genre;
mod keychain;
mod mcp;
mod musicbrainz;
mod normalize;
mod rate_limit;
mod store;
mod tags;
mod types;
mod xml;

use rmcp::ServiceExt;
use rmcp::transport::stdio;
use std::io::IsTerminal;

#[cfg(test)]
pub(crate) use bootstrap::environment::project_root;

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
    genre::init_overrides(cfg.genre_overrides);
    match bootstrap::mode::detect(std::env::args(), std::io::stdin().is_terminal()) {
        bootstrap::mode::LaunchMode::Cli => cli::run().await,
        bootstrap::mode::LaunchMode::McpStdio => {
            let server = mcp::ReklawdboxServer::new(db::resolve_db_path());
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
    fn real_change_pipeline() {
        let conn = crate::db::open_real_db().expect("backup tarball not found");
        let params = crate::db::SearchParams {
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
        let tracks = crate::db::search_tracks(&conn, &params).unwrap();
        assert!(!tracks.is_empty(), "need tracks for pipeline test");

        let track = &tracks[0];
        let cm = crate::changes::ChangeManager::new();
        let (staged, total) = cm.stage(vec![crate::types::TrackChange {
            track_id: track.id.clone(),
            genre: Some("Deep House".to_string()),
            comments: Some("integration test".to_string()),
            rating: Some(4),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        assert_eq!(staged, 1);
        assert_eq!(total, 1);

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
        assert_eq!(modified_track.genre, "Deep House");
        assert_eq!(modified_track.comments, "integration test");
        assert_eq!(modified_track.rating, 4);

        let xml = crate::xml::generate_xml(&modified);
        assert!(xml.contains("Genre=\"Deep House\""));
        assert!(xml.contains("Comments=\"integration test\""));
        assert!(xml.contains("Rating=\"204\""));

        for track in &modified {
            if track.id != modified_track.id {
                let original = tracks
                    .iter()
                    .find(|original| original.id == track.id)
                    .unwrap();
                assert_eq!(track.genre, original.genre);
                assert_eq!(track.comments, original.comments);
                assert_eq!(track.rating, original.rating);
            }
        }
    }
}
