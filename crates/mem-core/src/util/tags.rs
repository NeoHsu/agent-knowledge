use std::collections::HashSet;

use anyhow::Result;
use serde_json::Value;

use crate::error;

pub fn validate_tags(tags: &str) -> Result<()> {
    parse_string_array(tags)?;
    Ok(())
}

pub fn parse_string_array(raw: &str) -> Result<Vec<String>> {
    let parsed: Value = serde_json::from_str(raw).map_err(|source| {
        error::usage(format!("tags/memory_ids must be a JSON array: {source}"))
    })?;
    let Value::Array(values) = parsed else {
        return Err(error::usage("expected JSON array"));
    };
    values
        .into_iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| error::usage("array items must be strings"))
        })
        .collect()
}

pub fn memory_has_tag(tags: &str, wanted: &str) -> bool {
    parse_string_array(tags)
        .map(|tags| tags.iter().any(|tag| tag == wanted))
        .unwrap_or(false)
}

pub fn merge_tags(existing: &str, add: &str) -> Result<String> {
    let mut set = HashSet::new();
    for raw in [existing, add] {
        let parsed: Value = serde_json::from_str(raw)
            .map_err(|source| error::usage(format!("tags must be JSON arrays: {source}")))?;
        let Value::Array(values) = parsed else {
            return Err(error::usage("tags must be JSON arrays"));
        };
        for value in values {
            if let Some(tag) = value.as_str() {
                set.insert(tag.to_string());
            }
        }
    }
    let mut tags: Vec<_> = set.into_iter().collect();
    tags.sort();
    Ok(serde_json::to_string(&tags)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_arrays_only() {
        assert_eq!(parse_string_array(r#"["a","b"]"#).unwrap(), vec!["a", "b"]);
        assert!(parse_string_array(r#"["a",1]"#).is_err());
        assert!(parse_string_array(r#"{"a":1}"#).is_err());
        assert!(parse_string_array("not-json").is_err());
    }

    #[test]
    fn validates_and_matches_exact_tags() {
        validate_tags(r#"["project:one","workflow:test"]"#).expect("valid tags");
        assert!(memory_has_tag(
            r#"["project:one","workflow:test"]"#,
            "project:one"
        ));
        assert!(!memory_has_tag(r#"["project:one"]"#, "project"));
        assert!(!memory_has_tag("invalid", "project:one"));
        assert!(validate_tags(r#"["valid",2]"#).is_err());
    }

    #[test]
    fn merges_deduplicates_and_sorts_tags() {
        assert_eq!(
            merge_tags(r#"["z","a"]"#, r#"["b","a",3]"#).expect("merge tags"),
            r#"["a","b","z"]"#
        );
        assert!(merge_tags("not-json", "[]").is_err());
        assert!(merge_tags("{}", "[]").is_err());
    }
}
