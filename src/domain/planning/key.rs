//! Musical-key parsing, Camelot conversion, and pitch-shift helpers.

use super::{AxisScore, CamelotKey};

pub(crate) fn key_to_camelot(raw_key: &str) -> Option<CamelotKey> {
    parse_camelot_key(raw_key).or_else(|| musical_key_to_camelot(raw_key))
}

pub(crate) fn parse_camelot_key(raw_key: &str) -> Option<CamelotKey> {
    let trimmed = raw_key.trim().to_ascii_uppercase();
    if trimmed.len() < 2 {
        return None;
    }
    let (number, letter_str) = trimmed.split_at(trimmed.len() - 1);
    let letter = letter_str.chars().next()?;
    if letter != 'A' && letter != 'B' {
        return None;
    }
    let number: u8 = number.parse().ok()?;
    if !(1..=12).contains(&number) {
        return None;
    }
    Some(CamelotKey { number, letter })
}

pub(crate) fn musical_key_to_camelot(raw_key: &str) -> Option<CamelotKey> {
    let normalized = raw_key.trim().replace('♯', "#").replace('♭', "b");
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();

    let (root_raw, is_minor) = if lower.ends_with("minor") && normalized.len() > 5 {
        (&normalized[..normalized.len() - 5], true)
    } else if lower.ends_with("min") && normalized.len() > 3 {
        (&normalized[..normalized.len() - 3], true)
    } else if lower.ends_with('m') && normalized.len() > 1 {
        (&normalized[..normalized.len() - 1], true)
    } else if lower.ends_with("major") && normalized.len() > 5 {
        (&normalized[..normalized.len() - 5], false)
    } else if lower.ends_with("maj") && normalized.len() > 3 {
        (&normalized[..normalized.len() - 3], false)
    } else {
        (normalized.as_str(), false)
    };
    let root = normalize_key_root(root_raw)?;

    let (number, letter) = if is_minor {
        match root.as_str() {
            "G#" | "Ab" => (1, 'A'),
            "D#" | "Eb" => (2, 'A'),
            "A#" | "Bb" => (3, 'A'),
            "F" => (4, 'A'),
            "C" => (5, 'A'),
            "G" => (6, 'A'),
            "D" => (7, 'A'),
            "A" => (8, 'A'),
            "E" => (9, 'A'),
            "B" => (10, 'A'),
            "F#" | "Gb" => (11, 'A'),
            "C#" | "Db" => (12, 'A'),
            _ => return None,
        }
    } else {
        match root.as_str() {
            "B" => (1, 'B'),
            "F#" | "Gb" => (2, 'B'),
            "C#" | "Db" => (3, 'B'),
            "G#" | "Ab" => (4, 'B'),
            "D#" | "Eb" => (5, 'B'),
            "A#" | "Bb" => (6, 'B'),
            "F" => (7, 'B'),
            "C" => (8, 'B'),
            "G" => (9, 'B'),
            "D" => (10, 'B'),
            "A" => (11, 'B'),
            "E" => (12, 'B'),
            _ => return None,
        }
    };
    Some(CamelotKey { number, letter })
}

fn normalize_key_root(root: &str) -> Option<String> {
    let stripped: String = root.chars().filter(|ch| !ch.is_whitespace()).collect();
    if stripped.is_empty() {
        return None;
    }
    let mut chars = stripped.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    if !matches!(letter, 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G') {
        return None;
    }
    let accidental = chars.next();
    if chars.next().is_some() {
        return None;
    }
    match accidental {
        Some('#') => Some(format!("{letter}#")),
        Some('b') | Some('B') => Some(format!("{letter}b")),
        Some(_) => None,
        None => Some(letter.to_string()),
    }
}

pub(crate) fn format_camelot(key: CamelotKey) -> String {
    format!("{}{}", key.number, key.letter)
}

pub(crate) fn transpose_camelot_key(key: CamelotKey, semitones: i32) -> CamelotKey {
    let steps = ((semitones % 12) * 7).rem_euclid(12);
    let new_number = ((key.number as i32 - 1 + steps) % 12 + 1) as u8;
    CamelotKey {
        number: new_number,
        letter: key.letter,
    }
}

fn bracketed_keys(key: CamelotKey, exact_shift: f64) -> [(CamelotKey, f64); 2] {
    if !exact_shift.is_finite() || exact_shift.abs() < 0.01 {
        return [(key, 1.0), (key, 0.0)];
    }
    let floor_s = exact_shift.floor() as i32;
    let ceil_s = exact_shift.ceil() as i32;
    if floor_s == ceil_s {
        let transposed = transpose_camelot_key(key, floor_s);
        return [(transposed, 1.0), (transposed, 0.0)];
    }
    let fraction = exact_shift - floor_s as f64;
    [
        (transpose_camelot_key(key, floor_s), 1.0 - fraction),
        (transpose_camelot_key(key, ceil_s), fraction),
    ]
}

pub(crate) fn score_key_with_pitch_shifts(
    from: Option<CamelotKey>,
    to: Option<CamelotKey>,
    from_shift: f64,
    to_shift: f64,
) -> AxisScore {
    let Some(from_key) = from else {
        return super::score_key_axis(from, to);
    };
    let Some(to_key) = to else {
        return super::score_key_axis(from, to);
    };
    if from_shift.abs() < 0.01 && to_shift.abs() < 0.01 {
        return super::score_key_axis(Some(from_key), Some(to_key));
    }

    let from_keys = bracketed_keys(from_key, from_shift);
    let to_keys = bracketed_keys(to_key, to_shift);
    let mut blended = 0.0;
    let mut best_label = String::new();
    let mut best_weight = 0.0_f64;
    for &(from_t, from_w) in &from_keys {
        for &(to_t, to_w) in &to_keys {
            let weight = from_w * to_w;
            if weight < 1e-6 {
                continue;
            }
            let score = super::score_key_axis(Some(from_t), Some(to_t));
            blended += weight * score.value;
            if weight > best_weight {
                best_weight = weight;
                best_label = score.label;
            }
        }
    }
    let from_cents = (from_shift - from_shift.round()).abs() * 100.0;
    let to_cents = (to_shift - to_shift.round()).abs() * 100.0;
    let max_cents = from_cents.max(to_cents);
    let label = if max_cents > 10.0 {
        format!("{best_label} (~{max_cents:.0}¢ detuned)")
    } else {
        best_label
    };
    AxisScore {
        value: blended,
        label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_key_parses_camelot_without_transport_types() {
        assert_eq!(format_camelot(parse_camelot_key("8A").unwrap()), "8A");
    }
}
