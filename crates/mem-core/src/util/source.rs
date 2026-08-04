use serde_json::{Value, json};

use crate::db::Memory;

pub fn confidence_for_source(source: &str) -> &'static str {
    match source {
        "manual" => "high",
        "agent" => "medium",
        "daily_retro" | "weekly_retro" => "low",
        _ => "medium",
    }
}

pub fn source_priority(source: &str) -> u8 {
    match source {
        "manual" => 4,
        "agent" => 3,
        "daily_retro" => 2,
        "weekly_retro" => 1,
        _ => 2,
    }
}

pub fn version_conflict(memory: &Memory, expected_version: i64) -> Option<Value> {
    (memory.version != expected_version).then(|| {
        json!({
            "status": "version_conflict",
            "id": memory.id,
            "name": memory.name,
            "expected_version": expected_version,
            "actual_version": memory.version
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::now;

    fn memory_with_version(version: i64) -> Memory {
        Memory {
            id: "id".to_string(),
            r#type: "feedback".to_string(),
            name: "name".to_string(),
            description: None,
            content: Some("content".to_string()),
            tags: "[]".to_string(),
            scope: "global".to_string(),
            source: "manual".to_string(),
            confidence: "high".to_string(),
            protected: true,
            created_at: now(),
            updated_at: now(),
            expires_at: None,
            valid_until: None,
            superseded_by: None,
            version,
            access_count: 0,
            last_accessed_at: None,
            origin: "direct".to_string(),
            origin_ref: None,
            user_confirmed_at: Some(now()),
        }
    }

    #[test]
    fn source_priority_orders_trust() {
        assert!(source_priority("manual") > source_priority("agent"));
        assert!(source_priority("agent") > source_priority("daily_retro"));
        assert!(source_priority("daily_retro") > source_priority("weekly_retro"));
    }

    #[test]
    fn version_conflict_reports_mismatch() {
        let memory = memory_with_version(3);
        assert!(version_conflict(&memory, 3).is_none());
        let conflict = version_conflict(&memory, 2).expect("conflict");
        assert_eq!(conflict["status"], "version_conflict");
        assert_eq!(conflict["actual_version"], 3);
    }
}
