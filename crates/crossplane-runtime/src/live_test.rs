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
