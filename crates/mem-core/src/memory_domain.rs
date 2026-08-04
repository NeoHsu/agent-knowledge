//! Versioned memory-write inputs and persistence semantics.
//!
//! CLI and import adapters construct wire requests here, then validation and
//! normalization produce the only type accepted by the persistence boundary.

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{Memory, log_change, memory_by_id, unique_memory_id};
use crate::scope;
use crate::util::{
    confidence_for_source, now, sanitize_secret_field, slugify, source_priority,
    validate_memory_resource_limits, validate_tags,
};
use crate::{error, workflow};

pub const SAVE_REQUEST_SCHEMA_VERSION: u64 = 1;

/// Versioned request accepted from a CLI adapter before normalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveRequestV1 {
    pub schema_version: u64,
    #[serde(rename = "type")]
    pub memory_type: String,
    pub name: String,
    pub description: Option<String>,
    pub content: String,
    pub tags: String,
    pub scope: String,
    pub source: String,
    pub confidence: Option<String>,
    pub expires_at: Option<String>,
    pub why: Option<String>,
    pub force: bool,
    pub user_confirmed: bool,
    pub redact_secrets: bool,
    pub no_validate_workflow: bool,
    pub origin: Option<String>,
    pub origin_ref: Option<String>,
}

/// Validated and normalized memory write accepted by domain persistence.
#[derive(Debug, Clone)]
pub struct SaveRequest {
    pub memory_type: String,
    pub name: String,
    pub description: Option<String>,
    pub content: String,
    pub tags: String,
    pub scope: String,
    pub source: String,
    pub confidence: Option<String>,
    pub expires_at: Option<String>,
    pub why: Option<String>,
    pub force: bool,
    pub origin: Option<String>,
    pub origin_ref: Option<String>,
}

impl SaveRequestV1 {
    pub fn validate_and_normalize(self) -> Result<SaveRequest> {
        if self.schema_version != SAVE_REQUEST_SCHEMA_VERSION {
            return Err(error::compatibility(format!(
                "unsupported save request schema {}; expected {}",
                self.schema_version, SAVE_REQUEST_SCHEMA_VERSION
            )));
        }
        let scope = scope::resolve_write_scope(&self.scope)?;
        if self.source == "manual" && !self.user_confirmed {
            return Err(error::safety_violation(
                "source=manual requires --user-confirmed",
            ));
        }
        let name = sanitize_secret_field(&self.name, "name", self.redact_secrets)?;
        let description = self
            .description
            .as_deref()
            .map(|value| sanitize_secret_field(value, "description", self.redact_secrets))
            .transpose()?;
        let why = self
            .why
            .as_deref()
            .map(|value| sanitize_secret_field(value, "why", self.redact_secrets))
            .transpose()?;
        let tags = sanitize_secret_field(&self.tags, "tags", self.redact_secrets)?;
        validate_tags(&tags)?;
        let content = sanitize_secret_field(&self.content, "content", self.redact_secrets)?;
        validate_memory_resource_limits(
            &name,
            description.as_deref(),
            &content,
            &tags,
            &scope,
            why.as_deref(),
        )?;
        workflow::validate_memory(
            &self.memory_type,
            &content,
            &tags,
            &scope,
            self.no_validate_workflow,
        )?;
        Ok(SaveRequest {
            memory_type: self.memory_type,
            name,
            description,
            content,
            tags,
            scope,
            source: self.source,
            confidence: self.confidence,
            expires_at: self.expires_at,
            why,
            force: self.force,
            origin: self.origin,
            origin_ref: self.origin_ref,
        })
    }
}

fn default_schema_version() -> u64 {
    SAVE_REQUEST_SCHEMA_VERSION
}

fn default_memory_type() -> String {
    "reference".to_string()
}

fn default_scope() -> String {
    "global".to_string()
}

/// Versioned JSON import wire shape. Unknown export metadata remains additive,
/// while typed fields fail before any item-level write begins.
#[derive(Debug, Deserialize)]
pub struct ImportWireV1 {
    #[serde(default = "default_schema_version")]
    pub schema_version: u64,
    #[serde(rename = "type", default = "default_memory_type")]
    pub memory_type: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tags: Option<Value>,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

impl ImportWireV1 {
    pub fn from_value(value: Value) -> Result<Self> {
        serde_json::from_value(value)
            .map_err(|error| error::usage(format!("invalid import item: {error}")))
    }

    pub fn into_save_request(
        self,
        source: String,
        user_confirmed: bool,
        redact_secrets: bool,
        origin_ref: String,
        no_validate_workflow: bool,
    ) -> Result<SaveRequestV1> {
        let tags = match self.tags {
            Some(Value::String(tags)) => tags,
            Some(tags) => serde_json::to_string(&tags)?,
            None => "[]".to_string(),
        };
        Ok(SaveRequestV1 {
            schema_version: self.schema_version,
            memory_type: self.memory_type,
            name: self.name,
            description: self.description,
            content: self.content.unwrap_or_default(),
            tags,
            scope: self.scope,
            source,
            confidence: None,
            expires_at: self.expires_at,
            why: None,
            force: false,
            user_confirmed,
            redact_secrets,
            no_validate_workflow,
            origin: Some("import".to_string()),
            origin_ref: Some(origin_ref),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SaveOutcome {
    DuplicateFound {
        match_type: &'static str,
        existing: Memory,
        new_content: String,
    },
    Rejected {
        reason: &'static str,
        existing: Memory,
        new_source: String,
    },
    Updated {
        match_type: &'static str,
        id: String,
        version: i64,
    },
    Saved {
        id: String,
        version: i64,
    },
}

impl SaveOutcome {
    pub fn changed_id(&self) -> Option<&str> {
        match self {
            Self::Updated { id, .. } | Self::Saved { id, .. } => Some(id),
            Self::DuplicateFound { .. } | Self::Rejected { .. } => None,
        }
    }

    pub fn version(&self) -> Option<i64> {
        match self {
            Self::Updated { version, .. } | Self::Saved { version, .. } => Some(*version),
            Self::DuplicateFound { .. } | Self::Rejected { .. } => None,
        }
    }

    pub fn to_json(&self) -> Result<Value> {
        Ok(serde_json::to_value(self)?)
    }
}

pub fn existing_save_will_write(request: &SaveRequest, existing: &Memory) -> bool {
    request.force && source_priority(&request.source) >= source_priority(&existing.source)
}

pub fn persist_save(
    conn: &Connection,
    request: &SaveRequest,
    existing: Option<&Memory>,
) -> Result<SaveOutcome> {
    match existing {
        Some(existing) => persist_existing_memory(conn, request, existing),
        None => persist_new_memory(conn, request),
    }
}

fn persist_existing_memory(
    conn: &Connection,
    request: &SaveRequest,
    existing: &Memory,
) -> Result<SaveOutcome> {
    if !request.force {
        return Ok(SaveOutcome::DuplicateFound {
            match_type: "exact_name",
            existing: existing.clone(),
            new_content: request.content.clone(),
        });
    }
    if source_priority(&request.source) < source_priority(&existing.source) {
        return Ok(SaveOutcome::Rejected {
            reason: "lower_trust_source_cannot_overwrite",
            existing: existing.clone(),
            new_source: request.source.clone(),
        });
    }

    let timestamp = now();
    let user_confirmed_at = (request.source == "manual").then(|| timestamp.clone());
    let description = request
        .description
        .clone()
        .or_else(|| request.why.clone())
        .or_else(|| existing.description.clone());
    let confidence = request
        .confidence
        .clone()
        .unwrap_or_else(|| confidence_for_source(&request.source).to_string());
    conn.execute(
        "UPDATE memories
         SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
             source = ?6, confidence = ?7, protected = ?8, updated_at = ?9,
             expires_at = ?10, origin = ?11, origin_ref = ?12,
             user_confirmed_at = COALESCE(?13, user_confirmed_at),
             version = version + 1
         WHERE id = ?14",
        params![
            request.memory_type,
            description,
            request.content,
            request.tags,
            request.scope,
            request.source,
            confidence,
            request.source == "manual",
            timestamp,
            request.expires_at,
            request.origin.as_deref().unwrap_or("direct"),
            request.origin_ref,
            user_confirmed_at,
            existing.id
        ],
    )?;
    log_change(
        conn,
        &existing.id,
        "update",
        existing.content.as_deref(),
        Some(&request.content),
        &request.source,
    )?;
    let updated = memory_by_id(conn, &existing.id)?
        .ok_or_else(|| anyhow!("updated memory missing: {}", existing.id))?;
    Ok(SaveOutcome::Updated {
        match_type: "exact_name_force",
        id: updated.id,
        version: updated.version,
    })
}

fn persist_new_memory(conn: &Connection, request: &SaveRequest) -> Result<SaveOutcome> {
    let id = unique_memory_id(conn, &slugify(&request.name))?;
    let timestamp = now();
    let confidence = request
        .confidence
        .clone()
        .unwrap_or_else(|| confidence_for_source(&request.source).to_string());
    let protected = request.source == "manual";
    let user_confirmed_at = protected.then(|| timestamp.clone());
    let description = request.description.clone().or_else(|| request.why.clone());
    let origin = request.origin.as_deref().unwrap_or("direct");

    conn.execute(
        "INSERT INTO memories
        (id, type, name, description, content, tags, scope, source, confidence, protected,
         created_at, updated_at, expires_at, origin, origin_ref, user_confirmed_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12, ?13, ?14, ?15)",
        params![
            id,
            request.memory_type,
            request.name,
            description,
            request.content,
            request.tags,
            request.scope,
            request.source,
            confidence,
            protected,
            timestamp,
            request.expires_at,
            origin,
            request.origin_ref,
            user_confirmed_at
        ],
    )
    .context("insert memory")?;
    log_change(
        conn,
        &id,
        "save",
        None,
        Some(&request.content),
        &request.source,
    )?;

    Ok(SaveOutcome::Saved { id, version: 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire() -> SaveRequestV1 {
        SaveRequestV1 {
            schema_version: SAVE_REQUEST_SCHEMA_VERSION,
            memory_type: "reference".to_string(),
            name: "release_notes".to_string(),
            description: None,
            content: "Action: keep release notes. Why: test.".to_string(),
            tags: "[]".to_string(),
            scope: "global".to_string(),
            source: "agent".to_string(),
            confidence: None,
            expires_at: None,
            why: None,
            force: false,
            user_confirmed: false,
            redact_secrets: false,
            no_validate_workflow: false,
            origin: None,
            origin_ref: None,
        }
    }

    #[test]
    fn import_wire_defaults_and_normalizes_tags() {
        let import = ImportWireV1::from_value(serde_json::json!({
            "name": "release_notes",
            "content": "Action: import notes. Why: test.",
            "tags": ["domain:release"]
        }))
        .expect("parse import");
        let request = import
            .into_save_request(
                "agent".to_string(),
                false,
                false,
                "input.json".to_string(),
                false,
            )
            .expect("wire request")
            .validate_and_normalize()
            .expect("normalize request");

        assert_eq!(request.memory_type, "reference");
        assert_eq!(request.scope, "global");
        assert_eq!(request.tags, r#"["domain:release"]"#);
    }

    #[test]
    fn manual_source_requires_confirmation_at_domain_boundary() {
        let mut request = wire();
        request.source = "manual".to_string();

        let error = request.validate_and_normalize().expect_err("must reject");

        assert!(error.to_string().contains("requires --user-confirmed"));
    }

    #[test]
    fn unsupported_wire_version_fails_before_persistence() {
        let mut request = wire();
        request.schema_version += 1;

        let error = request.validate_and_normalize().expect_err("must reject");

        assert!(
            error
                .to_string()
                .contains("unsupported save request schema")
        );
    }
}
