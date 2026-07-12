use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

pub fn normalize_rfc3339(value: &str) -> Result<String> {
    DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid RFC3339 timestamp: {value}"))
        .map(|timestamp| timestamp.with_timezone(&Utc).to_rfc3339())
}

pub fn is_expired(expires_at: Option<&str>) -> bool {
    match expires_at {
        None => false,
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map(|expires| expires.with_timezone(&Utc) < Utc::now())
            // Invalid legacy timestamps must never make a memory look active.
            .unwrap_or(true),
    }
}
