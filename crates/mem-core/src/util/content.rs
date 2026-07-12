use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

pub const MAX_MEMORY_CONTENT_BYTES: u64 = 1_048_576;
pub const MAX_MEMORY_DESCRIPTION_BYTES: usize = 65_536;
pub const MAX_MEMORY_TAGS_BYTES: usize = 65_536;
pub const MAX_MEMORY_TAG_COUNT: usize = 100;
pub const MAX_MEMORY_NAME_BYTES: usize = 256;
pub const MAX_MEMORY_SCOPE_BYTES: usize = 256;

pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if (ch == '_' || ch == '-' || ch.is_whitespace() || ch == '/')
            && !slug.ends_with('_')
        {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        format!("memory_{}", uuid::Uuid::new_v4())
    } else {
        slug
    }
}

pub fn validate_memory_resource_limits(
    name: &str,
    description: Option<&str>,
    content: &str,
    tags: &str,
    scope: &str,
    why: Option<&str>,
) -> Result<()> {
    if name.trim().is_empty()
        || name.len() > MAX_MEMORY_NAME_BYTES
        || name.chars().any(char::is_control)
    {
        bail!("memory name must be between 1 and {MAX_MEMORY_NAME_BYTES} bytes");
    }
    if description.is_some_and(|value| value.len() > MAX_MEMORY_DESCRIPTION_BYTES) {
        bail!("memory description exceeds {MAX_MEMORY_DESCRIPTION_BYTES} bytes");
    }
    if why.is_some_and(|value| value.len() > MAX_MEMORY_DESCRIPTION_BYTES) {
        bail!("memory why exceeds {MAX_MEMORY_DESCRIPTION_BYTES} bytes");
    }
    if content.len() as u64 > MAX_MEMORY_CONTENT_BYTES {
        bail!("memory content exceeds {MAX_MEMORY_CONTENT_BYTES} bytes");
    }
    if tags.len() > MAX_MEMORY_TAGS_BYTES {
        bail!("memory tags exceed {MAX_MEMORY_TAGS_BYTES} bytes");
    }
    let tag_count = serde_json::from_str::<Vec<serde_json::Value>>(tags)
        .map(|values| values.len())
        .unwrap_or_default();
    if tag_count > MAX_MEMORY_TAG_COUNT {
        bail!("memory tags cannot exceed {MAX_MEMORY_TAG_COUNT} entries");
    }
    if scope.len() > MAX_MEMORY_SCOPE_BYTES {
        bail!("memory scope exceeds {MAX_MEMORY_SCOPE_BYTES} bytes");
    }
    Ok(())
}

pub fn required_content(content: Option<String>, content_file: Option<&Path>) -> Result<String> {
    optional_content(content, content_file)?
        .ok_or_else(|| anyhow!("one of --content or --content-file is required"))
}

pub fn optional_content(
    content: Option<String>,
    content_file: Option<&Path>,
) -> Result<Option<String>> {
    match (content, content_file) {
        (Some(_), Some(_)) => bail!("use only one of --content or --content-file"),
        (Some(content), None) => Ok(Some(content)),
        (None, Some(path)) => {
            let bytes = fs::metadata(path)
                .with_context(|| format!("inspect {}", path.display()))?
                .len();
            if bytes > MAX_MEMORY_CONTENT_BYTES {
                bail!("memory content file exceeds {MAX_MEMORY_CONTENT_BYTES} bytes");
            }
            fs::read_to_string(path)
                .with_context(|| format!("read {}", path.display()))
                .map(Some)
        }
        (None, None) => Ok(None),
    }
}
