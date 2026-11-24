use crate::adapters::rekordbox::cues::{list_cues, removal_inventory};
use rusqlite::Connection;
use serde_json::{Value, to_value};

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("
        CREATE TABLE djmdContent(ID TEXT PRIMARY KEY, UUID TEXT, Title TEXT, ArtistID TEXT, rb_local_deleted INT);
        CREATE TABLE djmdArtist(ID TEXT PRIMARY KEY, Name TEXT);
        CREATE TABLE djmdCue(ID TEXT PRIMARY KEY, ContentID TEXT, InMsec INTEGER, OutMsec INTEGER,
            Kind INTEGER, Comment TEXT COLLATE NOCASE, Color INTEGER, ColorTableIndex INTEGER,
            ActiveLoop INTEGER, rb_local_deleted INTEGER, extra BLOB);
        INSERT INTO djmdContent VALUES ('1','u1','one',NULL,0),('2','u2','two',NULL,NULL),('3','u3','gone',NULL,1);
        INSERT INTO djmdCue(ID,ContentID,InMsec,OutMsec,Kind,Comment,rb_local_deleted) VALUES
            ('a','1',10,-1,0,'AUTO',0),('b','1',10,30,0,'AUTO',0),('c','1',10,-1,1,'AUTO',0),
            ('d','1',10,-1,17,'AUTO',0),('e','2',10,NULL,0,'auto',0),('f','2',10,-1,0,NULL,0),
            ('g','3',10,-1,0,'AUTO',0),('h','2',10,-1,0,'AUTO',1),('i','2',10,-1,0,'',0);
    ").unwrap();
    conn
}

#[test]
fn exact_filter_preserves_unknown_kinds_loops_and_totals_on_empty_pages() {
    let conn = fixture();
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
    assert_eq!(result["cues"][2]["raw_kind"], 17);
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
fn verification_detects_protected_row_replacement_unknown_fields_and_track_loss() {
    let conn = fixture();
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
