pub(in crate::mcp) trait HasScore {
    fn score(&self) -> i32;
}

impl HasScore for crate::bandcamp::BandcampResult {
    fn score(&self) -> i32 {
        self.score
    }
}

impl HasScore for crate::musicbrainz::MusicBrainzResult {
    fn score(&self) -> i32 {
        self.score
    }
}
