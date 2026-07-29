use serde_json::{json, Value};

pub(super) fn check(
    id: impl Into<String>,
    status: &str,
    detail: String,
    fix: Option<&str>,
) -> Value {
    let mut entry = json!({
        "id": id.into(),
        "status": status,
        "detail": detail
    });
    if let Some(fix) = fix {
        entry["fix"] = json!(fix);
    }
    entry
}
