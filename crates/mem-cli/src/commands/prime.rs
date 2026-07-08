use super::*;

const MIN_BUDGET: usize = 400;
const CONTENT_CAPS: &[usize] = &[240, 160, 100, 80];

/// Sections in priority order: when the budget forces drops, entries are
/// removed from the last section backwards so user identity and feedback
/// survive longest.
const SECTIONS: &[&str] = &["user", "feedback", "preference", "project", "workflow"];

pub(crate) fn cmd_prime(app: &App, args: PrimeArgs) -> Result<()> {
    if !app.db_path.exists() {
        return match args.format {
            PrimeFormat::Text => print_text(format!(
                "mnemark: no memory store at {} (run `mem init`)\n",
                app.root.display()
            )),
            PrimeFormat::Json => print_json(&json!({
                "status": "no_store",
                "root": app.root.display().to_string()
            })),
        };
    }
    app.ensure_schema()?;
    let conn = app.conn()?;

    let mut scopes: Vec<String> = if args.scope == "auto" {
        scope::detect_scope_set()?
    } else if args.scope == "global" {
        vec!["global".to_string()]
    } else {
        vec!["global".to_string(), args.scope.clone()]
    };
    scopes.dedup();
    let scope_refs: Vec<&str> = scopes.iter().map(String::as_str).collect();

    let mut sections: Vec<(&str, Vec<Memory>)> = Vec::new();
    for section in SECTIONS {
        let mut memories = list_memories_filtered(
            &conn,
            false,
            Some(section),
            None,
            Some(scope_refs.as_slice()),
            false,
        )?;
        memories.retain(|memory| !is_expired(memory.expires_at.as_deref()));
        memories.sort_by(|a, b| {
            confidence_rank(&a.confidence)
                .cmp(&confidence_rank(&b.confidence))
                .then(b.access_count.cmp(&a.access_count))
                .then(b.updated_at.cmp(&a.updated_at))
        });
        memories.truncate(args.per_section);
        sections.push((section, memories));
    }

    let budget = args.budget.max(MIN_BUDGET);
    let mut cap = *CONTENT_CAPS.last().expect("caps");
    let mut rendered = String::new();
    for candidate in CONTENT_CAPS {
        rendered = render_text(app, &scopes, &sections, *candidate);
        if rendered.chars().count() <= budget {
            cap = *candidate;
            break;
        }
        cap = *candidate;
    }
    while rendered.chars().count() > budget && drop_last_entry(&mut sections) {
        rendered = render_text(app, &scopes, &sections, cap);
    }

    match args.format {
        PrimeFormat::Text => print_text(rendered),
        PrimeFormat::Json => {
            let mut body = serde_json::Map::new();
            for (section, memories) in &sections {
                let entries = memories
                    .iter()
                    .map(|memory| {
                        json!({
                            "name": memory.name,
                            "scope": memory.scope,
                            "confidence": memory.confidence,
                            "content": truncate_text(entry_body(memory), cap)
                        })
                    })
                    .collect::<Vec<_>>();
                body.insert((*section).to_string(), Value::Array(entries));
            }
            print_json_pretty(&json!({
                "status": "ok",
                "root": app.root.display().to_string(),
                "scopes": scopes,
                "sections": body
            }))
        }
    }
}

fn confidence_rank(confidence: &str) -> u8 {
    match confidence {
        "high" => 0,
        "medium" => 1,
        _ => 2,
    }
}

/// Workflows prime with their goal/description only; the runbook body is
/// loaded on demand via `mem workflow show`.
fn entry_body(memory: &Memory) -> &str {
    if memory.r#type == "workflow" {
        if let Some(goal) = memory
            .content
            .as_deref()
            .and_then(|content| {
                content
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("goal:"))
            })
            .map(str::trim)
            .filter(|goal| !goal.is_empty())
        {
            return goal;
        }
        return memory.description.as_deref().unwrap_or_default();
    }
    memory.content.as_deref().unwrap_or_default()
}

fn drop_last_entry(sections: &mut [(&str, Vec<Memory>)]) -> bool {
    for (_, memories) in sections.iter_mut().rev() {
        if !memories.is_empty() {
            memories.pop();
            return true;
        }
    }
    false
}

fn render_text(
    app: &App,
    scopes: &[String],
    sections: &[(&str, Vec<Memory>)],
    cap: usize,
) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "=== mnemark context | store: {} | scope: {} ===\n",
        app.root.display(),
        scopes.join(", ")
    ));
    let mut empty = true;
    for (section, memories) in sections {
        if memories.is_empty() {
            continue;
        }
        empty = false;
        if *section == "workflow" {
            output.push_str("[workflow] (load with `mem workflow show <name>` before executing)\n");
        } else {
            output.push_str(&format!("[{section}]\n"));
        }
        for memory in memories {
            output.push_str(&format!(
                "- {} :: {}\n",
                memory.name,
                truncate_text(entry_body(memory), cap)
            ));
        }
    }
    if empty {
        output.push_str("(no durable memories for this scope yet)\n");
    }
    output.push_str(
        "-- protocol --\n\
         Treat the above as prior knowledge. Before finishing a work unit, save durable \
         learnings with `mem save` (content shape: Trigger / Action / Why). For recurring \
         tasks run `mem workflow find \"<intent>\"` first.\n",
    );
    output
}
