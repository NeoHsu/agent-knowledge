use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;

use crate::atomic_file::atomic_write;
use crate::error;

const MAX_SECRET_SCAN_FILE_BYTES: u64 = 134_217_728;

/// Compiled regexes for secret stripping, built once and reused.
fn secret_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let raw = [
            // OpenAI-style keys
            r"sk-[A-Za-z0-9_\-]{16,}",
            // GitHub personal access tokens (classic and fine-grained)
            r"gh[opusr]_[A-Za-z0-9_]{16,}",
            r"github_pat_[A-Za-z0-9_]{20,}",
            // GitLab personal access tokens
            r"glpat-[A-Za-z0-9_\-]{16,}",
            // Anthropic and Stripe-style API keys
            r"sk-ant-[A-Za-z0-9_\-]{16,}",
            r"(?i)sk_(live|test)_[A-Za-z0-9]{16,}",
            // Google API keys
            r"AIza[0-9A-Za-z_\-]{20,}",
            // npm access tokens
            r"npm_[A-Za-z0-9]{20,}",
            // Slack bot tokens
            r"xoxb-[A-Za-z0-9\-]{16,}",
            // AWS access key IDs
            r"AKIA[0-9A-Z]{16}",
            // Bearer tokens (Authorization header style)
            r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}",
            // Plain password/secret assignments. JSON is handled separately
            // so redaction preserves valid object syntax.
            r"(?i)(password|passwd|secret|client_secret|private_key|aws_secret_access_key)\s*[=:]\s*[^ \n\r]+",
            // Credentials embedded in URLs
            r"(?i)[a-z][a-z0-9+.-]*://[^\s/:]+:[^\s/@]+@[^\s]+",
            // Generic API/access-key assignments
            r"(?i)(api_key|apikey|access_key|access_token)\s*[=:]\s*[^ \n\r]+",
            // token: without Bearer prefix (Fix S1)
            r"(?i)token\s*:\s*[A-Za-z0-9._\-]{8,}",
            // JWT tokens: three dot-separated base64url segments (Fix S1)
            r"eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+",
            // PEM private key blocks (Fix S1)
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        ];
        raw.iter()
            .map(|p| Regex::new(p).expect("invalid secret pattern"))
            .collect()
    })
}

fn json_secret_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)([\"']?(password|passwd|secret|client_secret|private_key|api_key|apikey|access_key|access_token|token)[\"']?\s*:\s*)[\"'][^\"'\r\n]+[\"']"#,
        )
        .expect("invalid JSON secret pattern")
    })
}

pub fn strip_secrets(input: &str) -> Result<String> {
    let mut output = json_secret_pattern()
        .replace_all(input, "$1\"[REDACTED]\"")
        .to_string();
    for re in secret_patterns() {
        output = re.replace_all(&output, "[REDACTED]").to_string();
    }
    Ok(output)
}

pub fn sanitize_secret_field(input: &str, field: &str, allow_redaction: bool) -> Result<String> {
    let redacted = strip_secrets(input)?;
    if redacted != input && !allow_redaction {
        return Err(error::safety_violation(format!(
            "secret-like value detected in {field}; write rejected without exposing the value. \
             Remove the secret or pass --redact-secrets explicitly"
        )));
    }
    Ok(redacted)
}

/// Validate a text or binary file for secret-like values and optionally redact
/// UTF-8 text in place. Binary data is never rewritten because doing so could
/// silently corrupt an executable or archive.
pub fn sanitize_secret_file(path: &Path, field: &str, allow_redaction: bool) -> Result<bool> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(error::safety_violation(format!(
            "refusing to scan non-regular {field}: {}",
            path.display()
        )));
    }
    let file_bytes = metadata.len();
    if file_bytes > MAX_SECRET_SCAN_FILE_BYTES {
        return Err(error::safety_violation(format!(
            "{field} exceeds the {MAX_SECRET_SCAN_FILE_BYTES}-byte secret-scan limit"
        )));
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    match String::from_utf8(bytes) {
        Ok(text) => {
            let sanitized = sanitize_secret_field(&text, field, allow_redaction)?;
            if sanitized == text {
                return Ok(false);
            }
            atomic_write(path, sanitized.as_bytes())
                .with_context(|| format!("redact {}", path.display()))?;
            Ok(true)
        }
        Err(error) => {
            let bytes = error.into_bytes();
            let text = String::from_utf8_lossy(&bytes);
            if strip_secrets(&text)? != text {
                return Err(error::safety_violation(format!(
                    "secret-like value detected in {field}; binary files cannot be redacted safely"
                )));
            }
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_common_secret_patterns() {
        let token = ["abcdefgh", "ijklmnop"].concat();
        let password = ["hun", "ter2"].concat();
        let input = format!("token=Bearer {token} password={password}");
        let stripped = strip_secrets(&input).expect("strip secrets");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains(&password));
    }

    #[test]
    fn strips_api_key_patterns() {
        let first = ["abc123", "supersecret"].concat();
        let second = ["xyzXYZ", "9876543210"].concat();
        let first_label = ["api", "_key"].concat();
        let second_label = ["api", "key"].concat();
        let input = format!("{first_label}={first} {second_label}: {second}");
        let stripped = strip_secrets(&input).expect("strip API keys");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains(&first));
    }

    #[test]
    fn redacts_json_secret_values_without_breaking_json() {
        let password = ["hun", "ter2"].concat();
        let input = serde_json::json!({"password": password, "safe": "value"}).to_string();
        let stripped = strip_secrets(&input).expect("strip JSON secret");
        let parsed: serde_json::Value =
            serde_json::from_str(&stripped).expect("parse redacted JSON");
        assert_eq!(parsed["password"], "[REDACTED]");
        assert_eq!(parsed["safe"], "value");
    }

    #[test]
    fn strips_token_without_bearer() {
        let token = ["abcdefgh", "ijklmnop"].concat();
        let stripped = strip_secrets(&format!("token: {token}")).expect("strip token");
        assert!(stripped.contains("[REDACTED]"));
    }

    #[test]
    fn strips_jwt_token() {
        let jwt = [
            ["eyJhbGci", "OiJIUzI1NiJ9"].concat(),
            ["eyJzdWIi", "OiJ1c2VyIn0"].concat(),
            ["SflKxwRJSMeKKF2Q", "T4fwpMeJf36POk6yJV_adQssw5c"].concat(),
        ]
        .join(".");
        let stripped = strip_secrets(&jwt).expect("strip JWT");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("eyJ"));
    }

    #[test]
    fn strips_pem_private_key() {
        let begin = ["-----BEGIN RSA ", "PRIVATE KEY-----"].concat();
        let end = ["-----END RSA ", "PRIVATE KEY-----"].concat();
        let body = ["MIIEowIB", "AAKCAQEA"].concat();
        let pem = format!("{begin}\n{body}\n{end}");
        let stripped = strip_secrets(&pem).expect("strip PEM");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains(&body));
    }

    #[test]
    fn strips_every_supported_credential_family() {
        // Assemble all synthetic values at runtime so repository secret
        // scanners do not mistake test fixtures for live credentials.
        let samples = [
            ["sk-", "placeholder1234567890"].concat(),
            ["ghp_", "placeholder1234567890"].concat(),
            ["github_", "pat_", "placeholder1234567890"].concat(),
            ["glpat-", "placeholder1234567890"].concat(),
            ["sk-ant-", "placeholder1234567890"].concat(),
            ["sk_", "live_", "placeholder1234567890"].concat(),
            ["AI", "za", "0123456789abcdefghijklmnop"].concat(),
            ["npm_", "placeholder1234567890123456"].concat(),
            ["xoxb-", "placeholder-1234567890123456"].concat(),
            ["Bearer ", "placeholder.token-1234567890"].concat(),
            [
                "https://user:",
                "placeholder-password@example.invalid/private",
            ]
            .concat(),
            ["access_", "token=", "placeholder1234567890"].concat(),
            ["AK", "IA", "0123456789ABCDEF"].concat(),
        ];
        for sample in &samples {
            let stripped = strip_secrets(sample)
                .unwrap_or_else(|error| panic!("strip credential family: {error}"));
            assert_ne!(&stripped, sample, "credential family was not detected");
            assert!(stripped.contains("[REDACTED]"));
        }
    }

    #[test]
    fn ordinary_operational_text_is_not_redacted() {
        let text =
            "Run mem doctor, verify project:example/app, then inspect the release checklist.";
        assert_eq!(strip_secrets(text).expect("scan safe text"), text);
    }
}
