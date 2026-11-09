use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    DropResourceVersion,
    DropManagedFields,
    MaskTimestamps,
    TokenizeUids,
    WrappedErrorTail,
    CollapseRepeatedRounds,
}

impl Rule {
    pub const ALL: [Rule; 6] = [
        Rule::DropResourceVersion,
        Rule::DropManagedFields,
        Rule::MaskTimestamps,
        Rule::TokenizeUids,
        Rule::WrappedErrorTail,
        Rule::CollapseRepeatedRounds,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Rule::DropResourceVersion => "drop-resource-version",
            Rule::DropManagedFields => "drop-managed-fields",
            Rule::MaskTimestamps => "mask-timestamps",
            Rule::TokenizeUids => "tokenize-uids",
            Rule::WrappedErrorTail => "wrapped-error-tail",
            Rule::CollapseRepeatedRounds => "collapse-repeated-rounds",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Rule {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        Rule::ALL.into_iter().find(|r| r.name() == s).ok_or_else(|| {
            let names: Vec<_> = Rule::ALL.iter().map(|r| r.name()).collect();
            format!("unknown rule {s:?}; rules are {}", names.join(", "))
        })
    }
}

pub struct Normalizer {
    disabled: BTreeSet<Rule>,
    uids: BTreeMap<String, String>,
}

impl Normalizer {
    pub fn new(disabled: BTreeSet<Rule>, uids: BTreeMap<String, String>) -> Self {
        Normalizer { disabled, uids }
    }

    pub fn on(&self, rule: Rule) -> bool {
        !self.disabled.contains(&rule)
    }

    pub fn object(&self, mut obj: Value) -> Value {
        if self.on(Rule::DropResourceVersion) {
            drop_resource_version(&mut obj);
        }
        if self.on(Rule::DropManagedFields) {
            drop_managed_fields(&mut obj);
        }
        if self.on(Rule::MaskTimestamps) {
            mask_timestamps(&mut obj);
        }
        if self.on(Rule::TokenizeUids) {
            tokenize_uids(&mut obj, &self.uids);
        }
        if self.on(Rule::WrappedErrorTail) {
            if let Some(conds) = obj.pointer_mut("/status/conditions").and_then(Value::as_array_mut) {
                for c in conds {
                    if let Some(Value::String(m)) = c.get_mut("message") {
                        *m = wrapped_error_tail(m);
                    }
                }
            }
        }
        obj
    }

    pub fn name(&self, name: &str) -> String {
        if self.on(Rule::TokenizeUids) {
            if let Some(t) = self.uids.get(name) {
                return t.clone();
            }
        }
        name.to_string()
    }

    pub fn message(&self, msg: &str) -> String {
        if self.on(Rule::WrappedErrorTail) {
            wrapped_error_tail(msg)
        } else {
            msg.to_string()
        }
    }
}

pub fn drop_resource_version(obj: &mut Value) {
    if let Some(m) = obj.get_mut("metadata").and_then(Value::as_object_mut) {
        m.remove("resourceVersion");
    }
}

pub fn drop_managed_fields(obj: &mut Value) {
    if let Some(m) = obj.get_mut("metadata").and_then(Value::as_object_mut) {
        m.remove("managedFields");
    }
}

pub const TIMESTAMP_TOKEN: &str = "<ts>";

pub fn mask_timestamps(v: &mut Value) {
    match v {
        Value::String(s) if is_rfc3339(s) => *s = TIMESTAMP_TOKEN.to_string(),
        Value::Array(xs) => xs.iter_mut().for_each(mask_timestamps),
        Value::Object(m) => m.values_mut().for_each(mask_timestamps),
        _ => {}
    }
}

pub fn is_rfc3339(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| r.clone().all(|i| b.get(i).is_some_and(u8::is_ascii_digit));
    if b.len() < 20
        || !digits(0..4)
        || b[4] != b'-'
        || !digits(5..7)
        || b[7] != b'-'
        || !digits(8..10)
        || b[10] != b'T'
        || !digits(11..13)
        || b[13] != b':'
        || !digits(14..16)
        || b[16] != b':'
        || !digits(17..19)
    {
        return false;
    }
    let mut i = 19;
    if b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    match &b[i..] {
        [b'Z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => [h1, h2, m1, m2].iter().all(|c| c.is_ascii_digit()),
        _ => false,
    }
}

pub fn uid_token(kind: &str, namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(ns) => format!("<uid {kind} {ns}/{name}>"),
        None => format!("<uid {kind} {name}>"),
    }
}

pub fn tokenize_uids(v: &mut Value, uids: &BTreeMap<String, String>) {
    match v {
        Value::String(s) => {
            if let Some(t) = uids.get(s.as_str()) {
                *s = t.clone();
            }
        }
        Value::Array(xs) => xs.iter_mut().for_each(|x| tokenize_uids(x, uids)),
        Value::Object(m) => m.values_mut().for_each(|x| tokenize_uids(x, uids)),
        _ => {}
    }
}

/// Error strings crossplane-runtime v2.4.3 wraps a cause with, from
/// pkg/reconciler/managed/{reconciler,api}.go, pkg/reconciler/providerconfig
/// and pkg/resource/providerconfig.go.
pub const RUNTIME_WRAP_PREFIXES: &[&str] = &[
    "cannot apply ProviderConfigUsage",
    "cannot create or update connection secret",
    "cannot delete ProviderConfigUsage",
    "cannot get ProviderConfig",
    "cannot get credentials secret",
    "cannot get managed resource",
    "cannot list ProviderConfigUsages",
    "cannot patch the managed resource via server-side apply",
    "cannot record change log entry",
    "cannot resolve references",
    "cannot update ProviderConfig",
    "cannot update ProviderConfig status",
    "cannot update critical annotations",
    "cannot update managed resource",
    "cannot update managed resource annotations",
    "cannot update managed resource status",
    "connect failed",
    "create failed",
    "delete failed",
    "observe failed",
    "update failed",
];

/// Causes crossplane-runtime v2.4.3 creates itself with errors.New and then
/// wraps, such as reconciler.go:1205.
pub const RUNTIME_TAILS: &[&str] = &[
    "external resource does not exist",
    "managed resource does not reference a ProviderConfig",
    "managed resource ProviderConfig reference has no Kind",
    "cannot extract from environment variable when none specified",
    "cannot extract from filesystem when no path specified",
    "cannot extract from secret key when none specified",
];

pub const WRAPPED_TOKEN: &str = "<wrapped error>";

/// The provider contract (parity/README.md) fixes the external client's own
/// error text, so a cause of that shape is authored text and stays exact.
pub fn is_contract_error(cause: &str) -> bool {
    let Some((call, status)) = cause.split_once(": status ") else {
        return false;
    };
    let Some((method, path)) = call.split_once(' ') else {
        return false;
    };
    matches!(method, "GET" | "PUT" | "DELETE")
        && path
            .strip_prefix("/things/")
            .is_some_and(|n| !n.is_empty() && !n.contains(' '))
        && !status.is_empty()
        && status.bytes().all(|c| c.is_ascii_digit())
}

pub fn wrapped_error_tail(msg: &str) -> String {
    let mut prefix_end = 0;
    let mut rest = msg;
    while let Some((head, tail)) = rest.split_once(": ") {
        if !RUNTIME_WRAP_PREFIXES.contains(&head) {
            break;
        }
        prefix_end += head.len() + 2;
        rest = tail;
    }
    if prefix_end == 0 || is_contract_error(rest) || RUNTIME_TAILS.contains(&rest) {
        return msg.to_string();
    }
    format!("{}{WRAPPED_TOKEN}", &msg[..prefix_end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn drop_resource_version_removes_only_metadata_resource_version() {
        let mut o = json!({"metadata": {"resourceVersion": "17", "name": "a"}, "spec": {"resourceVersion": "x"}});
        drop_resource_version(&mut o);
        assert_eq!(o, json!({"metadata": {"name": "a"}, "spec": {"resourceVersion": "x"}}));
    }

    #[test]
    fn drop_managed_fields_removes_only_managed_fields() {
        let mut o = json!({"metadata": {"managedFields": [{"manager": "m"}], "finalizers": ["f"]}});
        drop_managed_fields(&mut o);
        assert_eq!(o, json!({"metadata": {"finalizers": ["f"]}}));
    }

    #[test]
    fn mask_timestamps_masks_rfc3339_values_anywhere_and_keeps_keys() {
        let mut o = json!({
            "metadata": {"creationTimestamp": "2026-11-09T22:43:19Z",
                         "annotations": {"crossplane.io/external-create-pending": "2026-11-09T22:43:19.123456Z"}},
            "status": {"conditions": [{"lastTransitionTime": "2026-11-09T22:43:19+01:00", "reason": "Available"}]},
            "spec": {"field": "2026-11-09"}
        });
        mask_timestamps(&mut o);
        assert_eq!(
            o,
            json!({
                "metadata": {"creationTimestamp": "<ts>",
                             "annotations": {"crossplane.io/external-create-pending": "<ts>"}},
                "status": {"conditions": [{"lastTransitionTime": "<ts>", "reason": "Available"}]},
                "spec": {"field": "2026-11-09"}
            })
        );
    }

    #[test]
    fn is_rfc3339_rejects_near_misses() {
        for s in [
            "2026-11-09T22:43:19",
            "2026-11-09 22:43:19Z",
            "2026-11-09T22:43:19.Z",
            "v2026-11-09T22:43:19Z",
        ] {
            assert!(!is_rfc3339(s), "{s}");
        }
    }

    #[test]
    fn tokenize_uids_replaces_known_uids_in_any_string_value() {
        let uids = BTreeMap::from([("3f1c".to_string(), uid_token("MyType", Some("ns"), "a"))]);
        let mut o = json!({"metadata": {"name": "3f1c", "ownerReferences": [{"uid": "3f1c"}], "uid": "other"}});
        tokenize_uids(&mut o, &uids);
        assert_eq!(
            o,
            json!({"metadata": {"name": "<uid MyType ns/a>", "ownerReferences": [{"uid": "<uid MyType ns/a>"}], "uid": "other"}})
        );
    }

    #[test]
    fn wrapped_error_tail_keeps_contract_errors_exact() {
        let m = "create failed: PUT /things/a: status 500";
        assert_eq!(wrapped_error_tail(m), m);
    }

    #[test]
    fn wrapped_error_tail_keeps_runtime_authored_tails_exact() {
        let m = "observe failed: external resource does not exist";
        assert_eq!(wrapped_error_tail(m), m);
        assert_ne!(
            wrapped_error_tail(m),
            wrapped_error_tail("observe failed: something else")
        );
    }

    #[test]
    fn wrapped_error_tail_masks_a_client_error_behind_runtime_prefixes() {
        assert_eq!(
            wrapped_error_tail("connect failed: cannot apply ProviderConfigUsage: Operation cannot be fulfilled on x: the object has been modified"),
            "connect failed: cannot apply ProviderConfigUsage: <wrapped error>"
        );
    }

    #[test]
    fn wrapped_error_tail_leaves_messages_without_a_runtime_prefix_exact() {
        for m in [
            "PUT /things/a: status 500",
            "Successfully requested creation of external resource",
            "something else: Operation cannot be fulfilled",
            "create failed",
        ] {
            assert_eq!(wrapped_error_tail(m), m);
        }
    }

    #[test]
    fn wrapped_error_tail_still_compares_the_prefix() {
        assert_ne!(
            wrapped_error_tail("create failed: dial tcp: connection refused"),
            wrapped_error_tail("update failed: dial tcp: connection refused")
        );
    }
}
