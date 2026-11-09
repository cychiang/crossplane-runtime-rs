use crate::diff::{Difference, Kind, Layer};
use serde::Deserialize;

/// One reasoned exception. `key`, `a` and `b` are globs where `*` matches any
/// run of characters; an omitted glob matches anything.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Allowance {
    pub layer: Layer,
    pub key: String,
    pub kind: Kind,
    #[serde(default)]
    pub a: Option<String>,
    #[serde(default)]
    pub b: Option<String>,
    pub reason: String,
}

impl Allowance {
    pub fn matches(&self, d: &Difference) -> bool {
        self.layer == d.layer
            && self.kind == d.kind
            && glob(&self.key, &d.key)
            && self.a.as_deref().is_none_or(|p| glob(p, &d.a))
            && self.b.as_deref().is_none_or(|p| glob(p, &d.b))
    }
}

/// Parses JSON lines. Blank lines and lines starting with `#` are skipped.
pub fn parse(text: &str) -> Result<Vec<Allowance>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let a: Allowance = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 1))?;
        if a.reason.trim().is_empty() {
            return Err(format!(
                "line {}: reason must say why this divergence is acceptable",
                i + 1
            ));
        }
        out.push(a);
    }
    Ok(out)
}

pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || text.len() < first.len() + last.len() || !text.ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_stars() {
        assert!(glob("sample.example.com/mytypes *", "sample.example.com/mytypes ns/a"));
        assert!(glob("#* update*", "#3 update status"));
        assert!(!glob("#* patch*", "#3 update status"));
        assert!(glob("a*b*c", "abc"));
        assert!(!glob("a*a", "a"));
    }

    #[test]
    fn parse_requires_a_reason() {
        let err = parse(r#"{"layer":"audit","key":"*","kind":"verb","reason":"  "}"#).unwrap_err();
        assert!(err.contains("reason"), "{err}");
        assert!(parse(r#"{"layer":"audit","key":"*","kind":"verb"}"#).is_err());
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        assert!(parse(r#"{"layer":"audit","key":"*","kind":"verb","reason":"r","verbs":[]}"#).is_err());
    }
}
