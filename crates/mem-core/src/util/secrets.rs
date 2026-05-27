use anyhow::Result;
use regex::Regex;

pub fn strip_secrets(input: &str) -> Result<String> {
    let patterns = [
        r"sk-[A-Za-z0-9_\-]{16,}",
        r"ghp_[A-Za-z0-9_]{16,}",
        r"xoxb-[A-Za-z0-9\-]{16,}",
        r"AKIA[0-9A-Z]{16}",
        r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}",
        r"(?i)(password|secret)\s*=\s*[^ \n\r]+",
    ];
    let mut output = input.to_string();
    for pattern in patterns {
        let re = Regex::new(pattern)?;
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
}
