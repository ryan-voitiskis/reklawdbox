use super::super::*;
use rusqlite::Connection;

pub fn create_test_db() -> Connection {
    let conn = open_test();
    seed_test_db(&conn);
    conn
}

pub(super) fn seed_test_db(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE djmdArtist (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdAlbum (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdGenre (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdKey (
            ID VARCHAR(255) PRIMARY KEY,
            ScaleName VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdLabel (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdColor (
            ID VARCHAR(255) PRIMARY KEY,
            ColorCode INTEGER,
            Commnt VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdContent (
            ID VARCHAR(255) PRIMARY KEY,
            Title VARCHAR(255),
            ArtistID VARCHAR(255),
            AlbumID VARCHAR(255),
            GenreID VARCHAR(255),
            KeyID VARCHAR(255),
            ColorID VARCHAR(255),
            LabelID VARCHAR(255),
            RemixerID VARCHAR(255),
            BPM INTEGER DEFAULT 0,
            Rating INTEGER DEFAULT 0,
            Commnt TEXT DEFAULT '',
            ReleaseYear INTEGER DEFAULT 0,
            Length INTEGER DEFAULT 0,
            FolderPath VARCHAR(255) DEFAULT '',
            DJPlayCount VARCHAR(255) DEFAULT '0',
            BitRate INTEGER DEFAULT 0,
            SampleRate INTEGER DEFAULT 0,
            FileType INTEGER DEFAULT 0,
            created_at TEXT DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdPlaylist (
            ID VARCHAR(255) PRIMARY KEY,
            Seq INTEGER,
            Name VARCHAR(255),
            Attribute INTEGER DEFAULT 0,
            ParentID VARCHAR(255) DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdSongPlaylist (
            ID VARCHAR(255) PRIMARY KEY,
            PlaylistID VARCHAR(255),
            ContentID VARCHAR(255),
            TrackNo INTEGER
        );

        -- Lookup data
        INSERT INTO djmdArtist (ID, Name) VALUES ('a1', 'Burial');
        INSERT INTO djmdArtist (ID, Name) VALUES ('a2', 'Actress');
        INSERT INTO djmdArtist (ID, Name) VALUES ('a3', 'Ricardo Villalobos');
        INSERT INTO djmdAlbum (ID, Name) VALUES ('al1', 'Untrue');
        INSERT INTO djmdAlbum (ID, Name) VALUES ('al2', 'R.I.P.');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g1', 'Dubstep');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'Techno');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g3', 'Minimal');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g4', 'Wonky Bass');
        INSERT INTO djmdKey (ID, ScaleName) VALUES ('k1', 'Am');
        INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Cm');
        INSERT INTO djmdKey (ID, ScaleName) VALUES ('k3', 'Fm');
        INSERT INTO djmdLabel (ID, Name) VALUES ('l1', 'Hyperdub');
        INSERT INTO djmdLabel (ID, Name) VALUES ('l2', 'Ninja Tune');
        INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c1', 16711935, 'Rose');
        INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c2', 65280, 'Green');

        -- Tracks (created_at uses full datetime to match real Rekordbox format)
        INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, LabelID, ColorID, BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate, SampleRate, FileType, created_at)
        VALUES ('t1', 'Archangel', 'a1', 'al1', 'g1', 'k1', 'l1', 'c1', 13950, 204, 'iconic garage vocal', 2007, 240, '/Users/testuser/Music/Burial/Untrue/01 Archangel.flac', '12', 1411, 44100, 5, '2023-01-15 10:30:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, LabelID, BPM, Rating, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate, SampleRate, FileType, created_at)
        VALUES ('t2', 'Endorphin', 'a1', 'al1', 'g1', 'k2', 'l1', 14000, 153, 2007, 300, '/Users/testuser/Music/Burial/Untrue/02 Endorphin.flac', '5', 1411, 44100, 5, '2023-01-15 10:31:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, BPM, Rating, ReleaseYear, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t3', 'R.I.P.', 'a2', 'al2', 'g2', 'k3', 12800, 102, 2012, 360, '/Users/testuser/Music/Actress/R.I.P./01 R.I.P..flac', 1411, 44100, 5, '2023-02-20 14:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t4', 'Dexter', 'a3', 'g3', 12500, 480, '/Users/testuser/Music/Villalobos/Dexter.wav', 1411, 44100, 11, '2023-03-10 09:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t5', 'Unknown Track', 'a1', 0, 200, '/Users/testuser/Music/unknown.mp3', 320, 44100, 1, '2023-04-01 12:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t6', 'Loop Sample 01', 'a1', 'g2', 12000, 8, '/Users/alice/Music/rekordbox/Sampler/Loop/01.wav', 1411, 44100, 11, '2023-01-01 08:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t7', 'Wonky Bassline', 'a2', 'g4', 12600, 300, '/Users/testuser/Music/Actress/Wonky.flac', 1411, 44100, 5, '2023-02-20 15:00:00.000 +00:00');

        -- Playlists
        INSERT INTO djmdPlaylist (ID, Seq, Name, Attribute, ParentID) VALUES ('p1', 1, 'Deep Cuts', 0, 'root');
        INSERT INTO djmdPlaylist (ID, Seq, Name, Attribute, ParentID) VALUES ('p2', 2, 'Folders', 1, 'root');
        INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo) VALUES ('sp1', 'p1', 't1', 1);
        INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo) VALUES ('sp2', 'p1', 't3', 2);

        -- History tables
        CREATE TABLE djmdHistory (
            ID VARCHAR(255) PRIMARY KEY,
            Seq INTEGER,
            Name VARCHAR(255),
            Attribute INTEGER DEFAULT 0,
            ParentID VARCHAR(255) DEFAULT '',
            DateCreated TEXT DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdSongHistory (
            ID VARCHAR(255) PRIMARY KEY,
            HistoryID VARCHAR(255),
            ContentID VARCHAR(255),
            TrackNo INTEGER,
            created_at TEXT DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );

        -- Sessions: h1 (2025-03-01), h2 (2025-02-15), hf1 is a folder (Attribute=1)
        INSERT INTO djmdHistory (ID, Seq, Name, Attribute, DateCreated) VALUES ('h1', 1, '2025-03-01', 0, '2025-03-01');
        INSERT INTO djmdHistory (ID, Seq, Name, Attribute, DateCreated) VALUES ('h2', 2, '2025-02-15', 0, '2025-02-15');
        INSERT INTO djmdHistory (ID, Seq, Name, Attribute, DateCreated) VALUES ('hf1', 3, 'History Folder', 1, '2025-01-01');

        -- Song history: h1 has 3 tracks (t1, t3, t2) with 5min gaps; h2 has 1 track (t1)
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh1', 'h1', 't1', 1, '2025-03-01 22:00:00');
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh2', 'h1', 't3', 2, '2025-03-01 22:05:00');
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh3', 'h1', 't2', 3, '2025-03-01 22:10:00');
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh4', 'h2', 't1', 1, '2025-02-15 20:00:00');
        ",
    )
    .unwrap();
}
