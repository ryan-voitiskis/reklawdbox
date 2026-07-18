use std::collections::HashSet;

use super::*;

#[test]
fn all_genre_token_targets_are_canonical() {
    for &(token, genres) in GENRE_TOKENS {
        for &g in genres {
            assert!(
                canonical_genre_name(g).is_some(),
                "GENRE_TOKENS entry '{token}' references non-canonical genre '{g}'"
            );
        }
    }
}

#[test]
fn taxonomy_sorted() {
    let mut sorted = GENRES.to_vec();
    sorted.sort_by_key(|a| a.to_lowercase());
    assert_eq!(
        GENRES,
        sorted.as_slice(),
        "GENRES array must be sorted alphabetically"
    );
}

#[test]
fn taxonomy_has_reasonable_size() {
    assert!(
        GENRES.len() >= 30,
        "taxonomy seems too small: {}",
        GENRES.len()
    );
}

#[test]
fn known_genre_case_insensitive() {
    assert!(is_known_genre("deep house"));
    assert!(is_known_genre("Deep House"));
    assert!(is_known_genre("TECHNO"));
    assert!(is_known_genre("uk funky"));
    assert!(is_known_genre("R&B"));
    let removed_genre = ["Drone", "Techno"].join(" ");
    assert!(!is_known_genre(&removed_genre));
    assert!(!is_known_genre("Polka"));
}

#[test]
fn known_genre_trims_whitespace() {
    assert!(is_known_genre(" Techno"));
    assert!(is_known_genre("Techno "));
    assert!(is_known_genre("\tDeep House\t"));
}

#[test]
fn normalize_known_aliases() {
    assert_eq!(canonical_genre_from_alias("Hip-Hop"), Some("Hip Hop"));
    assert_eq!(canonical_genre_from_alias("DnB"), Some("Drum & Bass"));
    assert_eq!(canonical_genre_from_alias("Terror"), Some("Hardcore"));
    assert_eq!(canonical_genre_from_alias("Uptempo"), Some("Hardcore"));
    assert_eq!(
        canonical_genre_from_alias("UK / Happy Hardcore"),
        Some("Happy Hardcore")
    );
}

#[test]
fn normalize_case_insensitive() {
    assert_eq!(canonical_genre_from_alias("hip-hop"), Some("Hip Hop"));
    assert_eq!(canonical_genre_from_alias("HIP-HOP"), Some("Hip Hop"));
    assert_eq!(canonical_genre_from_alias("Hip-Hop"), Some("Hip Hop"));
    assert_eq!(canonical_genre_from_alias("dnb"), Some("Drum & Bass"));
    assert_eq!(canonical_genre_from_alias("DNB"), Some("Drum & Bass"));
}

#[test]
fn normalize_trims_whitespace() {
    assert_eq!(canonical_genre_from_alias(" hip-hop"), Some("Hip Hop"));
    assert_eq!(canonical_genre_from_alias("HIP-HOP "), Some("Hip Hop"));
    assert_eq!(canonical_genre_from_alias("\tdnb\t"), Some("Drum & Bass"));
}

#[test]
fn normalize_canonical_returns_none() {
    assert_eq!(canonical_genre_from_alias("Techno"), None);
    assert_eq!(canonical_genre_from_alias("Deep House"), None);
    assert_eq!(canonical_genre_from_alias("Drum & Bass"), None);
    assert_eq!(canonical_genre_from_alias("Hip Hop"), None);
    assert_eq!(canonical_genre_from_alias("Rock"), None);
    assert_eq!(canonical_genre_from_alias("Pop"), None);
}

#[test]
fn normalize_unknown_returns_none() {
    assert_eq!(canonical_genre_from_alias("Polka"), None);
    assert_eq!(canonical_genre_from_alias("Anti-music"), None);
    assert_eq!(canonical_genre_from_alias("Zydeco"), None);
}

#[test]
fn alias_map_not_empty() {
    let aliases = genre_alias_map();
    assert!(
        aliases.len() >= 26,
        "expected at least 26 aliases, got {}",
        aliases.len()
    );
}

#[test]
fn aliases_sorted() {
    for w in ALIASES.windows(2) {
        assert!(
            w[0].0 <= w[1].0,
            "ALIASES not sorted: {:?} > {:?}",
            w[0].0,
            w[1].0
        );
    }
}

#[test]
fn aliases_are_lowercase_and_casefold_unique() {
    let mut seen = HashSet::new();
    for &(alias, _) in ALIASES {
        assert!(alias.is_ascii(), "alias '{alias}' must be ASCII");
        assert_eq!(
            alias,
            alias.to_ascii_lowercase(),
            "alias '{alias}' must be lowercase ASCII"
        );
        let inserted = seen.insert(alias.to_ascii_lowercase());
        assert!(inserted, "duplicate alias key '{alias}' (case-insensitive)");
    }
}

#[test]
fn non_ascii_aliases_are_rejected() {
    let result = std::panic::catch_unwind(|| {
        let _ = build_alias_map(&[("Électro", "Electro")]);
    });
    assert!(result.is_err(), "expected non-ASCII alias to panic");
}

#[test]
fn all_alias_targets_are_canonical() {
    for &(alias, canonical) in ALIASES {
        assert!(
            is_known_genre(canonical),
            "alias '{alias}' maps to '{canonical}' which is not in taxonomy"
        );
    }
}

#[test]
fn no_alias_shadows_canonical() {
    for &(alias, target) in ALIASES {
        assert!(
            !is_known_genre(alias),
            "alias '{alias}' (-> '{target}') shadows a canonical genre — remove it"
        );
    }
}

#[test]
fn all_taxonomy_genres_have_family() {
    assert_eq!(genre_family("House"), GenreFamily::House);
    assert_eq!(genre_family("Deep House"), GenreFamily::House);
    assert_eq!(genre_family("Techno"), GenreFamily::Techno);
    assert_eq!(genre_family("Hard Techno"), GenreFamily::Techno);
    assert_eq!(genre_family("Trance"), GenreFamily::Techno);
    assert_eq!(genre_family("Psytrance"), GenreFamily::Techno);
    assert_eq!(genre_family("Drum & Bass"), GenreFamily::Bass);
    assert_eq!(genre_family("Dubstep"), GenreFamily::Bass);
    assert_eq!(genre_family("Hardcore"), GenreFamily::Hardcore);
    assert_eq!(genre_family("Gabber"), GenreFamily::Hardcore);
    assert_eq!(genre_family("Hardstyle"), GenreFamily::Hardcore);
    assert_eq!(genre_family("Happy Hardcore"), GenreFamily::Hardcore);
    assert_eq!(genre_family("Hard Trance"), GenreFamily::Hardcore);
    assert_eq!(genre_family("Ambient"), GenreFamily::Downtempo);
    assert_eq!(genre_family("Downtempo"), GenreFamily::Downtempo);

    // Every taxonomy genre must resolve without panicking
    for g in GENRES {
        let _ = genre_family(g);
    }
}

#[test]
fn bpm_ranges_are_valid() {
    for g in GENRES {
        if let Some(range) = genre_bpm_range(g) {
            assert!(
                range.typical_min < range.typical_max,
                "genre '{}' has invalid BPM range: {} >= {}",
                g,
                range.typical_min,
                range.typical_max
            );
            assert!(
                range.typical_min > 0.0,
                "genre '{}' has non-positive typical_min: {}",
                g,
                range.typical_min
            );
        }
    }
}

#[test]
fn token_extraction_specific_match() {
    let tokens = extract_genre_tokens("Electronic Techno");
    assert_eq!(tokens, vec!["Techno"]);
}

#[test]
fn token_extraction_compound_string() {
    let mut tokens = extract_genre_tokens("Electro Chill Out/Trip-Hop/Lounge");
    tokens.sort();
    assert!(tokens.contains(&"Downtempo"));
    assert!(tokens.contains(&"Electro"));
    assert!(tokens.contains(&"Trip-Hop"));
    assert_eq!(tokens.len(), 3);
}

#[test]
fn token_extraction_multi_word() {
    let tokens = extract_genre_tokens("Dance/Rap/Hip Hop");
    assert_eq!(tokens, vec!["Hip Hop"]);
}

#[test]
fn removed_genre_descriptor_decomposes_to_live_genres() {
    let removed_genre = ["Drone", "Techno"].join(" ");
    assert_eq!(
        extract_genre_tokens(&removed_genre),
        vec!["Ambient", "Techno"]
    );
}

#[test]
fn token_extraction_vague_returns_empty() {
    assert!(extract_genre_tokens("Electronica").is_empty());
    assert!(extract_genre_tokens("Electronic").is_empty());
    assert!(extract_genre_tokens("Anti-music").is_empty());
    assert!(extract_genre_tokens("").is_empty());
}

#[test]
fn token_extraction_skips_canonical() {
    assert!(extract_genre_tokens("Techno").is_empty());
    assert!(extract_genre_tokens("Deep House").is_empty());
}

#[test]
fn token_extraction_skips_aliases() {
    assert!(extract_genre_tokens("Hip-Hop").is_empty());
    assert!(extract_genre_tokens("DnB").is_empty());
}

#[test]
fn parenthetical_extracts_base_genre() {
    assert_eq!(
        extract_parenthetical_base("Techno (Peak Time / Driving)"),
        Some("Techno")
    );
    assert_eq!(
        extract_parenthetical_base("House (Progressive)"),
        Some("House")
    );
    // Base resolves through alias
    assert_eq!(
        extract_parenthetical_base("DnB (Liquid)"),
        Some("Drum & Bass")
    );
}

#[test]
fn parenthetical_returns_none_for_non_matching() {
    assert_eq!(extract_parenthetical_base("Techno"), None);
    assert_eq!(extract_parenthetical_base(""), None);
    assert_eq!(extract_parenthetical_base("(nothing before paren)"), None);
    assert_eq!(extract_parenthetical_base("Polka (Fast)"), None);
}

#[test]
fn label_genres_sorted() {
    for w in LABEL_GENRES.windows(2) {
        assert!(
            w[0].0 <= w[1].0,
            "LABEL_GENRES not sorted: {:?} > {:?}",
            w[0].0,
            w[1].0
        );
    }
}

#[test]
fn label_genres_are_lowercase() {
    for &(label, _) in LABEL_GENRES {
        assert!(label.is_ascii(), "label '{label}' must be ASCII");
        assert_eq!(
            label,
            label.to_ascii_lowercase(),
            "label '{label}' must be lowercase ASCII"
        );
    }
}

#[test]
fn all_label_genre_targets_are_canonical() {
    for &(label, canonical) in LABEL_GENRES {
        assert!(
            is_known_genre(canonical),
            "label '{label}' maps to '{canonical}' which is not in taxonomy"
        );
    }
}

#[test]
fn no_label_shadows_alias() {
    let alias_map = genre_alias_map();
    for &(label, _) in LABEL_GENRES {
        assert!(
            !alias_map.contains_key(label),
            "label '{label}' shadows an alias key"
        );
    }
}

#[test]
fn label_genre_exact_match() {
    assert_eq!(label_genre("mord"), Some("Hard Techno"));
    assert_eq!(label_genre("hospital"), Some("Drum & Bass"));
    assert_eq!(label_genre("kompakt"), Some("Minimal"));
}

#[test]
fn label_genre_suffix_stripping() {
    assert_eq!(label_genre("Tresor Records"), Some("Techno"));
    assert_eq!(label_genre("hospital records"), Some("Drum & Bass"));
    assert_eq!(label_genre("cocoon recordings"), Some("Techno"));
}

#[test]
fn label_genre_case_insensitive() {
    assert_eq!(label_genre("MORD"), Some("Hard Techno"));
    assert_eq!(label_genre("Hyperdub"), Some("Future Garage"));
    assert_eq!(label_genre("TRESOR"), Some("Techno"));
}

#[test]
fn suffix_stripped_labels_are_consistent() {
    let map = label_genre_map();
    for &(label, genre) in LABEL_GENRES {
        for suffix in LABEL_SUFFIXES {
            if let Some(prefix) = label.strip_suffix(suffix)
                && let Some(&prefix_genre) = map.get(prefix)
            {
                assert_eq!(
                    genre, prefix_genre,
                    "label '{label}' maps to '{genre}' but prefix '{prefix}' maps to '{prefix_genre}'"
                );
            }
        }
    }
}

#[test]
fn label_genre_unknown_returns_none() {
    assert_eq!(label_genre("warp"), None);
    assert_eq!(label_genre("xl recordings"), None);
    assert_eq!(label_genre(""), None);
}

#[test]
fn all_taxonomy_genres_have_depth() {
    for g in GENRES {
        let family = genre_family(g);
        let depth = genre_depth(g);
        if family != GenreFamily::Other {
            assert!(
                depth > 0,
                "genre '{g}' (family {family:?}) has depth 0 — add it to genre_depth()",
            );
        }
    }
}

#[test]
fn depth_ordering_house() {
    assert!(genre_depth("Deep House") > genre_depth("House"));
    assert!(genre_depth("House") > genre_depth("Disco"));
}

#[test]
fn depth_ordering_techno() {
    assert!(genre_depth("Deep Techno") > genre_depth("Techno"));
    assert!(genre_depth("Techno") > genre_depth("Hard Techno"));
    assert!(genre_depth("Ambient Techno") > genre_depth("Dub Techno"));
}

#[test]
fn depth_ordering_hardcore() {
    assert!(genre_depth("Gabber") > genre_depth("Hardcore"));
    assert!(genre_depth("Hardcore") > genre_depth("Hard Trance"));
    assert!(genre_depth("Hard Trance") > genre_depth("Happy Hardcore"));
    assert!(genre_depth("Happy Hardcore") > genre_depth("Hardstyle"));
}

#[test]
fn depth_ordering_bass() {
    assert!(genre_depth("Broken Beat") > genre_depth("Drum & Bass"));
    assert!(genre_depth("Dubstep") > genre_depth("Drum & Bass"));
}

#[test]
fn depth_ordering_downtempo() {
    assert!(genre_depth("Ambient") > genre_depth("Downtempo"));
    assert!(genre_depth("Downtempo") > genre_depth("IDM"));
}

#[test]
fn known_bpm_ranges() {
    let deep_techno = genre_bpm_range("Deep Techno").unwrap();
    assert_eq!(deep_techno.typical_min, 120.0);
    assert_eq!(deep_techno.typical_max, 132.0);

    let dnb = genre_bpm_range("Drum & Bass").unwrap();
    assert_eq!(dnb.typical_min, 168.0);
    assert_eq!(dnb.typical_max, 180.0);

    let hardcore = genre_bpm_range("Hardcore").unwrap();
    assert_eq!(hardcore.typical_min, 160.0);
    assert_eq!(hardcore.typical_max, 180.0);

    let gabber = genre_bpm_range("Gabber").unwrap();
    assert_eq!(gabber.typical_min, 160.0);
    assert_eq!(gabber.typical_max, 190.0);

    let hardstyle = genre_bpm_range("Hardstyle").unwrap();
    assert_eq!(hardstyle.typical_min, 148.0);
    assert_eq!(hardstyle.typical_max, 160.0);

    let happy_hc = genre_bpm_range("Happy Hardcore").unwrap();
    assert_eq!(happy_hc.typical_min, 165.0);
    assert_eq!(happy_hc.typical_max, 180.0);

    let hard_trance = genre_bpm_range("Hard Trance").unwrap();
    assert_eq!(hard_trance.typical_min, 138.0);
    assert_eq!(hard_trance.typical_max, 150.0);

    assert!(genre_bpm_range("IDM").is_none());
    assert!(genre_bpm_range("Experimental").is_none());
    assert!(genre_bpm_range("Jazz").is_none());
    assert!(genre_bpm_range("Polka").is_none());
}
