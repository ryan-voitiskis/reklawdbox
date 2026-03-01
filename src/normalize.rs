/// Normalize a string for matching: lowercase, keep alphanumeric chars and spaces, strip other characters.
pub fn normalize_for_matching(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect::<String>()
        .trim()
        .to_string()
}
