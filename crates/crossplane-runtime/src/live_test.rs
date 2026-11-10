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

use crate::kube_client::KubeClient;
use crate::manager::RestConfig;

pub(crate) async fn rest_config(test: &str) -> Option<RestConfig> {
    let Some(path) = std::env::var_os("XPRS_TEST_KUBECONFIG") else {
        eprintln!("skipping {test}: set XPRS_TEST_KUBECONFIG to a kubeconfig");
        return None;
    };
    Some(
        RestConfig::from_kubeconfig(path, "xprs-test", "v0.0.0")
            .await
            .expect("usable kubeconfig"),
    )
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

/// Polls `read` until it returns `Some`, for at most five seconds.
pub(crate) async fn eventually<T, Fut: Future<Output = Option<T>>>(what: &str, mut read: impl FnMut() -> Fut) -> T {
    for _ in 0..100 {
        if let Some(v) = read().await {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn the_cache_serves_reads_and_follows_writes() {
    use crossplane_core::client::ListOptions;

    use crate::cache::Cache;

    let Some(config) = rest_config("the_cache_serves_reads_and_follows_writes").await else {
        return;
    };
    let kube = config.into_kube_client().unwrap();
    let direct = &KubeClient::uncached(kube.clone());
    let cache = Cache::new(kube.clone(), Duration::ZERO);
    let cached = &KubeClient::new(kube, cache.clone());
    in_namespace(direct, "cache", |ns| async move {
        let (direct, cached): (&dyn Client, &dyn Client) = (direct, cached);
        let mut a = config_map(&ns, "a");
        direct.create(&mut a).await.unwrap();

        let got: ConfigMap = eventually("a in the cache", || async {
            cached.get_as::<ConfigMap>(&ObjectKey::namespaced(&ns, "a")).await.ok()
        })
        .await;
        assert_eq!(got.metadata.uid, a.metadata.uid);

        let missing = cached
            .get_as::<ConfigMap>(&ObjectKey::namespaced(&ns, "missing"))
            .await
            .unwrap_err();
        assert!(missing.is_not_found());
        assert_eq!(
            missing.to_string(),
            r#"ConfigMap "missing" not found"#,
            "Go's cache reader text"
        );

        a.data = Some([("k".to_owned(), "v".to_owned())].into());
        direct.update(&mut a).await.unwrap();
        let rv = a.metadata.resource_version.clone();
        eventually("the update in the cache", || async {
            let got: ConfigMap = cached.get_as(&ObjectKey::namespaced(&ns, "a")).await.ok()?;
            (got.metadata.resource_version == rv).then_some(())
        })
        .await;

        let mut b = config_map(&ns, "b");
        direct.create(&mut b).await.unwrap();
        let in_ns = ListOptions {
            namespace: Some(ns.clone()),
            ..Default::default()
        };
        let names = eventually("both in a cached list", || async {
            let listed: Vec<ConfigMap> = cached.list_as(&in_ns).await.ok()?;
            let names: Vec<_> = listed.into_iter().filter_map(|c| c.metadata.name).collect();
            (names.len() >= 3).then_some(names)
        })
        .await;
        assert_eq!(names, ["a", "b", "kube-root-ca.crt"], "the API server's order");

        direct.delete(&b).await.unwrap();
        eventually("the delete in the cache", || async {
            cached
                .get_as::<ConfigMap>(&ObjectKey::namespaced(&ns, "b"))
                .await
                .err()
                .filter(|e| e.is_not_found())
        })
        .await;
    })
    .await;
    cache.stop();
}

/// The baseline provider's kind, with an open spec and status so a test can write anything. Its
/// CRD has a status subresource, which a status-only write needs.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct MyType {
    metadata: ObjectMeta,
    spec: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<serde_json::Value>,
}

impl kube::Resource for MyType {
    type DynamicType = ();
    type Scope = kube::core::NamespaceResourceScope;

    fn kind(_: &()) -> std::borrow::Cow<'_, str> {
        "MyType".into()
    }
    fn group(_: &()) -> std::borrow::Cow<'_, str> {
        "sample.example.com".into()
    }
    fn version(_: &()) -> std::borrow::Cow<'_, str> {
        "v1alpha1".into()
    }
    fn plural(_: &()) -> std::borrow::Cow<'_, str> {
        "mytypes".into()
    }
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

/// Paused, so the installed Go provider only marks it paused.
fn my_type(ns: &str, name: &str) -> MyType {
    MyType {
        metadata: ObjectMeta {
            name: Some(name.into()),
            namespace: Some(ns.into()),
            annotations: Some([("crossplane.io/paused".to_owned(), "true".to_owned())].into()),
            ..Default::default()
        },
        spec: serde_json::json!({"forProvider": {"configurableField": "a"}}),
        status: None,
    }
}

/// The baseline's `example.com` ProviderConfigUsage, whose CRD is installed with the baseline, so
/// the builder test can watch usages with the real `EnqueueRequestForProviderConfig`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Usage {
    metadata: ObjectMeta,
    #[serde(flatten)]
    usage: crossplane_core::v2::TypedProviderConfigUsage,
}

impl kube::Resource for Usage {
    type DynamicType = ();
    type Scope = kube::core::NamespaceResourceScope;

    fn kind(_: &()) -> std::borrow::Cow<'_, str> {
        "ProviderConfigUsage".into()
    }
    fn group(_: &()) -> std::borrow::Cow<'_, str> {
        "example.com".into()
    }
    fn version(_: &()) -> std::borrow::Cow<'_, str> {
        "v1alpha1".into()
    }
    fn plural(_: &()) -> std::borrow::Cow<'_, str> {
        "providerconfigusages".into()
    }
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

impl crossplane_core::resource::ProviderConfigUsage for Usage {
    fn from_usage(metadata: ObjectMeta, usage: crossplane_core::v2::TypedProviderConfigUsage) -> Self {
        Self { metadata, usage }
    }
    fn usage(&self) -> &crossplane_core::v2::TypedProviderConfigUsage {
        &self.usage
    }
    fn usage_mut(&mut self) -> &mut crossplane_core::v2::TypedProviderConfigUsage {
        &mut self.usage
    }
}

/// Counts reconciles per object name, in this test's namespace only: the controllers watch every
/// namespace, and other tests and the parity runs hold MyTypes with the same names.
#[derive(Clone, Default)]
struct Counting(std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, usize>>>);

impl Counting {
    fn of(&self, name: &str) -> usize {
        self.0.lock().unwrap().get(name).copied().unwrap_or(0)
    }
}

impl crate::reconciler::Reconcile for Counting {
    fn reconcile<'a>(
        &'a self,
        req: &'a crate::reconciler::Request,
    ) -> futures::future::BoxFuture<'a, crossplane_core::errors::Result<crate::reconciler::Outcome>> {
        let ours = req.namespace.as_deref() == Some(&format!("xprs-builder-{}", std::process::id()));
        if ours {
            *self.0.lock().unwrap().entry(req.name.clone()).or_default() += 1;
        }
        async { Ok(crate::reconciler::Outcome::Done) }.boxed()
    }
}

/// Get, change, update, again on a conflict: reads come from the cache, so a write can race the
/// installed provider's status writes and get 409, as Go's cached reads do.
async fn update_retrying(client: &dyn Client, key: &ObjectKey, change: impl Fn(&mut MyType)) {
    for _ in 0..50 {
        let mut obj: MyType = client.get_as(key).await.unwrap();
        change(&mut obj);
        match client.update(&mut obj).await {
            Ok(()) => return,
            Err(e) if e.is_conflict() => tokio::time::sleep(Duration::from_millis(50)).await,
            Err(e) => panic!("update: {e}"),
        }
    }
    panic!("update kept conflicting");
}

/// Waits until `count()` stops changing for `quiet`, so a later "did not change" is not read
/// before the events have arrived.
async fn settle(count: impl Fn() -> usize, quiet: Duration) -> usize {
    let mut last = count();
    loop {
        tokio::time::sleep(quiet).await;
        let now = count();
        if now == last {
            return now;
        }
        last = now;
    }
}

#[tokio::test]
async fn desired_state_changed_filters_status_writes_and_resyncs() {
    use crate::controller::{self, ControllerOptions};
    use crate::manager::{Manager, ManagerOptions};

    let Some(config) = rest_config("desired_state_changed_filters_status_writes_and_resyncs").await else {
        return;
    };
    let sync = Duration::from_secs(3);
    let mgr = Manager::new(
        config,
        ManagerOptions {
            sync_period: sync,
            metrics_bind_address: "0".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let client = mgr.client();
    let (filtered, control) = (Counting::default(), Counting::default());
    controller::new_managed_by(&mgr)
        .named("filtered")
        .with_options(ControllerOptions::default())
        .with_event_filter(crate::resource::desired_state_changed())
        .for_kind::<MyType>()
        .watches::<Usage>(crate::resource::EnqueueRequestForProviderConfig::new("ProviderConfig"))
        .complete(filtered.clone())
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let running = tokio::spawn({
        let (mgr, stop) = (mgr.clone(), stop.clone());
        async move { mgr.start(stop.cancelled_owned()).await }
    });
    controller::new_managed_by(&mgr)
        .named("control")
        .for_kind::<MyType>()
        .complete(control.clone())
        .expect("a controller added after start runs at once, as the safe-start gate needs");

    let (filtered, control) = (&filtered, &control);
    let client = &*client;
    in_namespace(client, "builder", |ns| async move {
        let quiet = Duration::from_millis(700);
        let key = ObjectKey::namespaced(&ns, "a");
        let mut obj = my_type(&ns, "a");
        client.create(&mut obj).await.unwrap();
        eventually("the create to reach both controllers", || async {
            (filtered.of("a") >= 1 && control.of("a") >= 1).then_some(())
        })
        .await;
        let base = settle(|| filtered.of("a"), quiet).await;
        assert_eq!(base, 1, "the create triggered the filtered controller once");

        let control_before = control.of("a");
        let rv = loop {
            let mut obj: MyType = client.get_as(&key).await.unwrap();
            let mut status = obj.status.clone().unwrap_or_else(|| serde_json::json!({}));
            status["atProvider"] = serde_json::json!({"configurableField": "observed"});
            obj.status = Some(status);
            match client.update_status(&mut obj).await {
                Ok(()) => break obj.metadata.resource_version,
                Err(e) if e.is_conflict() => continue,
                Err(e) => panic!("status write: {e}"),
            }
        };
        eventually("the status write in the cache", || async {
            let got: MyType = client.get_as(&key).await.ok()?;
            (got.metadata.resource_version >= rv).then_some(())
        })
        .await;
        eventually("the control controller to see the status write", || async {
            (control.of("a") > control_before).then_some(())
        })
        .await;
        assert_eq!(
            settle(|| filtered.of("a"), quiet).await,
            1,
            "a status-only write does not retrigger"
        );

        let control_before = control.of("a");
        tokio::time::sleep(sync.mul_f64(1.2)).await;
        let control_after = control.of("a");
        let filtered_after = settle(|| filtered.of("a"), quiet).await;
        eprintln!(
            "resync window {:?}: control {control_before} -> {control_after}, filtered stays {filtered_after}",
            sync.mul_f64(1.2)
        );
        assert!(
            control_after > control_before,
            "the resync reached the unfiltered controller"
        );
        assert_eq!(filtered_after, 1, "a resync does not trigger the filtered controller");

        update_retrying(client, &key, |o| {
            o.spec["forProvider"]["configurableField"] = serde_json::json!("b")
        })
        .await;
        eventually("a spec change to trigger", || async {
            (filtered.of("a") == 2).then_some(())
        })
        .await;

        update_retrying(client, &key, |o| {
            o.metadata
                .annotations
                .get_or_insert_default()
                .insert("xprs/touch".into(), "1".into());
        })
        .await;
        eventually("an annotation change to trigger", || async {
            (filtered.of("a") == 3).then_some(())
        })
        .await;

        update_retrying(client, &key, |o| {
            o.metadata.annotations.get_or_insert_default().insert(
                crossplane_core::meta::ANNOTATION_KEY_EXTERNAL_CREATE_PENDING.into(),
                "x".into(),
            );
        })
        .await;
        tokio::time::sleep(quiet).await;
        assert_eq!(
            settle(|| filtered.of("a"), quiet).await,
            3,
            "the create-pending annotation is ignored"
        );

        let mut usage = Usage {
            metadata: ObjectMeta {
                name: Some("trigger".into()),
                namespace: Some(ns.clone()),
                ..Default::default()
            },
            usage: crossplane_core::v2::TypedProviderConfigUsage {
                provider_config_ref: crossplane_core::v2::ProviderConfigReference {
                    kind: "ProviderConfig".into(),
                    name: "a".into(),
                },
                resource_ref: crossplane_core::v2::TypedReference {
                    api_version: "sample.example.com/v1alpha1".into(),
                    kind: "MyType".into(),
                    name: "a".into(),
                    ..Default::default()
                },
            },
        };
        client.create(&mut usage).await.unwrap();
        eventually("a watched usage mapped to the config it references", || async {
            (filtered.of("a") == 4).then_some(())
        })
        .await;
        assert_eq!(
            filtered.of("trigger"),
            0,
            "the handler's mapping, not the usage's own key"
        );
        eprintln!(
            "filtered {:?}, control {:?}",
            filtered.0.lock().unwrap(),
            control.0.lock().unwrap()
        );
    })
    .await;
    stop.cancel();
    running.await.unwrap().expect("the manager stops cleanly");
}

/// Two electors on one Lease, observed through the API: the first acquires, the second waits; the
/// first stops without releasing (as controller-runtime leaves `ReleaseOnCancel` off) and the
/// second takes over once the lease expires; an outside write of the holder makes the leader
/// report the lease lost within the renew deadline.
#[tokio::test]
async fn a_second_elector_takes_over_an_abandoned_lease() {
    use k8s_openapi::api::coordination::v1::Lease;
    use tokio_util::sync::CancellationToken;

    use crate::manager::leader_election::Elector;

    let Some(config) = rest_config("a_second_elector_takes_over_an_abandoned_lease").await else {
        return;
    };
    let kube = config.into_kube_client().unwrap();
    let direct = &KubeClient::uncached(kube.clone());
    let (lease_duration, renew_deadline, retry) =
        (Duration::from_secs(4), Duration::from_secs(3), Duration::from_secs(1));
    in_namespace(direct, "lease", |ns| async move {
        let elector = |id: &str| {
            Elector::new(kube.clone(), &ns, "xprs-test-lease", id.to_owned(), lease_duration, renew_deadline, retry, None).unwrap()
        };
        let api: kube::Api<Lease> = kube::Api::namespaced(kube.clone(), &ns);
        let spec = || async { api.get("xprs-test-lease").await.unwrap().spec.unwrap() };

        let mut first = elector("first");
        let mut second = elector("second");
        let stop_first = CancellationToken::new();
        assert!(first.acquire(&stop_first).await, "the first elector creates the lease");
        let held = spec().await;
        assert_eq!(
            (held.holder_identity.as_deref(), held.lease_duration_seconds, held.lease_transitions),
            (Some("first"), Some(4), Some(0))
        );

        let renewing = tokio::spawn({
            let stop = stop_first.clone();
            async move { first.renew(&stop).await }
        });
        let stop_second = CancellationToken::new();
        let waiting = tokio::spawn({
            let stop = stop_second.clone();
            async move {
                let won = second.acquire(&stop).await;
                (won, std::time::Instant::now(), second)
            }
        });
        tokio::time::sleep(lease_duration + retry).await;
        assert!(!waiting.is_finished(), "the second elector waits while the first renews");
        let renewed = spec().await;
        assert!(renewed.renew_time > held.renew_time, "the first elector renewed");

        stop_first.cancel();
        let stopped = std::time::Instant::now();
        assert!(!renewing.await.unwrap(), "a cancelled leader does not report a loss");
        let (won, at, mut second) = waiting.await.unwrap();
        let takeover = at - stopped;
        let after = spec().await;
        eprintln!(
            "lease: takeover {takeover:?} after the leader stopped (lease {lease_duration:?}, retry {retry:?}); holder {:?}, transitions {:?}",
            after.holder_identity, after.lease_transitions
        );
        assert!(won);
        assert_eq!((after.holder_identity.as_deref(), after.lease_transitions), (Some("second"), Some(1)));
        assert!(takeover <= lease_duration + retry.mul_f64(2.2) + Duration::from_secs(1), "{takeover:?}");

        let renewing = tokio::spawn({
            let stop = stop_second.clone();
            async move { second.renew(&stop).await }
        });
        loop {
            let mut stolen = api.get("xprs-test-lease").await.unwrap();
            stolen.spec.as_mut().unwrap().holder_identity = Some("intruder".into());
            match api.replace("xprs-test-lease", &Default::default(), &stolen).await {
                Ok(_) => break,
                Err(kube::Error::Api(s)) if s.code == 409 => continue,
                Err(e) => panic!("steal: {e}"),
            }
        }
        let stolen_at = std::time::Instant::now();
        let lost = tokio::time::timeout(renew_deadline * 3, renewing).await.expect("loss reported").unwrap();
        eprintln!("lease: loss reported {:?} after the holder was overwritten", stolen_at.elapsed());
        assert!(lost, "the leader reports the lease lost");
        assert_eq!(spec().await.holder_identity.as_deref(), Some("intruder"), "and does not take it back");
    })
    .await;
}

/// The baseline `MyType` with Crossplane's managed spec and status, for code that needs `Managed`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct ManagedMyType {
    metadata: ObjectMeta,
    spec: ManagedMyTypeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<crossplane_core::v2::ManagedResourceStatus>,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct ManagedMyTypeSpec {
    #[serde(flatten)]
    managed: crossplane_core::v2::ManagedResourceSpec,
    #[serde(rename = "forProvider", default)]
    for_provider: serde_json::Value,
}

impl kube::Resource for ManagedMyType {
    type DynamicType = ();
    type Scope = kube::core::NamespaceResourceScope;

    fn kind(_: &()) -> std::borrow::Cow<'_, str> {
        "MyType".into()
    }
    fn group(_: &()) -> std::borrow::Cow<'_, str> {
        "sample.example.com".into()
    }
    fn version(_: &()) -> std::borrow::Cow<'_, str> {
        "v1alpha1".into()
    }
    fn plural(_: &()) -> std::borrow::Cow<'_, str> {
        "mytypes".into()
    }
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

impl crossplane_core::resource::Managed for ManagedMyType {
    fn managed_spec(&self) -> &crossplane_core::v2::ManagedResourceSpec {
        &self.spec.managed
    }
    fn managed_spec_mut(&mut self) -> &mut crossplane_core::v2::ManagedResourceSpec {
        &mut self.spec.managed
    }
    fn managed_status(&self) -> Option<&crossplane_core::v2::ManagedResourceStatus> {
        self.status.as_ref()
    }
    fn managed_status_mut(&mut self) -> &mut crossplane_core::v2::ManagedResourceStatus {
        self.status.get_or_insert_default()
    }
}

/// A plain HTTP/1.0 GET, enough for the metrics endpoint.
async fn http_get(address: std::net::SocketAddr, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let status = response.split(' ').nth(1).unwrap().parse().unwrap();
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn the_metrics_endpoint_serves_managed_resource_state() {
    use crate::manager::{Manager, ManagerOptions};
    use crate::statemetrics::{MrStateMetrics, MrStateRecorder};

    let Some(config) = rest_config("the_metrics_endpoint_serves_managed_resource_state").await else {
        return;
    };
    let mgr = Manager::new(
        config,
        ManagerOptions {
            metrics_bind_address: "127.0.0.1:0".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let metrics = MrStateMetrics::new();
    mgr.register_metrics(Box::new(metrics.clone())).unwrap();
    mgr.add(MrStateRecorder::<ManagedMyType>::new(
        mgr.client(),
        metrics,
        Duration::from_millis(500),
    ))
    .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let running = tokio::spawn({
        let (mgr, stop) = (mgr.clone(), stop.clone());
        async move { mgr.start(stop.cancelled_owned()).await }
    });
    let client = mgr.client();
    let client = &*client;
    let mgr = &mgr;
    in_namespace(client, "metrics", |ns| async move {
        for name in ["a", "b"] {
            client.create(&mut my_type(&ns, name)).await.unwrap();
        }
        let address = eventually("the metrics listener", || async { mgr.metrics_address() }).await;
        let label = r#"{gvk="sample.example.com/v1alpha1, Kind=MyType"}"#;
        let value = |body: &str, name: &str| -> Option<usize> {
            let prefix = format!("crossplane_managed_resource_{name}{label} ");
            body.lines().find_map(|l| l.strip_prefix(&prefix)?.parse().ok())
        };
        let body = eventually("the gauges to count every MyType", || async {
            let all: Vec<ManagedMyType> = client.list_as(&Default::default()).await.ok()?;
            let (status, body) = http_get(address, "/metrics").await;
            assert_eq!(status, 200);
            (value(&body, "exists") == Some(all.len()) && all.len() >= 2).then_some(body)
        })
        .await;
        let names: std::collections::BTreeSet<_> =
            body.lines().filter(|l| l.starts_with("# HELP crossplane_")).collect();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            [
                "# HELP crossplane_managed_resource_exists The number of managed resources that exist",
                "# HELP crossplane_managed_resource_ready The number of managed resources in Ready=True state",
                "# HELP crossplane_managed_resource_synced The number of managed resources in Synced=True state",
            ],
            "Go's names and help text"
        );
        eprintln!(
            "metrics: exists {:?} ready {:?} synced {:?}",
            value(&body, "exists"),
            value(&body, "ready"),
            value(&body, "synced")
        );
        assert_eq!(http_get(address, "/").await.0, 404);
    })
    .await;
    stop.cancel();
    running.await.unwrap().expect("the manager stops cleanly");
}

/// Reports its name when started, then runs until shutdown.
struct Started {
    name: &'static str,
    tx: tokio::sync::mpsc::UnboundedSender<(&'static str, Instant)>,
    gated: bool,
}

impl crate::manager::Runnable for Started {
    fn start(
        self: Box<Self>,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> futures::future::BoxFuture<'static, crossplane_core::errors::Result<()>> {
        let _ = self.tx.send((self.name, Instant::now()));
        Box::pin(async move {
            shutdown.cancelled().await;
            Ok(())
        })
    }

    fn needs_leader_election(&self) -> bool {
        self.gated
    }
}

/// The manager's leader election end to end: a gated runnable waits while another identity holds
/// a valid Lease, starts once the manager takes it over, the manager records Go's "became leader"
/// event on the Lease, and an outside write of the holder makes `start` return "leader election
/// lost".
#[tokio::test]
async fn the_manager_gates_runnables_on_the_lease_and_fails_when_it_is_lost() {
    use k8s_openapi::api::coordination::v1::{Lease, LeaseSpec};
    use k8s_openapi::api::core::v1::Event as CoreEvent;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime;

    use crate::manager::{LeaderElection, Manager, ManagerOptions};

    let Some(config) = rest_config("the_manager_gates_runnables_on_the_lease_and_fails_when_it_is_lost").await else {
        return;
    };
    let kube = config.clone().into_kube_client().unwrap();
    let direct = &KubeClient::uncached(kube.clone());
    in_namespace(direct, "leader", |ns| async move {
        let leases: kube::Api<Lease> = kube::Api::namespaced(kube.clone(), &ns);
        let id = "xprs-test-manager";
        let other = Lease {
            metadata: ObjectMeta {
                name: Some(id.into()),
                ..Default::default()
            },
            spec: Some(LeaseSpec {
                holder_identity: Some("other".into()),
                lease_duration_seconds: Some(4),
                acquire_time: Some(MicroTime(k8s_openapi::jiff::Timestamp::now())),
                renew_time: Some(MicroTime(k8s_openapi::jiff::Timestamp::now())),
                lease_transitions: Some(0),
                ..Default::default()
            }),
        };
        leases.create(&Default::default(), &other).await.unwrap();

        let renew_deadline = Duration::from_secs(3);
        let mgr = Manager::new(
            config,
            ManagerOptions {
                metrics_bind_address: "0".into(),
                leader_election: Some(LeaderElection {
                    namespace: Some(ns.clone()),
                    retry_period: Duration::from_secs(1),
                    ..LeaderElection::new(id)
                        .lease_duration(Duration::from_secs(4))
                        .renew_deadline(renew_deadline)
                }),
                ..Default::default()
            },
        )
        .unwrap();
        let (tx, mut started) = tokio::sync::mpsc::unbounded_channel();
        mgr.add(Started {
            name: "gated",
            tx: tx.clone(),
            gated: true,
        })
        .unwrap();
        mgr.add(Started {
            name: "ungated",
            tx,
            gated: false,
        })
        .unwrap();
        let begun = Instant::now();
        let running = tokio::spawn({
            let mgr = mgr.clone();
            async move { mgr.start(std::future::pending()).await }
        });

        let (first, _) = started.recv().await.unwrap();
        assert_eq!(first, "ungated", "a runnable that needs no lease starts at once");
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(
            started.try_recv().is_err(),
            "the gated runnable waits while `other` holds the lease"
        );
        let (gated, at) = tokio::time::timeout(Duration::from_secs(10), started.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(gated, "gated");
        let held = leases.get(id).await.unwrap().spec.unwrap();
        let holder = held.holder_identity.clone().unwrap();
        let host = hostname::get().unwrap().to_string_lossy().into_owned();
        let uuid = holder.strip_prefix(&format!("{host}_")).expect("hostname_uuid");
        assert_eq!(uuid.len(), 36, "{holder}");
        assert_eq!(held.lease_transitions, Some(1));
        eprintln!(
            "leader: gated runnable started {:?} after start; holder {holder}",
            at - begun
        );

        let events: kube::Api<CoreEvent> = kube::Api::namespaced(kube.clone(), &ns);
        let became = eventually("the became-leader event", || async {
            let list = events.list(&Default::default()).await.ok()?;
            list.items
                .into_iter()
                .find(|e| e.reason.as_deref() == Some("LeaderElection"))
        })
        .await;
        assert_eq!(
            became.message.as_deref(),
            Some(format!("{holder} became leader").as_str())
        );
        assert_eq!(became.involved_object.kind.as_deref(), Some("Lease"));
        assert_eq!(became.involved_object.name.as_deref(), Some(id));
        assert_eq!(
            became.source.as_ref().and_then(|s| s.component.as_deref()),
            Some(holder.as_str())
        );
        assert_eq!(became.type_.as_deref(), Some("Normal"));

        loop {
            let mut stolen = leases.get(id).await.unwrap();
            stolen.spec.as_mut().unwrap().holder_identity = Some("intruder".into());
            match leases.replace(id, &Default::default(), &stolen).await {
                Ok(_) => break,
                Err(kube::Error::Api(s)) if s.code == 409 => continue,
                Err(e) => panic!("steal: {e}"),
            }
        }
        let stolen_at = Instant::now();
        let result = tokio::time::timeout(renew_deadline * 3, running)
            .await
            .expect("start returned")
            .unwrap();
        eprintln!(
            "leader: start returned {:?} after the holder was overwritten",
            stolen_at.elapsed()
        );
        assert_eq!(result.unwrap_err().to_string(), "leader election lost");
    })
    .await;
}
