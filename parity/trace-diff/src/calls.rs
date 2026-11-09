use crate::diff::{align, Difference, Kind, Layer, Op};
use crate::run::Call;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Actor {
    Provider,
    Scenario,
}

/// The scenarios reach the fake service through the API server's service
/// proxy, which adds X-Forwarded-For. The provider calls it directly.
pub fn actor(c: &Call) -> Actor {
    if c.forwarded_for.as_deref().is_some_and(|f| !f.is_empty()) {
        Actor::Scenario
    } else {
        Actor::Provider
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mutation {
    pub actor: Actor,
    pub method: String,
    pub status: u16,
    pub fault: bool,
    pub body: Option<Value>,
}

impl Mutation {
    fn summary(&self) -> String {
        let who = match self.actor {
            Actor::Provider => "provider",
            Actor::Scenario => "scenario",
        };
        let fault = if self.fault { " (injected fault)" } else { "" };
        let body = self.body.as_ref().map(|b| format!(" {b}")).unwrap_or_default();
        format!("{who} {} {}{fault}{body}", self.method, self.status)
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Thing {
    pub mutations: Vec<Mutation>,
    pub provider_get_statuses: BTreeSet<u16>,
}

/// A GET counts as a mutation when a fault failed it, so an injected fault is
/// always compared in order.
pub fn things(calls: &[Call]) -> BTreeMap<String, Thing> {
    let mut sorted: Vec<&Call> = calls.iter().collect();
    sorted.sort_by_key(|c| c.seq);
    let mut out: BTreeMap<String, Thing> = BTreeMap::new();
    for c in sorted {
        let thing = out
            .entry(c.path.trim_start_matches("/things/").to_string())
            .or_default();
        let who = actor(c);
        if c.method == "GET" && !c.fault {
            if who == Actor::Provider {
                thing.provider_get_statuses.insert(c.status);
            }
            continue;
        }
        thing.mutations.push(Mutation {
            actor: who,
            method: c.method.clone(),
            status: c.status,
            fault: c.fault,
            body: c.body.clone(),
        });
    }
    out
}

pub fn compare(a: &BTreeMap<String, Thing>, b: &BTreeMap<String, Thing>) -> Vec<Difference> {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let empty = Thing::default();
    let mut out = Vec::new();
    for name in keys {
        let (x, y) = (a.get(name).unwrap_or(&empty), b.get(name).unwrap_or(&empty));
        let key = format!("thing {name}");
        let d = |kind, a, b| Difference {
            layer: Layer::Calls,
            key: key.clone(),
            kind,
            a,
            b,
            detail: vec![],
        };
        let (xs, ys) = (&x.mutations, &y.mutations);
        for op in align(xs, ys) {
            match op {
                Op::Same => {}
                Op::OnlyA(i) => out.push(d(Kind::Missing, format!("#{} {}", i + 1, xs[i].summary()), "-".into())),
                Op::OnlyB(j) => out.push(d(Kind::Extra, "-".into(), format!("#{} {}", j + 1, ys[j].summary()))),
                Op::Pair(i, j) => out.push(d(
                    Kind::Changed,
                    format!("#{} {}", i + 1, xs[i].summary()),
                    format!("#{} {}", j + 1, ys[j].summary()),
                )),
            }
        }
        if x.provider_get_statuses != y.provider_get_statuses {
            let fmt = |s: &BTreeSet<u16>| format!("provider GET statuses {s:?}");
            out.push(Difference {
                layer: Layer::Calls,
                key: format!("thing {name} GET"),
                kind: Kind::Changed,
                a: fmt(&x.provider_get_statuses),
                b: fmt(&y.provider_get_statuses),
                detail: vec![],
            });
        }
    }
    out
}
