use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use watcher::Event::{Apply, Delete, Init, InitApply, InitDone};

use super::*;

fn cm(name: &str, rv: &str) -> Arc<dyn Object> {
    Arc::new(ConfigMap {
        metadata: ObjectMeta {
            name: Some(name.into()),
            namespace: Some("ns".into()),
            resource_version: Some(rv.into()),
            ..Default::default()
        },
        ..Default::default()
    })
}

/// An event as `(kind, name, old rv, new rv)`, so cases compare plain tuples.
fn show(event: &InformerEvent) -> (&'static str, String, String, String) {
    let name = |o: &Arc<dyn Object>| o.meta().name.clone().unwrap();
    let rv = |o: &Arc<dyn Object>| o.meta().resource_version.clone().unwrap();
    match event {
        InformerEvent::Create(o) => ("create", name(o), String::new(), rv(o)),
        InformerEvent::Update { old, new } => ("update", name(new), rv(old), rv(new)),
        InformerEvent::Delete(o) => ("delete", name(o), rv(o), String::new()),
    }
}

fn drain(rx: &mut mpsc::UnboundedReceiver<InformerEvent>) -> Vec<(&'static str, String, String, String)> {
    std::iter::from_fn(|| rx.try_recv().ok()).map(|e| show(&e)).collect()
}

fn ev(kind: &'static str, name: &str, old: &str, new: &str) -> (&'static str, String, String, String) {
    (kind, name.into(), old.into(), new.into())
}

#[test]
fn the_first_list_is_creates_and_marks_the_store_synced() {
    let mut store = Store::default();
    let mut rx = store.subscribe();
    assert!(!store.apply(Init));
    assert!(!store.apply(InitApply(cm("a", "1"))));
    assert!(!store.apply(InitApply(cm("b", "1"))));
    assert!(drain(&mut rx).is_empty(), "nothing is sent before the list is complete");
    assert!(store.apply(InitDone), "InitDone completes the list");
    assert_eq!(drain(&mut rx), [ev("create", "a", "", "1"), ev("create", "b", "", "1")]);
}

#[test]
fn watch_events_carry_the_old_object() {
    let mut store = Store::default();
    store.apply(Init);
    store.apply(InitApply(cm("a", "1")));
    store.apply(InitDone);
    let mut rx = store.subscribe();
    store.apply(Apply(cm("a", "2")));
    store.apply(Apply(cm("c", "3")));
    store.apply(Delete(cm("a", "4")));
    assert_eq!(
        drain(&mut rx),
        [
            ev("create", "a", "", "1"),
            ev("update", "a", "1", "2"),
            ev("create", "c", "", "3"),
            ev("delete", "a", "2", ""),
        ]
    );
}

#[test]
fn a_relist_deletes_what_vanished_with_its_last_known_state_first() {
    let mut store = Store::default();
    store.apply(Init);
    for name in ["a", "b", "c"] {
        store.apply(InitApply(cm(name, "1")));
    }
    store.apply(InitDone);
    let mut rx = store.subscribe();
    drain(&mut rx);
    store.apply(Apply(cm("b", "5")));
    drain(&mut rx);

    // The watch dropped; while it was down `b` was deleted and `d` created.
    store.apply(Init);
    store.apply(InitApply(cm("c", "1")));
    store.apply(InitApply(cm("a", "7")));
    store.apply(InitApply(cm("d", "8")));
    assert!(drain(&mut rx).is_empty(), "a relist in progress changes nothing yet");
    assert!(store.get(&ObjectKey::namespaced("ns", "b")).is_some());
    store.apply(InitDone);
    assert_eq!(
        drain(&mut rx),
        [
            ev("delete", "b", "5", ""),
            ev("update", "c", "1", "1"),
            ev("update", "a", "1", "7"),
            ev("create", "d", "", "8"),
        ],
        "deletions first, then the list in order: client-go processReplacedAllInfo"
    );
    assert!(store.get(&ObjectKey::namespaced("ns", "b")).is_none());
}

#[test]
fn resync_replays_every_object_as_an_update_to_itself() {
    let mut store = Store::default();
    store.apply(Init);
    store.apply(InitApply(cm("a", "1")));
    store.apply(InitApply(cm("b", "2")));
    store.apply(InitDone);
    let mut rx = store.subscribe();
    drain(&mut rx);
    store.resync();
    assert_eq!(
        drain(&mut rx),
        [ev("update", "a", "1", "1"), ev("update", "b", "2", "2")]
    );
}

#[test]
fn a_late_subscriber_gets_the_store_then_live_events() {
    let mut store = Store::default();
    store.apply(Init);
    store.apply(InitApply(cm("a", "1")));
    store.apply(InitDone);
    let mut early = store.subscribe();
    store.apply(Apply(cm("b", "2")));
    let mut late = store.subscribe();
    store.apply(Apply(cm("a", "3")));
    assert_eq!(
        drain(&mut late),
        [
            ev("create", "a", "", "1"),
            ev("create", "b", "", "2"),
            ev("update", "a", "1", "3")
        ]
    );
    assert_eq!(drain(&mut early).len(), 3);
}

#[test]
fn a_subscriber_that_falls_behind_loses_nothing() {
    let mut store = Store::default();
    store.apply(Init);
    store.apply(InitDone);
    let mut slow = store.subscribe();
    let mut fast = store.subscribe();
    let n = 20_000;
    for i in 0..n {
        store.apply(Apply(cm(&format!("o{}", i % 7), &i.to_string())));
        if i % 100 == 0 {
            drain(&mut fast);
        }
    }
    let got = drain(&mut slow);
    assert_eq!(got.len(), n, "every event is queued for the subscriber that never read");
    let in_order = got.iter().enumerate().all(|(i, e)| e.3 == i.to_string());
    assert!(in_order, "in order");
}

#[test]
fn a_closed_subscriber_is_dropped() {
    let mut store = Store::default();
    let rx = store.subscribe();
    drop(rx);
    store.apply(Apply(cm("a", "1")));
    assert!(store.subscribers.is_empty());
}

#[test]
fn list_filters_by_namespace_and_labels_in_key_order() {
    let mut store = Store::default();
    let labelled = |ns: &str, name: &str, team: &str| -> Arc<dyn Object> {
        Arc::new(ConfigMap {
            metadata: ObjectMeta {
                name: Some(name.into()),
                namespace: Some(ns.into()),
                labels: Some([("team".to_owned(), team.to_owned())].into()),
                ..Default::default()
            },
            ..Default::default()
        })
    };
    store.apply(Init);
    for obj in [
        labelled("y", "b", "t1"),
        labelled("x", "c", "t1"),
        labelled("x", "a", "t2"),
    ] {
        store.apply(InitApply(obj));
    }
    store.apply(InitDone);
    let names = |options: ListOptions| -> Vec<String> {
        store.list(&options).iter().map(|o| format!("{:?}", o.key())).collect()
    };
    let key = |ns: &str, n: &str| format!("{:?}", ObjectKey::namespaced(ns, n));
    assert_eq!(
        names(ListOptions::default()),
        [key("x", "a"), key("x", "c"), key("y", "b")]
    );
    assert_eq!(
        names(ListOptions {
            namespace: Some("x".into()),
            ..Default::default()
        }),
        [key("x", "a"), key("x", "c")]
    );
    assert_eq!(
        names(ListOptions {
            namespace: None,
            labels: [("team".to_owned(), "t1".to_owned())].into(),
        }),
        [key("x", "c"), key("y", "b")]
    );
}
