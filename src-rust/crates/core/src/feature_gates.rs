// feature_gates.rs — Minimal env-var helpers.
//
// No remote service, no settings.json integration, no in-process overrides.
// Just simple env-var truthiness and parsing utilities.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Env-var truthiness helpers
// ---------------------------------------------------------------------------

/// Return `true` when `val` is a truthy env-var value.
///
/// Truthy: `"1"`, `"true"`, `"yes"`, `"on"` (case-insensitive).
/// `None` (variable unset) is falsy.
pub fn is_env_truthy(val: Option<&str>) -> bool {
    match val {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        None => false,
    }
}

/// Return `true` when `val` is an explicitly-falsy env-var value.
///
/// Falsy: `"0"`, `"false"`, `"no"`, `"off"` (case-insensitive).
/// `None` (variable unset) returns `false` — unset is *not* defined-falsy.
pub fn is_env_defined_falsy(val: Option<&str>) -> bool {
    match val {
        Some(v) => {
            matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off")
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Bare / simple mode
// ---------------------------------------------------------------------------

/// Return `true` when Claurst should run in "bare" (minimal) mode.
///
/// Bare mode skips LSP, plugin, and MCP startup for a faster, lighter
/// experience.  It is enabled by either:
///   - The `CLAURST_SIMPLE=1` environment variable, OR
///   - The `--bare` flag in `std::env::args()`.
pub fn is_bare_mode() -> bool {
    if is_env_truthy(std::env::var("CLAURST_SIMPLE").ok().as_deref()) {
        return true;
    }
    std::env::args().any(|a| a == "--bare")
}

// ---------------------------------------------------------------------------
// Env-var parsing for --env KEY=VALUE arguments
// ---------------------------------------------------------------------------

/// Parse a slice of `"KEY=VALUE"` strings into a `HashMap`.
///
/// Returns an error if any entry lacks a `=` separator.
pub fn parse_env_vars(args: &[String]) -> anyhow::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for entry in args {
        if let Some(pos) = entry.find('=') {
            let key = entry[..pos].to_string();
            let value = entry[pos + 1..].to_string();
            map.insert(key, value);
        } else {
            return Err(anyhow::anyhow!(
                "Invalid env-var format '{}': expected KEY=VALUE",
                entry
            ));
        }
    }
    Ok(map)
}

// ---------------------------------------------------------------------------
// AWS region
// ---------------------------------------------------------------------------

/// Resolve the AWS region, checking `AWS_REGION` then `AWS_DEFAULT_REGION`,
/// falling back to `"us-east-1"`.
pub fn get_aws_region() -> String {
    std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_values() {
        for v in &["1", "true", "True", "TRUE", "yes", "YES", "on", "ON"] {
            assert!(is_env_truthy(Some(v)), "expected truthy for {:?}", v);
        }
    }

    #[test]
    fn falsy_values_are_not_truthy() {
        for v in &["0", "false", "no", "off", "", "anything"] {
            assert!(!is_env_truthy(Some(v)), "expected non-truthy for {:?}", v);
        }
        assert!(!is_env_truthy(None));
    }

    #[test]
    fn defined_falsy_values() {
        for v in &["0", "false", "False", "FALSE", "no", "NO", "off", "OFF"] {
            assert!(
                is_env_defined_falsy(Some(v)),
                "expected defined-falsy for {:?}",
                v
            );
        }
    }

    #[test]
    fn non_falsy_values() {
        for v in &["1", "true", "yes", "on", ""] {
            assert!(
                !is_env_defined_falsy(Some(v)),
                "expected non-defined-falsy for {:?}",
                v
            );
        }
        assert!(!is_env_defined_falsy(None));
    }

    #[test]
    fn parse_env_vars_basic() {
        let args = vec!["KEY=VALUE".to_string(), "FOO=bar=baz".to_string()];
        let map = parse_env_vars(&args).unwrap();
        assert_eq!(map["KEY"], "VALUE");
        assert_eq!(map["FOO"], "bar=baz");
    }

    #[test]
    fn parse_env_vars_error_on_no_equals() {
        let args = vec!["NOEQUALSSIGN".to_string()];
        assert!(parse_env_vars(&args).is_err());
    }

    #[test]
    fn aws_region_fallback() {
        let region = get_aws_region();
        assert!(!region.is_empty());
    }
}
