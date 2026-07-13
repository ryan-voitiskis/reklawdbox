/// Convert 0-5 star rating to Rekordbox DB/XML encoding (0/51/102/153/204/255).
pub fn stars_to_rating(stars: u8) -> u16 {
    match stars {
        0 => 0,
        1 => 51,
        2 => 102,
        3 => 153,
        4 => 204,
        5 => 255,
        _ => 255,
    }
}

/// Convert Rekordbox DB/XML rating encoding to 0-5 stars.
pub fn rating_to_stars(rating: u16) -> u8 {
    match rating {
        0..=25 => 0,
        26..=76 => 1,
        77..=127 => 2,
        128..=178 => 3,
        179..=229 => 4,
        230..=255 => 5,
        _ => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rating_roundtrip() {
        for stars in 0..=5u8 {
            let encoded = stars_to_rating(stars);
            let decoded = rating_to_stars(encoded);
            assert_eq!(
                stars, decoded,
                "roundtrip failed for {stars} stars (encoded: {encoded})"
            );
        }
    }

    #[test]
    fn rating_exact_values() {
        assert_eq!(stars_to_rating(0), 0);
        assert_eq!(stars_to_rating(1), 51);
        assert_eq!(stars_to_rating(2), 102);
        assert_eq!(stars_to_rating(3), 153);
        assert_eq!(stars_to_rating(4), 204);
        assert_eq!(stars_to_rating(5), 255);
    }

    #[test]
    fn stars_out_of_range_saturates_to_five_stars() {
        assert_eq!(stars_to_rating(6), 255);
        assert_eq!(stars_to_rating(u8::MAX), 255);
        assert_eq!(rating_to_stars(300), 5);
        assert_eq!(rating_to_stars(u16::MAX), 5);
    }

    #[test]
    fn rating_bucket_boundaries() {
        assert_eq!(rating_to_stars(25), 0);
        assert_eq!(rating_to_stars(26), 1);
        assert_eq!(rating_to_stars(76), 1);
        assert_eq!(rating_to_stars(77), 2);
        assert_eq!(rating_to_stars(127), 2);
        assert_eq!(rating_to_stars(128), 3);
        assert_eq!(rating_to_stars(178), 3);
        assert_eq!(rating_to_stars(179), 4);
        assert_eq!(rating_to_stars(229), 4);
        assert_eq!(rating_to_stars(230), 5);
        assert_eq!(rating_to_stars(255), 5);
    }
}
