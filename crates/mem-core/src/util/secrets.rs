use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;

/// Compiled regexes for secret stripping, built once and reused.
fn secret_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let raw = [
            // OpenAI-style keys
            r"sk-[A-Za-z0-9_\-]{16,}",
            // GitHub personal access tokens
            r"ghp_[A-Za-z0-9_]{16,}",
            // Slack bot tokens
            r"xoxb-[A-Za-z0-9\-]{16,}",
            // AWS access key IDs
            r"AKIA[0-9A-Z]{16}",
            // Bearer tokens (Authorization header style)
            r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}",
            // password= / secret= assignments
            r"(?i)(password|secret)\s*=\s*[^ \n\r]+",
            // Generic api_key= or apikey: (Fix S1)
            r"(?i)(api_key|apikey)\s*[=:]\s*[^ \n\r]+",
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

pub fn strip_secrets(input: &str) -> Result<String> {
    let mut output = input.to_string();
    for re in secret_patterns() {
        output = re.replace_all(&output, "[REDACTED]").to_string();
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_common_secret_patterns() {
        let stripped = strip_secrets("token=Bearer abcdefghijklmnop password=hunter2").unwrap();
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("hunter2"));
    }

    #[test]
    fn strips_api_key_patterns() {
        let stripped = strip_secrets("api_key=abc123supersecret apikey: xyzXYZ9876543210").unwrap();
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("abc123supersecret"));
    }

    #[test]
    fn strips_token_without_bearer() {
        let stripped = strip_secrets("token: abcdefghijklmnop").unwrap();
        assert!(stripped.contains("[REDACTED]"));
    }

    #[test]
    fn strips_jwt_token() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let stripped = strip_secrets(jwt).unwrap();
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("eyJ"));
    }

    #[test]
    fn strips_pem_private_key() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY-----";
        let stripped = strip_secrets(pem).unwrap();
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("MIIEowIBAAKCAQEA"));
    }
}
