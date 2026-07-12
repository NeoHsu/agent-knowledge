use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use regex::Regex;

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
        bail!(
            "secret-like value detected in {field}; write rejected without exposing the value. \
             Remove the secret or pass --redact-secrets explicitly"
        );
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
        bail!("refusing to scan non-regular {field}: {}", path.display());
    }
    let file_bytes = metadata.len();
    if file_bytes > MAX_SECRET_SCAN_FILE_BYTES {
        bail!("{field} exceeds the {MAX_SECRET_SCAN_FILE_BYTES}-byte secret-scan limit");
    }
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    match String::from_utf8(bytes) {
        Ok(text) => {
            let sanitized = sanitize_secret_field(&text, field, allow_redaction)?;
            if sanitized == text {
                return Ok(false);
            }
            fs::write(path, sanitized).with_context(|| format!("redact {}", path.display()))?;
            Ok(true)
        }
        Err(error) => {
            let bytes = error.into_bytes();
            let text = String::from_utf8_lossy(&bytes);
            if strip_secrets(&text)? != text {
                bail!(
                    "secret-like value detected in {field}; binary files cannot be redacted safely"
                );
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
        let stripped =
            strip_secrets("token=Bearer abcdefghijklmnop password=hunter2").expect("strip secrets");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("hunter2"));
    }

    #[test]
    fn strips_api_key_patterns() {
        let stripped = strip_secrets("api_key=abc123supersecret apikey: xyzXYZ9876543210")
            .expect("strip API keys");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("abc123supersecret"));
    }

    #[test]
    fn redacts_json_secret_values_without_breaking_json() {
        let stripped =
            strip_secrets(r#"{"password":"hunter2","safe":"value"}"#).expect("strip JSON secret");
        let parsed: serde_json::Value =
            serde_json::from_str(&stripped).expect("parse redacted JSON");
        assert_eq!(parsed["password"], "[REDACTED]");
        assert_eq!(parsed["safe"], "value");
    }

    #[test]
    fn strips_token_without_bearer() {
        let stripped = strip_secrets("token: abcdefghijklmnop").expect("strip token");
        assert!(stripped.contains("[REDACTED]"));
    }

    #[test]
    fn strips_jwt_token() {
        let jwt = [
            "eyJhbGciOiJIUzI1NiJ9",
            "eyJzdWIiOiJ1c2VyIn0",
            "SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
        ]
        .join(".");
        let stripped = strip_secrets(&jwt).expect("strip JWT");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("eyJ"));
    }

    #[test]
    fn strips_pem_private_key() {
        let pem =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY-----";
        let stripped = strip_secrets(pem).expect("strip PEM");
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("MIIEowIBAAKCAQEA"));
    }
}
