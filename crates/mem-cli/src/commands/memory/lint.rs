use super::*;

/// Mechanical quality checks only; warnings never block a save. The goal is
/// to hold the memory-quality rules (one fact, absolute dates, tags) without
/// relying on the calling agent to remember them.
pub(super) fn lint_memory(r#type: &str, name: &str, content: &str, tags: &str) -> Vec<Value> {
    const RELATIVE_DATE_WORDS: &[&str] = &[
        "today",
        "yesterday",
        "tomorrow",
        "last week",
        "next week",
        "this week",
        "recently",
        "currently",
        "今天",
        "昨天",
        "明天",
        "上週",
        "上周",
        "下週",
        "下周",
        "本週",
        "本周",
        "最近",
        "目前",
    ];
    const VAGUE_NAMES: &[&str] = &["note", "notes", "misc", "temp", "todo", "memo", "important"];

    let mut warnings = Vec::new();
    if tags.trim() == "[]" && r#type != "reference" {
        warnings.push(json!({
            "code": "no_tags",
            "hint": "add 2-6 `type:value` tags so retrieval and retros can filter this memory"
        }));
    }
    if r#type != "workflow" && content.chars().count() > 1200 {
        warnings.push(json!({
            "code": "content_long",
            "hint": "content exceeds 1200 chars; split into one fact per memory"
        }));
    }
    if r#type != "workflow" {
        let lowered = content.to_lowercase();
        if let Some(word) = RELATIVE_DATE_WORDS
            .iter()
            .find(|word| lowered.contains(*word))
        {
            warnings.push(json!({
                "code": "relative_date_language",
                "hint": format!("content contains '{word}'; convert relative dates to absolute dates")
            }));
        }
    }
    if r#type != "workflow" {
        let extracted = extract_claims(content);
        if extracted.claims.iter().any(|claim| !claim.backticked) {
            warnings.push(json!({
                "code": "claims_outside_backticks",
                "hint": "content mentions paths outside backticks; wrap them in `...` so `mem reconcile` can verify them"
            }));
        }
    }
    let lowered_name = name.to_lowercase();
    if name.chars().count() < 3 || VAGUE_NAMES.contains(&lowered_name.as_str()) {
        warnings.push(json!({
            "code": "vague_name",
            "hint": "use a short, specific snake_case name that will stay stable"
        }));
    }
    warnings
}
