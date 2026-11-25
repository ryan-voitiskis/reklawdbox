use crate::adapters::rekordbox::cues::{list_cues, removal_inventory};
use crate::adapters::rekordbox::test_support::create_cue_test_db;
use rusqlite::params;
use serde_json::{Value, to_value};

#[test]
fn exact_filter_preserves_unknown_kinds_loops_and_totals_on_empty_pages() {
    let conn = create_cue_test_db();
    let page = |s, limit, offset| -> Value {
        to_value(list_cues(&conn, None, s, false, limit, offset).unwrap()).unwrap()
    };
    let eligible = to_value(list_cues(&conn, None, Some("AUTO"), true, 50, 0).unwrap()).unwrap();
    assert_eq!(eligible["total_cues"], 1);
    assert_eq!(eligible["cues"][0]["cue_id"], "a");
    let result = page(Some("AUTO"), 3, 1);
    assert_eq!(result["total_cues"], 4);
    assert_eq!(result["total_tracks"], 1);
    assert_eq!(result["cues"][0]["cue_id"], "b");
    assert_eq!(result["cues"][0]["loop_end_ms"], 30);
    assert_eq!(result["cues"][1]["hot_slot"], "A");
    assert_eq!(result["cues"][2]["raw_kind"], 99);
    assert_eq!(result["cues"][2]["kind"], "unknown");
    assert!(result["cues"][2]["loop_end_ms"].is_null());
    assert_eq!(page(Some(""), 0, 0)["total_cues"], 1);
    assert_eq!(page(None, 3, 99)["total_tracks"], 2);
    assert_eq!(page(None, 3, 99)["cues"], serde_json::json!([]));
    assert_eq!(
        to_value(list_cues(&conn, Some("2"), Some("AUTO"), false, 50, 0).unwrap()).unwrap()["total_cues"],
        0
    );
}

#[test]
fn verification_detects_protected_row_replacement_unknown_fields_and_track_identity_changes() {
    let conn = create_cue_test_db();
    let inv = || removal_inventory(&conn, Some("1"), "AUTO").unwrap();
    let before = inv();
    assert_eq!(
        (
            before.target_count,
            before.target_tracks,
            before.protected_count
        ),
        (1, 1, 6)
    );
    assert_ne!(
        before.fingerprint,
        removal_inventory(&conn, None, "AUTO").unwrap().fingerprint
    );
    conn.execute("UPDATE djmdCue SET rb_local_deleted=1 WHERE ID='a'", [])
        .unwrap();
    assert_eq!(inv().target_count, 0);
    assert_eq!(inv().fingerprint, before.fingerprint);
    conn.execute("UPDATE djmdCue SET extra=x'01' WHERE ID='d'", [])
        .unwrap();
    assert_ne!(inv().fingerprint, before.fingerprint);
    conn.execute(
        "UPDATE djmdCue SET extra=NULL, ID='replacement' WHERE ID='d'",
        [],
    )
    .unwrap();
    assert_ne!(inv().fingerprint, before.fingerprint);
    conn.execute("UPDATE djmdCue SET ID='d' WHERE ID='replacement'", [])
        .unwrap();
    conn.execute(
        "UPDATE djmdContent SET UUID='another-track' WHERE ID='2'",
        [],
    )
    .unwrap();
    assert_ne!(inv().fingerprint, before.fingerprint);
}

#[test]
fn list_identifies_all_sixteen_hot_slots_and_the_separate_memory_loop_kind() {
    let conn = create_cue_test_db();
    for (slot, kind) in (1..=3).chain(5..=17).enumerate() {
        conn.execute(
            "INSERT INTO djmdCue(ID,ContentID,InMsec,Kind,Comment)
             VALUES (?1,'1',?2,?3,'pads')",
            params![format!("pad-{slot}"), slot, kind],
        )
        .unwrap();
    }
    let page = to_value(list_cues(&conn, Some("1"), Some("pads"), false, 50, 0).unwrap()).unwrap();
    let pads = page["cues"].as_array().unwrap();
    assert_eq!(pads.len(), 16);
    for (cue, letter) in pads.iter().zip('A'..='P') {
        assert_eq!(cue["kind"], "hot");
        assert_eq!(cue["hot_slot"], letter.to_string());
    }
    conn.execute_batch(
        "INSERT INTO djmdCue(ID,ContentID,InMsec,OutMsec,Kind,Comment) VALUES
         ('1-loop','1',1,500,4,'edge'), ('2-no-end','1',2,-1,4,'edge'),
         ('3-null-kind','1',3,NULL,NULL,'edge'), ('4-null-end','1',4,NULL,0,'edge'),
         ('5-max-end','1',5,4294967295,0,'edge');",
    )
    .unwrap();
    let page = to_value(list_cues(&conn, Some("1"), Some("edge"), false, 50, 0).unwrap()).unwrap();
    let rows = page["cues"].as_array().unwrap();
    assert_eq!(rows[0]["kind"], "memory");
    assert_eq!(rows[0]["loop_end_ms"], 500);
    assert!(rows[0]["hot_slot"].is_null());
    assert_eq!(rows[1]["kind"], "unknown");
    assert_eq!(rows[2]["kind"], "unknown");
    assert!(rows[2]["raw_kind"].is_null());
    let eligible =
        to_value(list_cues(&conn, Some("1"), Some("edge"), true, 50, 0).unwrap()).unwrap();
    let eligible = eligible["cues"].as_array().unwrap();
    assert_eq!(eligible.len(), 2);
    assert_eq!(eligible[0]["cue_id"], "4-null-end");
    assert_eq!(eligible[1]["cue_id"], "5-max-end");
    assert!(eligible.iter().all(|cue| cue["loop_end_ms"].is_null()));
    assert_eq!(
        removal_inventory(&conn, Some("1"), "edge")
            .unwrap()
            .target_count,
        2
    );
}
