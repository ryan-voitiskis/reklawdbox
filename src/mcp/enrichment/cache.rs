pub(in crate::mcp) trait HasScore {
    fn score(&self) -> i32;
}

impl HasScore for crate::adapters::providers::bandcamp::BandcampResult {
    fn score(&self) -> i32 {
        self.score
    }
}

impl HasScore for crate::adapters::providers::musicbrainz::MusicBrainzResult {
    fn score(&self) -> i32 {
        self.score
    }
}
