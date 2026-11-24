use crate::adapters::rekordbox::cues::RemovalInventory;
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct RemovalPreview {
    matched_memory_points: usize,
    matched_tracks: usize,
    protected_cue_rows: usize,
    verification_fingerprint: Option<String>,
}

pub(crate) fn preview(inventory: RemovalInventory) -> RemovalPreview {
    RemovalPreview {
        matched_memory_points: inventory.target_count,
        matched_tracks: inventory.target_tracks,
        protected_cue_rows: inventory.protected_count,
        verification_fingerprint: (inventory.target_count > 0).then_some(inventory.fingerprint),
    }
}

#[derive(Serialize)]
pub(crate) struct RemovalVerification {
    verified: bool,
    remaining_memory_points: usize,
    protected_library_matches: bool,
}

pub(crate) fn verify(current: RemovalInventory, expected: &str) -> RemovalVerification {
    let protected_library_matches = current.fingerprint == expected;
    RemovalVerification {
        verified: protected_library_matches && current.target_count == 0,
        remaining_memory_points: current.target_count,
        protected_library_matches,
    }
}
