use super::*;

pub(super) fn sanitize_incoming_memory(
    memory: &mut Memory,
    incoming_store: &str,
    allow_secret_redaction: bool,
) -> Result<()> {
    memory.name = sanitize_secret_field(&memory.name, "memory name", allow_secret_redaction)?;
    memory.description = memory
        .description
        .as_deref()
        .map(|value| sanitize_secret_field(value, "memory description", allow_secret_redaction))
        .transpose()?;
    memory.content = memory
        .content
        .as_deref()
        .map(|value| sanitize_secret_field(value, "memory content", allow_secret_redaction))
        .transpose()?;
    memory.tags = sanitize_secret_field(&memory.tags, "memory tags", allow_secret_redaction)?;
    memory.scope = sanitize_secret_field(&memory.scope, "memory scope", allow_secret_redaction)?;
    validate_tags(&memory.tags)?;
    scope::validate_scope(&memory.scope)?;
    validate_memory_resource_limits(
        &memory.name,
        memory.description.as_deref(),
        memory.content.as_deref().unwrap_or_default(),
        &memory.tags,
        &memory.scope,
        None,
    )?;
    memory.origin = "merge".to_string();
    memory.origin_ref = Some(incoming_store.to_string());
    Ok(())
}

pub(super) fn sanitize_optional(
    value: Option<&str>,
    field: &str,
    allow_secret_redaction: bool,
) -> Result<Option<String>> {
    value
        .map(|value| sanitize_secret_field(value, field, allow_secret_redaction))
        .transpose()
}
