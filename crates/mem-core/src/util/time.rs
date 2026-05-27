use chrono::{DateTime, Utc};

pub fn now() -> String {
    Utc::now().to_rfc3339()
}

pub fn is_expired(expires_at: Option<&str>) -> bool {
    expires_at
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|expires| expires.with_timezone(&Utc) < Utc::now())
        .unwrap_or(false)
}
