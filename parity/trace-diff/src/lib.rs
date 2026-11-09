pub mod allow;
pub mod audit;
pub mod calls;
pub mod chainsaw;
pub mod diff;
pub mod normalize;
pub mod run;

use allow::Allowance;
use diff::{Difference, Layer};
use normalize::{Normalizer, Rule};
use run::Run;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub struct Options {
    pub layers: BTreeSet<Layer>,
    pub disabled: BTreeSet<Rule>,
    pub allowances: Vec<Allowance>,
}

pub struct Outcome {
    pub differences: Vec<(Difference, Option<usize>)>,
    pub sizes: BTreeMap<Layer, [String; 2]>,
    pub allowances: Vec<Allowance>,
    pub failed_writes: [BTreeMap<String, usize>; 2],
}

impl Outcome {
    pub fn unallowed(&self) -> impl Iterator<Item = &Difference> {
        self.differences.iter().filter(|(_, a)| a.is_none()).map(|(d, _)| d)
    }

    pub fn equal(&self) -> bool {
        self.unallowed().next().is_none()
    }
}

pub fn compare(a: &Run, b: &Run, opts: &Options) -> Outcome {
    let norm = |r: &Run| Normalizer::new(opts.disabled.clone(), audit::uid_map(&r.writes));
    let (na, nb) = (norm(a), norm(b));
    let mut found = Vec::new();
    let mut sizes = BTreeMap::new();
    for layer in &opts.layers {
        let (diffs, size) = match layer {
            Layer::Chainsaw => {
                let n = |r: &Run| {
                    format!(
                        "{} tests, {} steps",
                        r.report.tests.len(),
                        r.report.tests.iter().map(|t| t.steps.len()).sum::<usize>()
                    )
                };
                (chainsaw::compare(&a.report, &b.report), [n(a), n(b)])
            }
            Layer::Audit => {
                let (x, y) = (audit::groups(&a.writes, &na), audit::groups(&b.writes, &nb));
                let n = |g: &audit::Groups| {
                    format!(
                        "{} objects, {} writes after collapse",
                        g.len(),
                        g.values().map(Vec::len).sum::<usize>()
                    )
                };
                (audit::compare_groups(&x, &y), [n(&x), n(&y)])
            }
            Layer::Events => {
                let (x, y) = (audit::events(&a.writes, &na), audit::events(&b.writes, &nb));
                let n = |e: &audit::Events| format!("{} event keys", e.len());
                (audit::compare_events(&x, &y), [n(&x), n(&y)])
            }
            Layer::Calls => {
                let (x, y) = (calls::things(&a.calls), calls::things(&b.calls));
                let n = |t: &BTreeMap<String, calls::Thing>| {
                    format!(
                        "{} things, {} mutating calls",
                        t.len(),
                        t.values().map(|t| t.mutations.len()).sum::<usize>()
                    )
                };
                (calls::compare(&x, &y), [n(&x), n(&y)])
            }
        };
        found.extend(diffs);
        sizes.insert(*layer, size);
    }
    let differences = found
        .into_iter()
        .map(|d| {
            let allowed = opts.allowances.iter().position(|al| al.matches(&d));
            (d, allowed)
        })
        .collect();
    Outcome {
        differences,
        sizes,
        allowances: opts.allowances.clone(),
        failed_writes: [
            audit::failed_writes(&a.writes, &na),
            audit::failed_writes(&b.writes, &nb),
        ],
    }
}

pub fn render(o: &Outcome, opts: &Options) -> String {
    let mut s = String::new();
    let disabled: Vec<_> = opts.disabled.iter().map(|r| r.name()).collect();
    let on: Vec<_> = Rule::ALL
        .iter()
        .filter(|r| !opts.disabled.contains(r))
        .map(|r| r.name())
        .collect();
    let _ = writeln!(s, "rules on: {}", on.join(", "));
    if !disabled.is_empty() {
        let _ = writeln!(s, "rules off: {}", disabled.join(", "));
    }
    for layer in &opts.layers {
        let here: Vec<_> = o.differences.iter().filter(|(d, _)| d.layer == *layer).collect();
        let verdict = if here.is_empty() {
            "equal".to_string()
        } else {
            format!("{} difference(s)", here.len())
        };
        let _ = writeln!(s, "\n== {layer}: {verdict}");
        if let Some([x, y]) = o.sizes.get(layer) {
            let _ = writeln!(s, "  compared A: {x}; B: {y}");
        }
        for (d, allowed) in here {
            let tag = allowed
                .map(|i| format!(" [allowed by allowance {}]", i + 1))
                .unwrap_or_default();
            let _ = writeln!(s, "  {} | {}{tag}", d.key, d.kind);
            let _ = writeln!(s, "      A: {}", d.a);
            let _ = writeln!(s, "      B: {}", d.b);
            for line in &d.detail {
                let _ = writeln!(s, "         {line}");
            }
        }
    }
    if opts.layers.contains(&Layer::Audit) {
        let _ = writeln!(s, "\n== failed writes (listed, not compared)");
        for (side, writes) in ["A", "B"].iter().zip(&o.failed_writes) {
            if writes.is_empty() {
                let _ = writeln!(s, "  {side}: none");
            }
            for (k, n) in writes {
                let _ = writeln!(s, "  {side}: {k} x{n}");
            }
        }
    }
    if !o.allowances.is_empty() {
        let _ = writeln!(s, "\n== allowances");
        for (i, al) in o.allowances.iter().enumerate() {
            let used = o.differences.iter().filter(|(_, a)| *a == Some(i)).count();
            let state = if used == 0 {
                "UNUSED".to_string()
            } else {
                format!("used {used}x")
            };
            let _ = writeln!(
                s,
                "  {}: {state}: {} {} {}: {}",
                i + 1,
                al.layer,
                al.key,
                al.kind,
                al.reason
            );
        }
    }
    let n = o.unallowed().count();
    let allowed = o.differences.len() - n;
    let _ = writeln!(
        s,
        "\nresult: {} ({n} difference(s), {allowed} allowed)",
        if n == 0 { "EQUAL" } else { "DIFFERENT" }
    );
    s
}
