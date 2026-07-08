use std::sync::OnceLock;

use regex::Regex;

/// Kind of a verifiable claim extracted from memory content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimKind {
    Path,
    Command,
}

impl ClaimKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Command => "command",
        }
    }
}

/// One mechanically verifiable claim: a filesystem path or a command name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub text: String,
    pub kind: ClaimKind,
    /// Claims inside backtick code spans follow the memory-quality convention;
    /// plain-text claims are extracted heuristically and may be noisier.
    pub backticked: bool,
}

/// Extraction result: verifiable claims plus backtick spans that name
/// something concrete but cannot be checked mechanically.
#[derive(Debug, Default)]
pub struct ExtractedClaims {
    pub claims: Vec<Claim>,
    pub unverifiable: Vec<String>,
}

fn span_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"`([^`\n]+)`").expect("invalid span pattern"))
}

/// Extract path and command claims from memory content. Backtick code spans
/// are the primary source (per the memory-quality convention that Action lines
/// carry exact commands/paths); bare path-like tokens in plain text are picked
/// up as a fallback so unconventional memories still get checked.
pub fn extract_claims(content: &str) -> ExtractedClaims {
    let mut extracted = ExtractedClaims::default();
    for capture in span_pattern().captures_iter(content) {
        claims_from_span(capture[1].trim(), &mut extracted);
    }
    let plain = span_pattern().replace_all(content, " ");
    for token in plain_path_tokens(&plain) {
        push_claim(&mut extracted.claims, token, ClaimKind::Path, false);
    }
    extracted
}

fn claims_from_span(span: &str, extracted: &mut ExtractedClaims) {
    if span.is_empty() {
        return;
    }
    let tokens: Vec<&str> = span.split_whitespace().map(trim_token).collect();
    let mut verifiable = false;
    if let Some((first, rest)) = tokens.split_first() {
        if is_path_like(first) {
            push_claim(
                &mut extracted.claims,
                first.to_string(),
                ClaimKind::Path,
                true,
            );
            verifiable = true;
        } else if !rest.is_empty() && is_command_word(first) {
            push_claim(
                &mut extracted.claims,
                first.to_string(),
                ClaimKind::Command,
                true,
            );
            verifiable = true;
        }
        for token in rest {
            if is_path_like(token) {
                push_claim(
                    &mut extracted.claims,
                    token.to_string(),
                    ClaimKind::Path,
                    true,
                );
                verifiable = true;
            }
        }
    }
    if !verifiable
        && !extracted
            .unverifiable
            .iter()
            .any(|existing| existing == span)
    {
        extracted.unverifiable.push(span.to_string());
    }
}

fn push_claim(claims: &mut Vec<Claim>, text: String, kind: ClaimKind, backticked: bool) {
    if claims
        .iter()
        .any(|claim| claim.text == text && claim.kind == kind)
    {
        return;
    }
    claims.push(Claim {
        text,
        kind,
        backticked,
    });
}

fn trim_token(token: &str) -> &str {
    token
        .trim_matches(|ch| matches!(ch, '"' | '\'' | ',' | ';' | '(' | ')'))
        .trim_end_matches('.')
}

/// A token is a checkable path when it has directory structure and is free of
/// scheme/scope/email punctuation (`https://…`, `project:owner/repo`, `a@b`).
fn is_path_like(token: &str) -> bool {
    if token.len() < 2 || token.contains(':') || token.contains('@') {
        return false;
    }
    token.contains('/') || token == "~" || token.starts_with("~/")
}

fn is_command_word(token: &str) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '+' | '-'))
}

fn plain_path_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut run = String::new();
    let mut previous: Option<char> = None;
    let mut run_start_previous: Option<char> = None;
    for ch in text.chars() {
        if is_path_char(ch) {
            if run.is_empty() {
                run_start_previous = previous;
            }
            run.push(ch);
        } else {
            finish_plain_run(&mut run, run_start_previous, &mut tokens);
        }
        previous = Some(ch);
    }
    finish_plain_run(&mut run, run_start_previous, &mut tokens);
    tokens
}

fn is_path_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '~' | '/' | '<' | '>' | '-' | '+')
}

fn finish_plain_run(run: &mut String, preceding: Option<char>, tokens: &mut Vec<String>) {
    if run.is_empty() {
        return;
    }
    let token = std::mem::take(run).trim_end_matches('.').to_string();
    // Runs directly after ':' are scope strings or URL remainders, not paths.
    if preceding == Some(':') {
        return;
    }
    if !is_path_like(&token) || !token.contains('/') {
        return;
    }
    if token.chars().all(|ch| matches!(ch, '/' | '.')) {
        return;
    }
    // A leading-slash run with a single segment is a slash command (`/plan`,
    // `/experts`), not an absolute path; real absolute paths have more depth.
    if token.starts_with('/') && token.matches('/').count() < 2 {
        return;
    }
    if !tokens.contains(&token) {
        tokens.push(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim_texts(extracted: &ExtractedClaims, kind: ClaimKind) -> Vec<&str> {
        extracted
            .claims
            .iter()
            .filter(|claim| claim.kind == kind)
            .map(|claim| claim.text.as_str())
            .collect()
    }

    #[test]
    fn extracts_command_and_path_from_command_span() {
        let extracted =
            extract_claims("Action: 執行 `mem save --content-file templates/workflow.yaml`");
        assert_eq!(claim_texts(&extracted, ClaimKind::Command), vec!["mem"]);
        assert_eq!(
            claim_texts(&extracted, ClaimKind::Path),
            vec!["templates/workflow.yaml"]
        );
    }

    #[test]
    fn extracts_backticked_path_span() {
        let extracted = extract_claims("檢查 `crates/mem-cli/src/args.rs` 的定義");
        let paths = claim_texts(&extracted, ClaimKind::Path);
        assert_eq!(paths, vec!["crates/mem-cli/src/args.rs"]);
        assert!(extracted.claims[0].backticked);
    }

    #[test]
    fn keeps_placeholder_paths() {
        let extracted = extract_claims("到 `.claude/commands/experts/<domain>/` 找答案");
        assert_eq!(
            claim_texts(&extracted, ClaimKind::Path),
            vec![".claude/commands/experts/<domain>/"]
        );
    }

    #[test]
    fn rejects_scopes_urls_and_emails() {
        let extracted = extract_claims(
            "scope 用 project:disler/agent-experts，見 https://example.com/docs/page，寄給 a@b.co",
        );
        assert!(
            extracted.claims.is_empty(),
            "claims: {:?}",
            extracted.claims
        );
    }

    #[test]
    fn extracts_plain_text_paths_as_non_backticked() {
        let extracted = extract_claims(
            "參考 apps/orchestrator_3_stream/backend/modules/autocomplete_agent.py 搭配狀態檔",
        );
        assert_eq!(
            claim_texts(&extracted, ClaimKind::Path),
            vec!["apps/orchestrator_3_stream/backend/modules/autocomplete_agent.py"]
        );
        assert!(!extracted.claims[0].backticked);
    }

    #[test]
    fn single_word_span_is_unverifiable() {
        let extracted = extract_claims("回傳 `duplicate_found` 時停下，或 `type=workflow` 標記");
        assert!(extracted.claims.is_empty());
        assert_eq!(
            extracted.unverifiable,
            vec!["duplicate_found", "type=workflow"]
        );
    }

    #[test]
    fn skips_slash_commands_in_plain_text() {
        let extracted = extract_claims("人工執行 /plan 或 /experts:web:self-improve 的版本");
        assert!(
            extracted.claims.is_empty(),
            "claims: {:?}",
            extracted.claims
        );
        let absolute = extract_claims("store 在 /Users/neo/.mnemark 底下");
        assert_eq!(
            claim_texts(&absolute, ClaimKind::Path),
            vec!["/Users/neo/.mnemark"]
        );
    }

    #[test]
    fn deduplicates_claims_across_spans_and_plain_text() {
        let extracted = extract_claims("`scripts/build.sh` 之後再跑一次 scripts/build.sh 確認");
        assert_eq!(
            claim_texts(&extracted, ClaimKind::Path),
            vec!["scripts/build.sh"]
        );
    }

    #[test]
    fn expands_home_style_paths() {
        let extracted = extract_claims("binary 在 `~/.cargo/bin/mem`");
        assert_eq!(
            claim_texts(&extracted, ClaimKind::Path),
            vec!["~/.cargo/bin/mem"]
        );
    }
}
