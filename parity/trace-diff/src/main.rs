use clap::Parser;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;
use trace_diff::diff::Layer;
use trace_diff::normalize::Rule;
use trace_diff::{allow, compare, render, run, Options};

/// Compare two parity run directories. Exit 0 when they are equal under the
/// normalization rules, 1 when they differ, 2 on bad input.
#[derive(Parser)]
struct Cli {
    run_a: PathBuf,
    run_b: PathBuf,
    /// JSON-lines file of reasoned exceptions.
    #[arg(long)]
    allow: Option<PathBuf>,
    /// Compare only these layers (chainsaw, audit, events, calls). Default: all.
    #[arg(long = "layer", value_parser = parse_layer)]
    layers: Vec<Layer>,
    /// Turn a normalization rule off, to see what it erases.
    #[arg(long = "disable-rule")]
    disabled: Vec<Rule>,
}

fn parse_layer(s: &str) -> Result<Layer, String> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|_| format!("unknown layer {s:?}"))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run_cli(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("trace-diff: {e}");
            ExitCode::from(2)
        }
    }
}

fn run_cli(cli: Cli) -> Result<bool, String> {
    let layers: BTreeSet<Layer> = if cli.layers.is_empty() {
        [Layer::Chainsaw, Layer::Audit, Layer::Events, Layer::Calls].into()
    } else {
        cli.layers.into_iter().collect()
    };
    let allowances = match &cli.allow {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
            allow::parse(&text).map_err(|e| format!("{}: {e}", p.display()))?
        }
        None => vec![],
    };
    let opts = Options {
        layers,
        disabled: cli.disabled.into_iter().collect(),
        allowances,
    };
    let load = |p: &PathBuf| run::load(p, opts.layers.contains(&Layer::Calls));
    let (a, b) = (load(&cli.run_a)?, load(&cli.run_b)?);
    println!("A: {}\nB: {}", cli.run_a.display(), cli.run_b.display());
    let outcome = compare(&a, &b, &opts);
    print!("{}", render(&outcome, &opts));
    Ok(outcome.equal())
}
