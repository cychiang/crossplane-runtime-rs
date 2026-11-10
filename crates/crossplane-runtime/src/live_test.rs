//! The manager's parts against a real API server. Each test returns early with a message unless
//! `XPRS_TEST_KUBECONFIG` names a kubeconfig. Each works in throwaway `xprs-` namespaces it creates
//! and deletes. With `XPRS_TEST_AUDIT_LOG` naming the kube-apiserver audit log, the client test
//! also checks the User-Agent the server recorded.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use crossplane_core::client::Client;
use crossplane_core::resource::ObjectKey;
use futures::FutureExt;
use k8s_openapi::api::core::v1::{ConfigMap, Namespace};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::config::{KubeConfigOptions, Kubeconfig};

use crate::kube_client::KubeClient;
use crate::manager::RestConfig;

pub(crate) async fn rest_config(test: &str) -> Option<RestConfig> {
    let Some(path) = std::env::var_os("XPRS_TEST_KUBECONFIG") else {
        eprintln!("skipping {test}: set XPRS_TEST_KUBECONFIG to a kubeconfig");
        return None;
    };
    let kubeconfig = Kubeconfig::read_from(&path).expect("readable kubeconfig");
    let config = kube::Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
        .await
        .expect("usable kubeconfig");
    Some(RestConfig::from_config(config, "xprs-test", "v0.0.0"))
}

pub(crate) fn namespace(name: &str) -> Namespace {
    Namespace {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Runs `body` in a fresh namespace named `xprs-<prefix>-<pid>`, deleting it afterwards even when
/// `body` panics.
pub(crate) async fn in_namespace<F, Fut>(client: &dyn Client, prefix: &str, body: F)
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = ()>,
{
    let ns = format!("xprs-{prefix}-{}", std::process::id());
    let mut namespace = namespace(&ns);
    client.create(&mut namespace).await.expect("create the test namespace");
    let outcome = AssertUnwindSafe(body(ns.clone())).catch_unwind().await;
    let cleanup = client.delete(&namespace).await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    cleanup.expect("delete the test namespace");
}

fn config_map(ns: &str, name: &str) -> ConfigMap {
    ConfigMap {
        metadata: ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(ns.to_owned()),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// `n` concurrent direct reads of one object; returns how long they all took.
async fn burst_of_reads(client: &KubeClient, key: &ObjectKey, n: usize) -> Duration {
    let client: &dyn Client = client;
    let started = Instant::now();
    let reads = (0..n).map(|_| client.get_as::<ConfigMap>(key));
    for read in futures::future::join_all(reads).await {
        read.expect("get");
    }
    started.elapsed()
}

#[tokio::test]
async fn rest_config_throttles_and_sends_the_user_agent() {
    let Some(config) = rest_config("rest_config_throttles_and_sends_the_user_agent").await else {
        return;
    };
    let user_agent = config.user_agent().to_owned();
    let fast = KubeClient::uncached(config.clone().throttled(1000.0, 1000).into_kube_client().unwrap());
    let (qps, burst, n) = (5.0, 10, 30);
    let slow = KubeClient::uncached(config.clone().throttled(qps, burst).into_kube_client().unwrap());

    let (fast, slow) = (&fast, &slow);
    in_namespace(fast, "throttle", |ns| async move {
        let mut cm = config_map(&ns, "target");
        (fast as &dyn Client).create(&mut cm).await.expect("create");
        let key = ObjectKey::namespaced(&ns, "target");

        let unthrottled = burst_of_reads(fast, &key, n).await;
        let throttled = burst_of_reads(slow, &key, n).await;
        let floor = Duration::from_secs_f32((n - burst as usize) as f32 / qps);
        eprintln!(
            "throttle: {n} reads, unthrottled {unthrottled:?}, at qps {qps} burst {burst} {throttled:?}, floor {floor:?}"
        );
        assert!(throttled >= floor, "{throttled:?} is under the floor {floor:?}");
        assert!(
            throttled < floor + Duration::from_secs(2),
            "{throttled:?} is far over the floor {floor:?}: something else limits it"
        );
        assert!(unthrottled < floor / 4, "the control is not limited by the server: {unthrottled:?}");
    })
    .await;

    if let Some(audit) = std::env::var_os("XPRS_TEST_AUDIT_LOG") {
        let ns = format!("xprs-throttle-{}", std::process::id());
        let mut seen = Vec::new();
        for _ in 0..20 {
            let log = std::fs::read_to_string(&audit).expect("readable audit log");
            seen = log
                .lines()
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                .filter(|e| e["objectRef"]["namespace"] == ns.as_str() && e["verb"] == "create")
                .map(|e| e["userAgent"].as_str().unwrap_or_default().to_owned())
                .collect();
            if !seen.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        eprintln!("audit: creates in {ns} carried User-Agent {seen:?}");
        assert!(!seen.is_empty(), "no create in {ns} in the audit log");
        assert!(seen.iter().all(|ua| *ua == user_agent), "{seen:?} != {user_agent}");
    }
}

/// The Go baseline wrote `CreatedExternalResource`-style events as one POST of a core/v1 Event
/// named `<object>.<hex nanos>` and then a strategic-merge PATCH of `count`, `lastTimestamp` and
/// `message` per repeat (`parity/.work/runs/go-a/audit.jsonl`). Twelve identical events must give
/// one Event with count 12: under the spam burst of 25, and one message so nothing aggregates.
#[tokio::test]
async fn the_recorder_writes_one_event_and_patches_its_count() {
    use crate::event::{ApiRecorder, KubeEventSink, MAX_QUEUED_EVENTS, run_broadcaster};
    use k8s_openapi::api::core::v1::Event as CoreEvent;
    use tokio_util::sync::CancellationToken;

    let Some(config) = rest_config("the_recorder_writes_one_event_and_patches_its_count").await else {
        return;
    };
    let user_agent = config.user_agent().to_owned();
    let kube = config.into_kube_client().unwrap();
    let client = &KubeClient::uncached(kube.clone());
    let user_agent = &user_agent;
    in_namespace(client, "events", |ns| async move {
        let mut cm = config_map(&ns, "involved");
        (client as &dyn Client).create(&mut cm).await.expect("create");

        let (tx, rx) = tokio::sync::mpsc::channel(MAX_QUEUED_EVENTS);
        let cancel = CancellationToken::new();
        let broadcaster = tokio::spawn(run_broadcaster(
            std::sync::Arc::new(KubeEventSink::new(kube.clone())),
            rx,
            std::sync::Arc::new(crate::clock::SystemClock),
            crate::event::DEFAULT_SLEEP_DURATION,
            cancel.clone(),
        ));
        let recorder = ApiRecorder::new("managed/mytype.sample.example.com", tx);
        let recorder = crate::event::Recorder::with_annotations(&recorder, &[("external-name", "ext-1")]);
        for _ in 0..12 {
            recorder.event(
                &cm,
                crate::event::Event::normal(
                    crate::event::Reason("PendingExternalResource".into()),
                    "Waiting for external resource existence to be confirmed",
                ),
            );
        }
        let api: kube::Api<CoreEvent> = kube::Api::namespaced(kube.clone(), &ns);
        let mut events = Vec::new();
        for _ in 0..50 {
            events = api.list(&Default::default()).await.unwrap().items;
            if events.first().and_then(|e| e.count) == Some(12) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        cancel.cancel();
        broadcaster.await.unwrap();

        let shown: Vec<_> = events
            .iter()
            .map(|e| (e.metadata.name.clone().unwrap_or_default(), e.count, e.reason.clone()))
            .collect();
        eprintln!("events in {ns}: {shown:?}");
        assert_eq!(events.len(), 1, "one Event object: {shown:?}");
        let e = &events[0];
        assert_eq!(e.count, Some(12));
        let name = e.metadata.name.as_deref().unwrap();
        let hex = name.strip_prefix("involved.").expect("<object>.<hex>");
        assert!(hex.len() == 16 && u64::from_str_radix(hex, 16).is_ok(), "{name}");
        assert_eq!(e.type_.as_deref(), Some("Normal"));
        assert_eq!(
            e.source.as_ref().and_then(|s| s.component.as_deref()),
            Some("managed/mytype.sample.example.com")
        );
        assert_eq!(
            e.reporting_component.as_deref(),
            Some("managed/mytype.sample.example.com")
        );
        assert_eq!(
            e.metadata.annotations,
            Some([("external-name".to_owned(), "ext-1".to_owned())].into())
        );
        let o = &e.involved_object;
        assert_eq!(
            (
                o.kind.as_deref(),
                o.api_version.as_deref(),
                o.name.as_deref(),
                o.namespace.as_deref()
            ),
            (Some("ConfigMap"), Some("v1"), Some("involved"), Some(ns.as_str()))
        );
        assert_eq!(o.uid, cm.metadata.uid);
        assert_eq!(o.resource_version, cm.metadata.resource_version);

        if let Some(audit) = std::env::var_os("XPRS_TEST_AUDIT_LOG") {
            let mut verbs = Vec::new();
            for _ in 0..20 {
                let log = std::fs::read_to_string(&audit).expect("readable audit log");
                verbs = log
                    .lines()
                    .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
                    .filter(|a| a["objectRef"]["resource"] == "events" && a["objectRef"]["namespace"] == ns.as_str())
                    .map(|a| {
                        assert_eq!(a["userAgent"], user_agent.as_str());
                        (
                            a["verb"].as_str().unwrap().to_owned(),
                            a["responseStatus"]["code"].as_u64().unwrap(),
                        )
                    })
                    .collect();
                if verbs.len() >= 12 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            eprintln!("audit: event writes in {ns}: {verbs:?}");
            let mut want = vec![("create".to_owned(), 201)];
            want.extend(std::iter::repeat_n(("patch".to_owned(), 200), 11));
            assert_eq!(verbs, want, "one create, then a patch per repeat");
        }
    })
    .await;
}
