use std::collections::HashSet;

pub fn content_similarity(a: &str, b: &str) -> f64 {
    let a_set = shingles(&normalized_text(a));
    let b_set = shingles(&normalized_text(b));
    if a_set.is_empty() || b_set.is_empty() {
        return 0.0;
    }
    let intersection = a_set.intersection(&b_set).count() as f64;
    let union = a_set.union(&b_set).count() as f64;
    let smaller = a_set.len().min(b_set.len()) as f64;
    (intersection / union).max(intersection / smaller)
}

pub fn normalized_text(input: &str) -> String {
    input
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

pub fn remote_to_scope(remote: &str) -> String {
    let cleaned = remote
        .trim_end_matches(".git")
        .replace("git@github.com:", "")
        .replace("https://github.com/", "");
    if cleaned.contains('/') {
        format!("project:{cleaned}")
    } else {
        "global".to_string()
    }
}

fn shingles(input: &str) -> HashSet<String> {
    let chars: Vec<char> = input.chars().collect();
    if chars.is_empty() {
        return HashSet::new();
    }
    if chars.len() <= 2 {
        return HashSet::from([input.to_string()]);
    }
    chars
        .windows(2)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_similarity_handles_cjk_overlap() {
        let score = content_similarity("不要使用 emoji", "不要在回覆中使用 emoji");
        assert!(score > 0.8);
    }

    #[test]
    fn remote_scope_supports_ssh_and_https() {
        assert_eq!(
            remote_to_scope("git@github.com:NeoHsu/mnemark.git"),
            "project:NeoHsu/mnemark"
        );
        assert_eq!(
            remote_to_scope("https://github.com/NeoHsu/mnemark.git"),
            "project:NeoHsu/mnemark"
        );
    }
}
