use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use super::Gate;

const SETTLE: Duration = Duration::from_millis(10);
const WAIT: Duration = Duration::from_millis(500);

fn signal(tx: &Sender<()>) -> impl FnOnce() + Send + 'static + use<> {
    let tx = tx.clone();
    move || tx.send(()).unwrap()
}

fn fired(rx: &Receiver<()>, wait: Duration) -> bool {
    rx.recv_timeout(wait).is_ok()
}

fn wait_for(want_called: bool) -> Duration {
    if want_called { WAIT } else { SETTLE }
}

struct RegisterCase {
    name: &'static str,
    depends: Vec<&'static str>,
    want_called: bool,
}

#[test]
fn register_calls_the_function_only_when_there_is_nothing_to_wait_for() {
    type Case = RegisterCase;
    let cases = [
        // go: pkg/gate TestGateRegister/NoDependencies
        Case {
            name: "NoDependencies",
            depends: vec![],
            want_called: true,
        },
        // go: pkg/gate TestGateRegister/SingleDependency
        Case {
            name: "SingleDependency",
            depends: vec!["condition1"],
            want_called: false,
        },
        // go: pkg/gate TestGateRegister/MultipleDependencies
        Case {
            name: "MultipleDependencies",
            depends: vec!["condition1", "condition2"],
            want_called: false,
        },
    ];
    for tc in cases {
        let g = Gate::<&str>::default();
        let (tx, rx) = mpsc::channel();
        g.register(signal(&tx), tc.depends);
        assert_eq!(fired(&rx, wait_for(tc.want_called)), tc.want_called, "{}", tc.name);
    }
}

struct IntegrationCase {
    name: &'static str,
    setup: fn(&Gate<&'static str>, &Sender<()>),
    want_called: bool,
}

#[test]
fn integration_cases() {
    type Case = IntegrationCase;
    let cases = [
        // go: pkg/gate TestGateIntegration/SingleDependencyMet
        Case {
            name: "SingleDependencyMet",
            setup: |g, tx| {
                g.register(signal(tx), ["condition1"]);
                g.set("condition1", true);
            },
            want_called: true,
        },
        // go: pkg/gate TestGateIntegration/MultipleDependenciesMet
        Case {
            name: "MultipleDependenciesMet",
            setup: |g, tx| {
                g.register(signal(tx), ["condition1", "condition2"]);
                g.set("condition1", true);
                g.set("condition2", true);
            },
            want_called: true,
        },
        // go: pkg/gate TestGateIntegration/PartialDependenciesMet
        Case {
            name: "PartialDependenciesMet",
            setup: |g, tx| {
                g.register(signal(tx), ["condition1", "condition2"]);
                g.set("condition1", true);
            },
            want_called: false,
        },
        // go: pkg/gate TestGateIntegration/DependenciesAlreadyMet
        Case {
            name: "DependenciesAlreadyMet",
            setup: |g, tx| {
                g.set("condition1", true);
                g.set("condition2", true);
                g.register(signal(tx), ["condition1", "condition2"]);
            },
            want_called: true,
        },
        // go: pkg/gate TestGateIntegration/DependencySetThenUnset
        Case {
            name: "DependencySetThenUnset",
            setup: |g, tx| {
                g.register(signal(tx), ["condition1"]);
                g.set("condition1", true);
                g.set("condition1", false);
            },
            want_called: true,
        },
        // go: pkg/gate TestGateIntegration/FunctionCalledOnlyOnce
        Case {
            name: "FunctionCalledOnlyOnce",
            setup: |g, tx| {
                g.register(signal(tx), ["condition1"]);
                g.set("condition1", true);
                g.set("condition1", false);
                g.set("condition1", true);
            },
            want_called: true,
        },
    ];
    for tc in cases {
        let g = Gate::<&str>::default();
        let (tx, rx) = mpsc::channel();
        (tc.setup)(&g, &tx);
        assert_eq!(fired(&rx, wait_for(tc.want_called)), tc.want_called, "{}", tc.name);
        if tc.name == "FunctionCalledOnlyOnce" {
            assert!(!fired(&rx, Duration::from_millis(50)), "called more than once");
        }
    }
}

// go: pkg/gate TestGateConcurrency
#[test]
fn concurrent_registrations_all_fire_once_the_shared_condition_is_set() {
    const N: usize = 100;
    let g = Arc::new(Gate::<&str>::default());
    let (tx, rx) = mpsc::channel();
    let workers: Vec<_> = (0..N)
        .map(|_| {
            let (g, tx) = (g.clone(), tx.clone());
            std::thread::spawn(move || g.register(signal(&tx), ["shared-condition"]))
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    g.set("shared-condition", true);
    drop(tx);
    let count = std::iter::from_fn(|| rx.recv_timeout(WAIT).ok()).count();
    assert_eq!(count, N, "function calls");
}

// go: pkg/gate TestGateTypeSafety
#[test]
fn any_hashable_key_type_works() {
    let g = Gate::<i32>::default();
    let (tx, rx) = mpsc::channel();
    g.register(signal(&tx), [1, 2, 3]);
    g.set(1, true);
    g.set(2, true);
    g.set(3, true);
    assert!(
        fired(&rx, WAIT),
        "function should have been called when all int conditions were met"
    );
}

#[test]
fn set_reports_whether_the_value_changed() {
    let g = Gate::<&str>::default();
    assert!(g.set("a", false), "absent counts as changed, even when set false");
    assert!(!g.set("a", false));
    assert!(g.set("a", true));
    assert!(!g.set("a", true));
    assert!(g.set("a", false));
}

#[test]
fn a_key_set_true_then_false_does_not_satisfy_a_later_registration() {
    let g = Gate::<&str>::default();
    g.set("a", true);
    g.set("a", false);
    let (tx, rx) = mpsc::channel();
    g.register(signal(&tx), ["a"]);
    assert!(!fired(&rx, SETTLE));
    g.set("a", true);
    assert!(fired(&rx, WAIT), "fires when the key is true again");
}

#[test]
fn a_released_registration_is_forgotten_so_a_later_flip_does_not_rerun_it() {
    let g = Gate::<&str>::default();
    let (tx, rx) = mpsc::channel();
    g.register(signal(&tx), ["a"]);
    g.set("a", true);
    assert!(fired(&rx, WAIT));
    g.set("a", false);
    g.set("a", true);
    assert!(!fired(&rx, Duration::from_millis(50)));
}

/// Several tasks race to set keys while registrations are added. Every registration fires
/// exactly once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tasks_setting_keys_concurrently_release_each_registration_exactly_once() {
    const KEYS: usize = 8;
    const REGS: usize = 40;
    let g = Arc::new(Gate::<usize>::default());
    let (tx, rx) = mpsc::channel::<usize>();

    let mut tasks = Vec::new();
    for r in 0..REGS {
        let (g, tx) = (g.clone(), tx.clone());
        tasks.push(tokio::spawn(async move {
            g.register(move || tx.send(r).unwrap(), [r % KEYS, (r + 3) % KEYS]);
        }));
    }
    for k in 0..KEYS {
        let g = g.clone();
        tasks.push(tokio::spawn(async move {
            g.set(k, true);
            g.set(k, false);
            g.set(k, true);
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    drop(tx);

    let mut got: Vec<usize> =
        tokio::task::spawn_blocking(move || std::iter::from_fn(|| rx.recv_timeout(WAIT).ok()).collect())
            .await
            .unwrap();
    got.sort_unstable();
    assert_eq!(
        got,
        (0..REGS).collect::<Vec<_>>(),
        "each registration fires exactly once"
    );
}
