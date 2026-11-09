use crate::diff::{align, Difference, Kind, Layer, Op};
use crate::run::{Report, Test};
use std::collections::{BTreeMap, BTreeSet};

pub fn compare(a: &Report, b: &Report) -> Vec<Difference> {
    let xa: BTreeMap<&String, &Test> = a.tests.iter().map(|t| (&t.name, t)).collect();
    let xb: BTreeMap<&String, &Test> = b.tests.iter().map(|t| (&t.name, t)).collect();
    let names: BTreeSet<&String> = xa.keys().chain(xb.keys()).copied().collect();
    let mut out = Vec::new();
    for name in names {
        let key = format!("test {name}");
        let d = |kind, a: String, b: String| Difference {
            layer: Layer::Chainsaw,
            key: key.clone(),
            kind,
            a,
            b,
            detail: vec![],
        };
        let (x, y) = match (xa.get(name), xb.get(name)) {
            (Some(x), Some(y)) => (x, y),
            (Some(x), None) => {
                out.push(d(Kind::Missing, x.status.clone(), "-".into()));
                continue;
            }
            (None, Some(y)) => {
                out.push(d(Kind::Extra, "-".into(), y.status.clone()));
                continue;
            }
            (None, None) => unreachable!(),
        };
        if x.status != y.status {
            out.push(d(Kind::Changed, x.status.clone(), y.status.clone()));
        }
        let steps = |t: &Test| {
            t.steps
                .iter()
                .map(|s| format!("{}: {}", s.name, s.status))
                .collect::<Vec<_>>()
        };
        let (sx, sy) = (steps(x), steps(y));
        for op in align(&sx, &sy) {
            match op {
                Op::Same => {}
                Op::OnlyA(i) => out.push(d(Kind::Missing, format!("step {}", sx[i]), "-".into())),
                Op::OnlyB(j) => out.push(d(Kind::Extra, "-".into(), format!("step {}", sy[j]))),
                Op::Pair(i, j) => out.push(d(Kind::Changed, format!("step {}", sx[i]), format!("step {}", sy[j]))),
            }
        }
    }
    out
}
