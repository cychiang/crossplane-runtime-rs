use serde_json::Value;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Chainsaw,
    Audit,
    Events,
    Calls,
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Layer::Chainsaw => "chainsaw",
            Layer::Audit => "audit",
            Layer::Events => "events",
            Layer::Calls => "calls",
        })
    }
}

/// `Missing` is present in run A only, `Extra` in run B only. `Verb` is a
/// changed audit write whose verb or subresource differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Missing,
    Extra,
    Changed,
    Verb,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Missing => "only in A",
            Kind::Extra => "only in B",
            Kind::Changed => "changed",
            Kind::Verb => "verb changed",
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    pub layer: Layer,
    pub key: String,
    pub kind: Kind,
    pub a: String,
    pub b: String,
    pub detail: Vec<String>,
}

pub enum Op {
    Same,
    OnlyA(usize),
    OnlyB(usize),
    Pair(usize, usize),
}

/// Longest-common-subsequence alignment. A run of A-only entries directly
/// followed by B-only entries is paired up positionally, so one changed write
/// reads as one change instead of a removal and an insertion.
pub fn align<T: PartialEq>(a: &[T], b: &[T]) -> Vec<Op> {
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut raw = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            raw.push(Op::Same);
            i += 1;
            j += 1;
        } else if j < m && (i == n || lcs[i][j + 1] >= lcs[i + 1][j]) {
            raw.push(Op::OnlyB(j));
            j += 1;
        } else {
            raw.push(Op::OnlyA(i));
            i += 1;
        }
    }
    let mut out = Vec::new();
    let mut k = 0;
    while k < raw.len() {
        let start = k;
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        while k < raw.len() && !matches!(raw[k], Op::Same) {
            match raw[k] {
                Op::OnlyA(i) => xs.push(i),
                Op::OnlyB(j) => ys.push(j),
                _ => unreachable!(),
            }
            k += 1;
        }
        if start == k {
            out.push(Op::Same);
            k += 1;
            continue;
        }
        let paired = xs.len().min(ys.len());
        for p in 0..paired {
            out.push(Op::Pair(xs[p], ys[p]));
        }
        out.extend(xs[paired..].iter().map(|&i| Op::OnlyA(i)));
        out.extend(ys[paired..].iter().map(|&j| Op::OnlyB(j)));
    }
    out
}

pub fn json_diff(a: &Value, b: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk("", a, b, &mut out);
    out
}

fn walk(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let p = format!("{path}.{k}");
                match (x.get(k), y.get(k)) {
                    (Some(u), Some(v)) => walk(&p, u, v, out),
                    (Some(u), None) => out.push(format!("{p}: {u} -> (absent)")),
                    (None, Some(v)) => out.push(format!("{p}: (absent) -> {v}")),
                    (None, None) => unreachable!(),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                walk(&format!("{path}[{i}]"), u, v, out);
            }
        }
        _ if a != b => out.push(format!("{}: {a} -> {b}", if path.is_empty() { "." } else { path })),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn shape(ops: &[Op]) -> String {
        ops.iter()
            .map(|o| match o {
                Op::Same => "=".to_string(),
                Op::OnlyA(i) => format!("-{i}"),
                Op::OnlyB(j) => format!("+{j}"),
                Op::Pair(i, j) => format!("~{i}{j}"),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn align_pairs_a_changed_entry() {
        assert_eq!(shape(&align(&[1, 2, 3], &[1, 9, 3])), "= ~11 =");
    }

    #[test]
    fn align_reports_missing_and_extra_entries() {
        assert_eq!(shape(&align(&[1, 2, 3], &[1, 3])), "= -1 =");
        assert_eq!(shape(&align(&[1, 3], &[1, 2, 3])), "= +1 =");
    }

    #[test]
    fn json_diff_names_the_changed_path() {
        let a = json!({"status": {"conditions": [{"reason": "Creating"}]}, "x": 1});
        let b = json!({"status": {"conditions": [{"reason": "Available"}]}, "y": 1});
        assert_eq!(
            json_diff(&a, &b),
            vec![
                r#".status.conditions[0].reason: "Creating" -> "Available""#.to_string(),
                ".x: 1 -> (absent)".to_string(),
                ".y: (absent) -> 1".to_string(),
            ]
        );
    }
}
