/// Standard Rekordbox color palette (8 colors).
/// Values are the integer representation of the hex color codes used in Rekordbox XML.
pub const COLORS: &[(&str, i32)] = &[
    ("Blue", 0x0000FF),
    ("Green", 0x00FF00),
    ("Lemon", 0xFFFF00),
    ("Orange", 0xFFA500),
    ("Red", 0xFF0000),
    ("Rose", 0xFF007F),
    ("Turquoise", 0x25FDE9),
    ("Violet", 0x660099),
];

/// Convert a color name to its Rekordbox hex code. Case-insensitive.
pub fn color_name_to_code(name: &str) -> Option<i32> {
    COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, code)| *code)
}

/// Returns the canonical casing of a color name, or None if unknown.
pub fn canonical_casing(name: &str) -> Option<&'static str> {
    COLORS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(n, _)| *n)
}

pub fn is_valid_color(name: &str) -> bool {
    canonical_casing(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_to_code_exact_case() {
        assert_eq!(color_name_to_code("Rose"), Some(0xFF007F));
        assert_eq!(color_name_to_code("Red"), Some(0xFF0000));
        assert_eq!(color_name_to_code("Blue"), Some(0x0000FF));
        assert_eq!(color_name_to_code("Green"), Some(0x00FF00));
        assert_eq!(color_name_to_code("Orange"), Some(0xFFA500));
        assert_eq!(color_name_to_code("Lemon"), Some(0xFFFF00));
        assert_eq!(color_name_to_code("Turquoise"), Some(0x25FDE9));
        assert_eq!(color_name_to_code("Violet"), Some(0x660099));
    }

    #[test]
    fn name_to_code_case_insensitive() {
        assert_eq!(color_name_to_code("rose"), Some(0xFF007F));
        assert_eq!(color_name_to_code("ROSE"), Some(0xFF007F));
        assert_eq!(color_name_to_code("rOsE"), Some(0xFF007F));
        assert_eq!(color_name_to_code("red"), Some(0xFF0000));
        assert_eq!(color_name_to_code("GREEN"), Some(0x00FF00));
        assert_eq!(color_name_to_code("vIoLeT"), Some(0x660099));
    }

    #[test]
    fn unknown_color_returns_none() {
        assert_eq!(color_name_to_code("Purple"), None);
        assert_eq!(color_name_to_code("Yellow"), None);
        assert_eq!(color_name_to_code(""), None);
        assert_eq!(color_name_to_code("Teal"), None);
        assert_eq!(color_name_to_code("Pink"), None);
        assert_eq!(color_name_to_code("Magenta"), None);
        assert_eq!(color_name_to_code("Olive"), None);
    }

    #[test]
    fn canonical_casing_normalizes() {
        assert_eq!(canonical_casing("rose"), Some("Rose"));
        assert_eq!(canonical_casing("RED"), Some("Red"));
        assert_eq!(canonical_casing("green"), Some("Green"));
        assert_eq!(canonical_casing("TURQUOISE"), Some("Turquoise"));
        assert_eq!(canonical_casing("Purple"), None);
    }

    #[test]
    fn is_valid_color_works() {
        assert!(is_valid_color("Rose"));
        assert!(is_valid_color("rose"));
        assert!(is_valid_color("RED"));
        assert!(is_valid_color("Lemon"));
        assert!(is_valid_color("Turquoise"));
        assert!(is_valid_color("Violet"));
        assert!(!is_valid_color("Purple"));
        assert!(!is_valid_color("Pink"));
        assert!(!is_valid_color(""));
    }

    #[test]
    fn colors_sorted() {
        for w in COLORS.windows(2) {
            assert!(
                w[0].0 <= w[1].0,
                "COLORS not sorted: {:?} > {:?}",
                w[0].0,
                w[1].0
            );
        }
    }
}
