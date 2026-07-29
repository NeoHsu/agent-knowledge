use std::fmt;

use anyhow::Error;
use serde_json::{json, Map, Value};

pub(crate) const INDEX_STALE_AFTER_WRITE: &str = "index_stale_after_write";

#[derive(Debug)]
pub(crate) struct StructuredCommandError {
    code: &'static str,
    message: String,
    exit_code: u8,
    retryable: bool,
    details: Value,
}

impl StructuredCommandError {
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn exit_code(&self) -> u8 {
        self.exit_code
    }

    pub(crate) fn retryable(&self) -> bool {
        self.retryable
    }

    pub(crate) fn details(&self) -> &Value {
        &self.details
    }
}

impl fmt::Display for StructuredCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StructuredCommandError {}

fn structured_error(
    code: &'static str,
    message: impl Into<String>,
    exit_code: u8,
    details: Value,
) -> Error {
    StructuredCommandError {
        code,
        message: message.into(),
        exit_code,
        retryable: false,
        details,
    }
    .into()
}

pub(crate) fn committed_index_error(
    operation: &str,
    operation_details: Value,
    source: Error,
) -> Error {
    let mut details = match operation_details {
        Value::Object(details) => details,
        _ => Map::new(),
    };
    details.insert("operation".to_string(), json!(operation));
    details.insert("durable_write_committed".to_string(), json!(true));
    details.insert("index_stale".to_string(), json!(true));
    details.insert("recovery".to_string(), json!("mem reindex"));

    structured_error(
        INDEX_STALE_AFTER_WRITE,
        format!(
            "durable {operation} committed, but the search index update failed; run `mem reindex`: {source:#}"
        ),
        1,
        Value::Object(details),
    )
}
