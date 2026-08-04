use std::collections::BTreeSet;

use super::lint::lint_memory;
use super::*;

pub(super) fn resolve_mutation_memory(
    conn: &Connection,
    reference: &str,
    scope_value: &str,
) -> Result<Memory> {
    let scopes = if scope_value == "auto" {
        scope::detect_scope_set()?
    } else {
        scope::validate_scope(scope_value)?;
        vec![scope_value.to_string()]
    };
    let scope_refs = scopes.iter().map(String::as_str).collect::<Vec<_>>();
    let id = resolve_memory_ref_in_scopes(conn, reference, Some(&scope_refs))?;
    memory_by_id(conn, &id)?.ok_or_else(|| anyhow!("memory not found: {reference}"))
}

fn update_tag_set(
    current: &str,
    set: Option<&str>,
    add: Option<&str>,
    remove: Option<&str>,
) -> Result<String> {
    let base = set.unwrap_or(current);
    let mut tags = parse_string_array(base)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    if let Some(add) = add {
        tags.extend(parse_string_array(add)?);
    }
    if let Some(remove) = remove {
        for tag in parse_string_array(remove)? {
            tags.remove(&tag);
        }
    }
    serde_json::to_string(&tags.into_iter().collect::<Vec<_>>()).map_err(Into::into)
}

pub(crate) fn cmd_update(app: &App, args: UpdateArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.conn()?;
    let old = resolve_mutation_memory(&conn, &args.name, &args.scope)?;
    if let Some(expected) = args.expected_version
        && let Some(conflict) = version_conflict(&old, expected)
    {
        print_json(&conflict)?;
        return Ok(());
    }

    let update_source = args.source.as_deref().unwrap_or(&old.source).to_string();
    if update_source == "manual" && !args.user_confirmed {
        bail!("source=manual requires --user-confirmed");
    }
    if source_priority(&update_source) < source_priority(&old.source) {
        print_json(&json!({
            "status": "rejected",
            "reason": "lower_trust_source_cannot_update",
            "existing_source": old.source,
            "new_source": update_source,
            "id": old.id
        }))?;
        return Ok(());
    }

    let new_type = args.r#type.as_deref().unwrap_or(&old.r#type).to_string();
    let new_scope = match args.set_scope.as_deref() {
        Some(scope) => scope::resolve_write_scope(scope)?,
        None => old.scope.clone(),
    };
    if let Some(collision) = memory_by_name_in_scope(&conn, &old.name, &new_scope)?
        && collision.id != old.id
    {
        bail!(
            "memory name already exists in destination scope {}: {}",
            new_scope,
            old.name
        );
    }
    let new_content = match optional_content(args.content, args.content_file.as_deref())? {
        Some(content) => Some(sanitize_secret_field(
            &content,
            "content",
            args.redact_secrets,
        )?),
        None => old.content.clone(),
    };
    let description = if args.clear_description {
        None
    } else {
        match args.description.as_deref() {
            Some(value) => Some(sanitize_secret_field(
                value,
                "description",
                args.redact_secrets,
            )?),
            None => old.description.clone(),
        }
    };
    let set_tags = args
        .set_tags
        .as_deref()
        .map(|value| sanitize_secret_field(value, "set_tags", args.redact_secrets))
        .transpose()?;
    let add_tags = args
        .add_tags
        .as_deref()
        .map(|value| sanitize_secret_field(value, "add_tags", args.redact_secrets))
        .transpose()?;
    let remove_tags = args
        .remove_tags
        .as_deref()
        .map(|value| sanitize_secret_field(value, "remove_tags", args.redact_secrets))
        .transpose()?;
    let tags = update_tag_set(
        &old.tags,
        set_tags.as_deref(),
        add_tags.as_deref(),
        remove_tags.as_deref(),
    )?;
    let expires_at = if args.clear_expires_at {
        None
    } else {
        args.expires_at.clone().or(old.expires_at.clone())
    };
    let confidence = args
        .confidence
        .as_deref()
        .unwrap_or(&old.confidence)
        .to_string();
    validate_memory_resource_limits(
        &old.name,
        description.as_deref(),
        new_content.as_deref().unwrap_or_default(),
        &tags,
        &new_scope,
        None,
    )?;
    workflow_core::validate_memory(
        &new_type,
        new_content.as_deref().unwrap_or_default(),
        &tags,
        &new_scope,
        args.no_validate_workflow,
    )?;
    let now = now();
    let user_confirmed_at = (update_source == "manual").then(|| now.clone());

    with_transaction(&conn, |conn| {
        conn.execute(
            "UPDATE memories
             SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
                 source = ?6, confidence = ?7, protected = ?8, expires_at = ?9,
                 origin = 'direct', origin_ref = NULL,
                 user_confirmed_at = COALESCE(?10, user_confirmed_at),
                 updated_at = ?11, version = version + 1
             WHERE id = ?12",
            params![
                new_type,
                description,
                new_content,
                tags,
                new_scope,
                update_source,
                confidence,
                update_source == "manual",
                expires_at,
                user_confirmed_at,
                now,
                old.id
            ],
        )?;
        log_change(
            conn,
            &old.id,
            "update",
            old.content.as_deref(),
            new_content.as_deref(),
            &update_source,
        )?;
        mem_core::graph::set_graph_dirty(conn, true)
    })?;
    let updated = memory_by_id(&conn, &old.id)?
        .ok_or_else(|| anyhow!("updated memory missing: {}", old.id))?;
    finish_committed_index_write(
        memory_index::upsert_or_mark_stale(app, &conn, &old.id),
        "memory update",
        json!({"memory_id": old.id, "version": updated.version}),
    )?;
    let warnings = lint_memory(
        &updated.r#type,
        &updated.name,
        updated.content.as_deref().unwrap_or_default(),
        &updated.tags,
    );
    print_write_json(
        app,
        json!({
            "status": "updated",
            "id": updated.id,
            "scope": updated.scope,
            "version": updated.version,
            "warnings": warnings
        }),
    )?;
    Ok(())
}
