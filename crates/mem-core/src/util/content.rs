use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

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
        (None, Some(path)) => fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))
            .map(Some),
        (None, None) => Ok(None),
    }
}
