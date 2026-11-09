use crate::diff::{align, json_diff, Difference, Kind, Layer, Op};
use crate::normalize::{uid_token, Normalizer, Rule};
use crate::run::Write;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Maps each UID seen in a run to a token naming its object, so a UID token is
/// stable across runs and one extra object does not renumber the rest.
pub fn uid_map(writes: &[Write]) -> BTreeMap<String, String> {
    let mut uids = BTreeMap::new();
    let mut add = |uid: Option<&str>, kind: Option<&str>, ns: Option<&str>, name: Option<&str>| {
        if let (Some(uid), Some(kind), Some(name)) = (uid, kind, name) {
            uids.entry(uid.to_string()).or_insert_with(|| uid_token(kind, ns, name));
        }
    };
    for w in writes {
        let Some(o) = &w.request_object else { continue };
        let s = |v: &Value, p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
        let ns = s(o, "/metadata/namespace").or_else(|| w.namespace.clone());
        add(
            s(o, "/metadata/uid").as_deref(),
            s(o, "/kind").as_deref(),
            ns.as_deref(),
            s(o, "/metadata/name").as_deref(),
        );
        for owner in o
            .pointer("/metadata/ownerReferences")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            add(
                s(owner, "/uid").as_deref(),
                s(owner, "/kind").as_deref(),
                ns.as_deref(),
                s(owner, "/name").as_deref(),
            );
        }
        for p in ["/involvedObject", "/regarding"] {
            if let Some(r) = o.pointer(p) {
                let rns = s(r, "/namespace");
                add(
                    s(r, "/uid").as_deref(),
                    s(r, "/kind").as_deref(),
                    rns.as_deref(),
                    s(r, "/name").as_deref(),
                );
            }
        }
    }
    uids
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub verb: String,
    pub subresource: Option<String>,
    pub object: Value,
}

impl Entry {
    fn summary(&self) -> String {
        match &self.subresource {
            Some(s) => format!("{} {s}", self.verb),
            None => self.verb.clone(),
        }
    }
}

pub type Groups = BTreeMap<String, Vec<Entry>>;

pub fn group_key(w: &Write, norm: &Normalizer) -> String {
    let group = w.api_group.as_deref().filter(|g| !g.is_empty()).unwrap_or("core");
    let name = w.name.as_deref().map(|n| norm.name(n)).unwrap_or_default();
    match w.namespace.as_deref() {
        Some(ns) => format!("{group}/{} {ns}/{name}", w.resource),
        None => format!("{group}/{} {name}", w.resource),
    }
}

pub fn collapse_repeats<T: PartialEq>(mut v: Vec<T>, max_round: usize) -> Vec<T> {
    let mut changed = true;
    while changed {
        changed = false;
        for k in 1..=max_round.min(v.len() / 2) {
            let mut i = 0;
            while i + 2 * k <= v.len() {
                if v[i..i + k] == v[i + k..i + 2 * k] {
                    v.drain(i + k..i + 2 * k);
                    changed = true;
                } else {
                    i += 1;
                }
            }
        }
    }
    v
}

/// Only ProviderConfig groups showed a repeated reconcile round between runs.
/// On an MR the repeated rounds are retries, and their number is behaviour.
pub fn is_provider_config(group_key: &str) -> bool {
    group_key.split_once(' ').is_some_and(|(resource, _)| {
        resource.ends_with("/providerconfigs") || resource.ends_with("/clusterproviderconfigs")
    })
}

pub fn groups(writes: &[Write], norm: &Normalizer) -> Groups {
    let mut sorted: Vec<&Write> = writes.iter().filter(|w| w.succeeded() && !w.is_event()).collect();
    sorted.sort_by_key(|w| w.order_key());
    let mut out: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for w in sorted {
        out.entry(group_key(w, norm)).or_default().push(Entry {
            verb: w.verb.clone(),
            subresource: w.subresource.clone(),
            object: norm.object(w.request_object.clone().unwrap_or(Value::Null)),
        });
    }
    out.into_iter()
        .map(|(k, v)| {
            let max_round = if norm.on(Rule::CollapseRepeatedRounds) && is_provider_config(&k) {
                usize::MAX
            } else {
                1
            };
            (k, collapse_repeats(v, max_round))
        })
        .collect()
}

pub fn compare_groups(a: &Groups, b: &Groups) -> Vec<Difference> {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let mut out = Vec::new();
    let none = Vec::new();
    for key in keys {
        let (xs, ys) = (a.get(key).unwrap_or(&none), b.get(key).unwrap_or(&none));
        let d = |kind, a: String, b: String, detail| Difference {
            layer: Layer::Audit,
            key: key.clone(),
            kind,
            a,
            b,
            detail,
        };
        for op in align(xs, ys) {
            match op {
                Op::Same => {}
                Op::OnlyA(i) => out.push(d(
                    Kind::Missing,
                    format!("#{} {}", i + 1, xs[i].summary()),
                    "-".into(),
                    vec![],
                )),
                Op::OnlyB(j) => out.push(d(
                    Kind::Extra,
                    "-".into(),
                    format!("#{} {}", j + 1, ys[j].summary()),
                    vec![],
                )),
                Op::Pair(i, j) => {
                    let (x, y) = (&xs[i], &ys[j]);
                    let kind = if x.summary() == y.summary() {
                        Kind::Changed
                    } else {
                        Kind::Verb
                    };
                    out.push(d(
                        kind,
                        format!("#{} {}", i + 1, x.summary()),
                        format!("#{} {}", j + 1, y.summary()),
                        json_diff(&x.object, &y.object),
                    ))
                }
            }
        }
    }
    out
}

/// Event messages keyed by (involved object, type, reason). Counts, names and
/// timestamps are left out on purpose: client-go's correlator decides them.
pub type Events = BTreeMap<String, BTreeSet<String>>;

pub fn events(writes: &[Write], norm: &Normalizer) -> Events {
    let mut sorted: Vec<&Write> = writes.iter().filter(|w| w.succeeded() && w.is_event()).collect();
    sorted.sort_by_key(|w| w.order_key());
    let mut key_by_name: BTreeMap<(Option<String>, Option<String>), String> = BTreeMap::new();
    let mut out: Events = BTreeMap::new();
    for w in sorted {
        let Some(o) = &w.request_object else { continue };
        let s = |p: &str| o.pointer(p).and_then(Value::as_str);
        let id = (
            w.namespace.clone(),
            w.name.clone().or_else(|| s("/metadata/name").map(str::to_owned)),
        );
        if w.verb == "create" {
            let r = if o.get("regarding").is_some() {
                "/regarding"
            } else {
                "/involvedObject"
            };
            let field = |f: &str| s(&format!("{r}/{f}")).unwrap_or("").to_string();
            let key = format!(
                "{} {}/{} {} {}",
                field("kind"),
                field("namespace"),
                norm.name(&field("name")),
                s("/type").unwrap_or(""),
                s("/reason").unwrap_or("")
            );
            key_by_name.insert(id.clone(), key);
        }
        let Some(key) = key_by_name.get(&id) else { continue };
        let msg = s("/message").or_else(|| s("/note"));
        let set = out.entry(key.clone()).or_default();
        if let Some(m) = msg {
            set.insert(norm.message(m));
        }
    }
    out
}

pub fn compare_events(a: &Events, b: &Events) -> Vec<Difference> {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let fmt = |m: &BTreeSet<String>| m.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ");
    let mut out = Vec::new();
    for key in keys {
        let d = |kind, a, b| Difference {
            layer: Layer::Events,
            key: key.clone(),
            kind,
            a,
            b,
            detail: vec![],
        };
        match (a.get(key), b.get(key)) {
            (Some(x), None) => out.push(d(Kind::Missing, fmt(x), "-".into())),
            (None, Some(y)) => out.push(d(Kind::Extra, "-".into(), fmt(y))),
            (Some(x), Some(y)) if x != y => out.push(d(Kind::Changed, fmt(x), fmt(y))),
            _ => {}
        }
    }
    out
}

/// Failed writes are listed, never compared: Go's 409s come from informer
/// cache staleness that another client need not share.
pub fn failed_writes(writes: &[Write], norm: &Normalizer) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for w in writes.iter().filter(|w| !w.succeeded()) {
        let sub = w.subresource.as_deref().map(|s| format!(" {s}")).unwrap_or_default();
        *out.entry(format!("{} {}{sub} -> {}", group_key(w, norm), w.verb, w.code))
            .or_insert(0) += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn collapse_repeats_with_round_one_collapses_only_consecutive_identical_writes() {
        assert_eq!(collapse_repeats(vec![1, 1, 2, 1, 2, 2], 1), vec![1, 2, 1, 2]);
    }

    #[test]
    fn collapse_repeats_collapses_a_repeated_reconcile_round() {
        let go_fresh = vec![
            "M0", "S0", "M0", "S0", "M0", "S0", "M0", "S1", "M1", "S1", "M1", "S0", "M0", "S0", "Mx",
        ];
        let go_1 = vec![
            "M0", "S0", "M0", "S0", "M0", "S1", "M1", "S1", "M1", "S0", "M0", "S0", "Mx",
        ];
        let want = vec!["M0", "S0", "M0", "S1", "M1", "S0", "M0", "S0", "Mx"];
        assert_eq!(collapse_repeats(go_fresh, usize::MAX), want);
        assert_eq!(collapse_repeats(go_1, usize::MAX), want);
    }

    #[test]
    fn collapse_repeats_keeps_every_distinct_state() {
        assert_eq!(
            collapse_repeats(vec!["Creating", "Available", "Deleting"], usize::MAX),
            vec!["Creating", "Available", "Deleting"]
        );
    }

    #[test]
    fn only_provider_config_groups_collapse_repeated_rounds() {
        assert!(is_provider_config("example.com/providerconfigs ns/parity"));
        assert!(!is_provider_config("sample.example.com/mytypes ns/providerconfigs"));
    }

    #[test]
    fn uid_map_names_uids_by_their_object_and_owner_references() {
        let w: Write = serde_json::from_value(json!({
            "verb": "create", "apiGroup": "example.com", "resource": "providerconfigusages", "namespace": "ns",
            "name": "u1", "code": 201, "requestReceivedTimestamp": "2026-11-09T22:43:19Z",
            "requestObject": {"kind": "ProviderConfigUsage", "metadata": {"name": "u1",
                "ownerReferences": [{"kind": "MyType", "name": "a", "uid": "u1"}]}}
        }))
        .unwrap();
        assert_eq!(
            uid_map(&[w]),
            BTreeMap::from([("u1".to_string(), "<uid MyType ns/a>".to_string())])
        );
    }
}
