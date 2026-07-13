use unicode_normalization::UnicodeNormalization;

/// Normalize a string for matching: NFC normalize, lowercase, keep alphanumeric
/// chars and spaces, strip other characters.
///
/// NFC normalization ensures that combining mark sequences (e.g. `e` + `\u{0301}`)
/// are composed into single codepoints (e.g. `é`) before the alphanumeric filter
/// runs.  Without this, NFD input (common on macOS) would lose diacritics.
pub fn normalize_for_matching(input: &str) -> String {
    input
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_lowercasing() {
        assert_eq!(normalize_for_matching("Hello World"), "hello world");
    }

    #[test]
    fn strips_special_characters() {
        assert_eq!(normalize_for_matching("Drum & Bass"), "drum  bass");
        assert_eq!(normalize_for_matching("AC/DC"), "acdc");
        assert_eq!(normalize_for_matching("R&B"), "rb");
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(normalize_for_matching("  Techno  "), "techno");
    }

    #[test]
    fn empty_and_whitespace_only() {
        assert_eq!(normalize_for_matching(""), "");
        assert_eq!(normalize_for_matching("   "), "");
        assert_eq!(normalize_for_matching("&-!"), "");
    }

    #[test]
    fn unicode_accented_characters() {
        assert_eq!(normalize_for_matching("Señorita"), "señorita");
        assert_eq!(normalize_for_matching("Aníbal"), "aníbal");
        assert_eq!(normalize_for_matching("Björk"), "björk");
    }

    #[test]
    fn nfc_nfd_produce_same_result() {
        // NFC: ñ as single codepoint U+00F1
        let nfc = "Se\u{00F1}orita";
        // NFD: n + combining tilde U+0303
        let nfd = "Sen\u{0303}orita";
        assert_ne!(nfc, nfd); // different byte sequences
        assert_eq!(normalize_for_matching(nfc), normalize_for_matching(nfd));
    }

    #[test]
    fn nfc_nfd_accented_e() {
        let nfc = "\u{00E9}"; // é precomposed
        let nfd = "e\u{0301}"; // e + combining acute
        assert_eq!(normalize_for_matching(nfc), normalize_for_matching(nfd));
    }
}
