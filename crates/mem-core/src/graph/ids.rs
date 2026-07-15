//! Stable graph node identifier construction.

pub(super) fn memory_node_id(id: &str) -> String {
    format!("memory:{id}")
}

pub(super) fn tag_node_id(tag: &str) -> String {
    format!("tag:{tag}")
}

pub(super) fn scope_node_id(scope: &str) -> String {
    format!("scope:{scope}")
}

pub(super) fn type_node_id(memory_type: &str) -> String {
    format!("type:{memory_type}")
}

pub(super) fn source_node_id(source: &str) -> String {
    format!("source:{source}")
}

pub(super) fn artifact_node_id(path: &str) -> String {
    format!("artifact:{path}")
}

pub(super) fn workflow_step_node_id(memory_id: &str, step_id: &str) -> String {
    format!("workflow_step:{memory_id}:{}", safe_node_part(step_id))
}

pub(super) fn safe_node_part(input: &str) -> String {
    let mut slug = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '_' | '-' | '/' | ':' | '.') {
            slug.push(ch);
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        stable_hash_hex(input)
    } else {
        slug
    }
}

pub(super) fn stable_hash_hex(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
