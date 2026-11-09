use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn recorded_run() -> Option<PathBuf> {
    let runs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.work/runs");
    let label = std::env::var("TRACE_DIFF_RUN").unwrap_or_else(|_| "go-a".into());
    let dir = runs.join(label);
    dir.join("audit.jsonl").exists().then_some(dir)
}

#[test]
fn one_edited_condition_reason_in_a_real_run_is_the_only_difference_reported() {
    let Some(run) = recorded_run() else {
        eprintln!("skipped: no recorded run under parity/.work/runs (set TRACE_DIFF_RUN to pick one)");
        return;
    };
    let copy = std::env::temp_dir().join(format!("trace-diff-mutation-{}", std::process::id()));
    let _ = fs::remove_dir_all(&copy);
    fs::create_dir_all(&copy).unwrap();
    for f in ["audit.jsonl", "calls.jsonl", "chainsaw-report.json"] {
        fs::copy(run.join(f), copy.join(f)).unwrap();
    }

    let audit = fs::read_to_string(copy.join("audit.jsonl")).unwrap();
    let target = audit
        .lines()
        .position(|l| {
            l.contains(r#""resource":"mytypes""#)
                && l.contains(r#""subresource":"status""#)
                && l.contains(r#""namespace":"parity-lifecycle""#)
                && l.contains(r#""reason":"Available""#)
        })
        .expect("a lifecycle status write with reason Available");
    let edited: Vec<String> = audit
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i == target {
                l.replacen(r#""reason":"Available""#, r#""reason":"Unavailable""#, 1)
            } else {
                l.to_string()
            }
        })
        .collect();
    fs::write(copy.join("audit.jsonl"), edited.join("\n") + "\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_trace-diff"))
        .arg(&run)
        .arg(&copy)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let _ = fs::remove_dir_all(&copy);
    println!("{text}");
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(
        text.contains("result: DIFFERENT (1 difference(s), 0 allowed)"),
        "{text}"
    );
    assert!(
        text.contains("sample.example.com/mytypes parity-lifecycle/parity-lifecycle | changed"),
        "{text}"
    );
    let details: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("         "))
        .map(str::trim)
        .collect();
    assert_eq!(
        details,
        vec![r#".status.conditions[0].reason: "Available" -> "Unavailable""#],
        "{text}"
    );
}

#[test]
fn a_real_run_equals_itself() {
    let Some(run) = recorded_run() else {
        eprintln!("skipped: no recorded run under parity/.work/runs");
        return;
    };
    let out = Command::new(env!("CARGO_BIN_EXE_trace-diff"))
        .arg(&run)
        .arg(&run)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
}
