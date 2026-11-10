//! Every unit together against a real API server: a Manager running a managed controller wired
//! as a provider wires it (`examples/provider-sample/src/controller/mytype/wiring.rs`), over a
//! kind of its own whose CRD the test installs and removes, with an in-memory external system.
//! Everything is asserted through the API with an uncached client. Skipped unless
//! `XPRS_TEST_KUBECONFIG` names a cluster.
//!
//! Not exercised here: the ProviderConfig reconciler and usage tracker, the safe-start gate,
//! change logs, connection secrets, metrics and leader election.

use std::borrow::Cow;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::client::{Client, ListOptions, Patch};
use crossplane_core::errors::Result;
use crossplane_core::meta;
use crossplane_core::resource::{Managed, ObjectKey};
use crossplane_core::v2::{Condition, ManagedResourceSpec, ManagedResourceStatus};
use futures::FutureExt;
use k8s_openapi::api::core::v1::Event as CoreEvent;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::controller::{self, Options};
use crate::feature::Features;
use crate::gate::Gate;
use crate::kube_client::KubeClient;
use crate::live_test::{namespace, rest_config};
use crate::manager::{Manager, ManagerOptions};
use crate::reconciler::managed::{
    self, ExternalClient, ExternalConnector, ExternalCreation, ExternalDelete, ExternalObservation, ExternalUpdate,
};
use crate::{ratelimiter, resource};

const POLL: Duration = Duration::from_secs(1);
const CREATION_GRACE: Duration = Duration::from_secs(3);

/// Per process, so concurrent runs and a run that died before cleanup never share a CRD.
fn group() -> String {
    format!("xprs-e2e-{}.example.com", std::process::id())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GadgetSpec {
    #[serde(flatten)]
    managed: ManagedResourceSpec,
    for_provider: GadgetParameters,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GadgetParameters {
    field: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GadgetStatus {
    #[serde(flatten)]
    managed: ManagedResourceStatus,
    #[serde(default)]
    at_provider: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Gadget {
    metadata: ObjectMeta,
    spec: GadgetSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<GadgetStatus>,
}

impl kube::Resource for Gadget {
    type DynamicType = ();
    type Scope = kube::core::NamespaceResourceScope;

    fn kind(_: &()) -> Cow<'_, str> {
        "Gadget".into()
    }
    fn group(_: &()) -> Cow<'_, str> {
        group().into()
    }
    fn version(_: &()) -> Cow<'_, str> {
        "v1alpha1".into()
    }
    fn plural(_: &()) -> Cow<'_, str> {
        "gadgets".into()
    }
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

impl Managed for Gadget {
    fn managed_spec(&self) -> &ManagedResourceSpec {
        &self.spec.managed
    }
    fn managed_spec_mut(&mut self) -> &mut ManagedResourceSpec {
        &mut self.spec.managed
    }
    fn managed_status(&self) -> Option<&ManagedResourceStatus> {
        self.status.as_ref().map(|s| &s.managed)
    }
    fn managed_status_mut(&mut self) -> &mut ManagedResourceStatus {
        &mut self.status.get_or_insert_default().managed
    }
}

fn crd() -> CustomResourceDefinition {
    serde_json::from_value(json!({
        "metadata": {"name": format!("gadgets.{}", group())},
        "spec": {
            "group": group(),
            "names": {"kind": "Gadget", "listKind": "GadgetList", "plural": "gadgets", "singular": "gadget"},
            "scope": "Namespaced",
            "versions": [{
                "name": "v1alpha1",
                "served": true,
                "storage": true,
                "subresources": {"status": {}},
                "schema": {"openAPIV3Schema": {
                    "type": "object",
                    "properties": {
                        "spec": {
                            "type": "object",
                            "x-kubernetes-preserve-unknown-fields": true,
                            "properties": {"managementPolicies": {
                                "type": "array",
                                "items": {"type": "string"},
                                "default": ["*"]
                            }}
                        },
                        "status": {"type": "object", "x-kubernetes-preserve-unknown-fields": true}
                    }
                }}
            }]
        }
    }))
    .unwrap()
}

/// The external system: things by external name, every write in order, and a count of observes.
#[derive(Default)]
struct World {
    things: HashMap<String, String>,
    writes: Vec<String>,
    observes: usize,
}

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<World>>);

impl Shared {
    fn writes(&self) -> Vec<String> {
        self.0.lock().unwrap().writes.clone()
    }
    fn observes(&self) -> usize {
        self.0.lock().unwrap().observes
    }
}

impl ExternalConnector<Gadget> for Shared {
    type Client = Shared;

    async fn connect(&self, _: &Gadget) -> Result<Shared> {
        Ok(self.clone())
    }
}

/// Mirrors the sample provider's `External`: Available on observe, Creating on create, Deleting
/// on delete, up to date when the external value equals the spec.
impl ExternalClient<Gadget> for Shared {
    async fn observe(&self, mg: &mut Gadget) -> Result<ExternalObservation> {
        let mut world = self.0.lock().unwrap();
        world.observes += 1;
        let Some(value) = world.things.get(meta::external_name(mg)).cloned() else {
            return Ok(ExternalObservation::default());
        };
        let up_to_date = value == mg.spec.for_provider.field;
        mg.status.get_or_insert_default().at_provider = json!({"field": value});
        mg.set_conditions([Condition::available()]);
        Ok(ExternalObservation {
            resource_exists: true,
            resource_up_to_date: up_to_date,
            ..Default::default()
        })
    }

    async fn create(&self, mg: &mut Gadget) -> Result<ExternalCreation> {
        mg.set_conditions([Condition::creating()]);
        let (name, value) = (meta::external_name(mg).to_owned(), mg.spec.for_provider.field.clone());
        let mut world = self.0.lock().unwrap();
        world.writes.push(format!("create {name}={value}"));
        world.things.insert(name, value);
        Ok(ExternalCreation::default())
    }

    async fn update(&self, mg: &mut Gadget) -> Result<ExternalUpdate> {
        let (name, value) = (meta::external_name(mg).to_owned(), mg.spec.for_provider.field.clone());
        let mut world = self.0.lock().unwrap();
        world.writes.push(format!("update {name}={value}"));
        world.things.insert(name, value);
        Ok(ExternalUpdate::default())
    }

    async fn delete(&self, mg: &mut Gadget) -> Result<ExternalDelete> {
        mg.set_conditions([Condition::deleting()]);
        let name = meta::external_name(mg).to_owned();
        let mut world = self.0.lock().unwrap();
        world.writes.push(format!("delete {name}"));
        world.things.remove(&name);
        Ok(ExternalDelete::default())
    }
}

async fn within<T, Fut: Future<Output = Option<T>>>(what: &str, wait: Duration, mut read: impl FnMut() -> Fut) -> T {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        if let Some(v) = read().await {
            return v;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out after {wait:?} waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// `(type, status, reason)` of a condition in the object as the API server holds it.
fn condition(obj: &Gadget, type_: &str) -> Option<(String, String, String)> {
    let status = serde_json::to_value(obj.status.as_ref()?).ok()?;
    status["conditions"].as_array()?.iter().find_map(|c| {
        (c["type"] == type_).then(|| {
            (
                type_.to_owned(),
                c["status"].as_str().unwrap_or_default().to_owned(),
                c["reason"].as_str().unwrap_or_default().to_owned(),
            )
        })
    })
}

fn want(type_: &str, status: &str, reason: &str) -> Option<(String, String, String)> {
    Some((type_.to_owned(), status.to_owned(), reason.to_owned()))
}

/// `(type, reason, message, source component)` of every core/v1 Event about `name`.
async fn events(direct: &dyn Client, ns: &str, name: &str) -> Vec<(String, String, String, String)> {
    let options = ListOptions {
        namespace: Some(ns.to_owned()),
        ..Default::default()
    };
    let all: Vec<CoreEvent> = direct.list_as(&options).await.unwrap();
    all.into_iter()
        .filter(|e| {
            e.involved_object.name.as_deref() == Some(name) && e.involved_object.kind.as_deref() == Some("Gadget")
        })
        .map(|e| {
            (
                e.type_.unwrap_or_default(),
                e.reason.unwrap_or_default(),
                e.message.unwrap_or_default(),
                e.source.and_then(|s| s.component).unwrap_or_default(),
            )
        })
        .collect()
}

async fn has_event(
    direct: &dyn Client,
    ns: &str,
    name: &str,
    type_: &str,
    reason: &str,
    message: &str,
    component: &str,
) {
    let want = (
        type_.to_owned(),
        reason.to_owned(),
        message.to_owned(),
        component.to_owned(),
    );
    within(&format!("event {reason}"), Duration::from_secs(15), || async {
        events(direct, ns, name).await.contains(&want).then_some(())
    })
    .await;
}

async fn merge(direct: &dyn Client, obj: &mut Gadget, patch: Value) {
    direct.patch(obj, Patch::Merge(patch)).await.expect("merge patch");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_managed_resource_lives_through_the_assembled_runtime() {
    let Some(config) = rest_config("a_managed_resource_lives_through_the_assembled_runtime").await else {
        return;
    };
    let direct = KubeClient::uncached(config.clone().throttled(100.0, 200).into_kube_client().unwrap());
    let direct: &dyn Client = &direct;

    let mut crd = crd();
    direct.create(&mut crd).await.expect("install the test CRD");
    let crd_key = ObjectKey::cluster(crd.metadata.name.clone().unwrap());
    within("the test CRD to be established", Duration::from_secs(30), || async {
        let got: CustomResourceDefinition = direct.get_as(&crd_key).await.ok()?;
        let conditions = got.status?.conditions?;
        conditions
            .iter()
            .any(|c| c.type_ == "Established" && c.status == "True")
            .then_some(())
    })
    .await;

    let ns = format!("xprs-e2e-{}", std::process::id());
    let mut namespace = namespace(&ns);
    direct.create(&mut namespace).await.expect("create the test namespace");

    let mgr = Manager::new(
        config,
        ManagerOptions {
            metrics_bind_address: "0".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let o = Options {
        max_concurrent_reconciles: 10,
        poll_interval: POLL,
        global_rate_limiter: ratelimiter::new_global(10),
        features: Features::default(),
        gate: Arc::new(Gate::default()),
        metric_options: None,
        change_log_options: None,
    };
    let world = Shared::default();
    let name = managed::controller_name::<Gadget>();
    let r = managed::Reconciler::<Gadget>::builder(mgr.client())
        .external_connector(world.clone())
        .poll_interval(o.poll_interval)
        // Go's 30s default holds a delete made within 30s of the create in the Pending branch
        // (`reconciler.go:1218`, checked before the deletion branch at :1231), as it does here.
        .creation_grace_period(CREATION_GRACE)
        .recorder(mgr.event_recorder_for(&name))
        .build();
    controller::new_managed_by(&mgr)
        .named(&name)
        .with_options(o.for_controller())
        .with_event_filter(resource::desired_state_changed())
        .for_kind::<Gadget>()
        .complete(ratelimiter::Reconciler::new(&name, r, o.global_rate_limiter.clone()))
        .unwrap();
    let stop = tokio_util::sync::CancellationToken::new();
    let running = tokio::spawn({
        let (mgr, stop) = (mgr.clone(), stop.clone());
        async move { mgr.start(stop.cancelled_owned()).await }
    });

    let outcome = AssertUnwindSafe(lifecycle(direct, &ns, &name, &world))
        .catch_unwind()
        .await;

    stop.cancel();
    let stopped = running.await;
    eprintln!("external writes: {:?}", world.writes());
    eprintln!("events: {:#?}", events(direct, &ns, "g").await);
    cleanup(direct, &ns, &namespace, &crd, &crd_key).await;
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
    stopped.unwrap().expect("the manager stops cleanly");
}

/// Strips finalizers a failed run left, so deleting the namespace and the CRD cannot hang.
async fn cleanup(
    direct: &dyn Client,
    ns: &str,
    namespace: &k8s_openapi::api::core::v1::Namespace,
    crd: &CustomResourceDefinition,
    crd_key: &ObjectKey,
) {
    let options = ListOptions {
        namespace: Some(ns.to_owned()),
        ..Default::default()
    };
    for mut g in direct.list_as::<Gadget>(&options).await.unwrap_or_default() {
        let _ = direct
            .patch(&mut g, Patch::Merge(json!({"metadata": {"finalizers": null}})))
            .await;
    }
    direct.delete(namespace).await.expect("delete the test namespace");
    direct.delete(crd).await.expect("delete the test CRD");
    within("the test CRD to go", Duration::from_secs(60), || async {
        direct
            .get_as::<CustomResourceDefinition>(crd_key)
            .await
            .err()
            .filter(|e| e.is_not_found())
    })
    .await;
}

async fn lifecycle(direct: &dyn Client, ns: &str, controller: &str, world: &Shared) {
    let key = ObjectKey::namespaced(ns, "g");
    let get = || async { direct.get_as::<Gadget>(&key).await.ok() };
    let settle = Duration::from_secs(20);

    let mut g = Gadget {
        metadata: ObjectMeta {
            name: Some("g".into()),
            namespace: Some(ns.to_owned()),
            ..Default::default()
        },
        spec: GadgetSpec {
            managed: ManagedResourceSpec::default(),
            for_provider: GadgetParameters { field: "x".into() },
        },
        status: None,
    };
    direct.create(&mut g).await.expect("create the gadget");

    let g = within("Synced and Ready after the create", settle, || async {
        let g = get().await?;
        (condition(&g, "Synced") == want("Synced", "True", "ReconcileSuccess")
            && condition(&g, "Ready") == want("Ready", "True", "Available"))
        .then_some(g)
    })
    .await;
    assert_eq!(
        g.metadata.finalizers.as_deref(),
        Some(&["finalizer.managedresource.crossplane.io".to_owned()][..]),
        "the managed finalizer"
    );
    let annotations = g.metadata.annotations.clone().unwrap_or_default();
    assert_eq!(
        annotations.get("crossplane.io/external-name").map(String::as_str),
        Some("g"),
        "the object name becomes the external name"
    );
    assert!(
        annotations.contains_key("crossplane.io/external-create-pending"),
        "{annotations:?}"
    );
    assert!(
        annotations.contains_key("crossplane.io/external-create-succeeded"),
        "{annotations:?}"
    );
    assert!(
        !annotations.contains_key("crossplane.io/external-create-failed"),
        "{annotations:?}"
    );
    assert_eq!(world.writes(), ["create g=x"], "one external create");
    has_event(
        direct,
        ns,
        "g",
        "Normal",
        "CreatedExternalResource",
        "Successfully requested creation of external resource",
        controller,
    )
    .await;

    let mut g = g;
    merge(direct, &mut g, json!({"spec": {"forProvider": {"field": "y"}}})).await;
    within("a spec change to reach the external system", settle, || async {
        world.writes().contains(&"update g=y".to_owned()).then_some(())
    })
    .await;
    has_event(
        direct,
        ns,
        "g",
        "Normal",
        "UpdatedExternalResource",
        "Successfully requested update of external resource",
        controller,
    )
    .await;
    within("at provider to follow the update", settle, || async {
        let g = get().await?;
        (g.status?.at_provider == json!({"field": "y"})).then_some(())
    })
    .await;

    merge(
        direct,
        &mut g,
        json!({"metadata": {"annotations": {"crossplane.io/paused": "true"}}}),
    )
    .await;
    within("the ReconcilePaused condition", settle, || async {
        let g = get().await?;
        (condition(&g, "Synced") == want("Synced", "False", "ReconcilePaused")).then_some(())
    })
    .await;
    has_event(
        direct,
        ns,
        "g",
        "Normal",
        "ReconciliationPaused",
        "Reconciliation is paused either through the `spec.managementPolicies` or the pause annotation",
        controller,
    )
    .await;
    tokio::time::sleep(POLL * 2).await;
    let observes = world.observes();
    let mut g = get().await.unwrap();
    merge(direct, &mut g, json!({"spec": {"forProvider": {"field": "z"}}})).await;
    tokio::time::sleep(POLL * 4).await;
    assert_eq!(
        world.observes(),
        observes,
        "no observe while paused, across four poll intervals"
    );
    assert!(
        !world.writes().contains(&"update g=z".to_owned()),
        "a spec change while paused reaches nothing: {:?}",
        world.writes()
    );

    merge(
        direct,
        &mut g,
        json!({"metadata": {"annotations": {"crossplane.io/paused": null}}}),
    )
    .await;
    within("unpausing to apply the change made while paused", settle, || async {
        world.writes().contains(&"update g=z".to_owned()).then_some(())
    })
    .await;
    within("Synced again after unpausing", settle, || async {
        let g = get().await?;
        (condition(&g, "Synced") == want("Synced", "True", "ReconcileSuccess")).then_some(())
    })
    .await;

    direct.delete(&g).await.expect("delete the gadget");
    within("the external delete and the finalizer removed", settle, || async {
        let gone = direct
            .get_as::<Gadget>(&key)
            .await
            .err()
            .is_some_and(|e| e.is_not_found());
        (gone && world.writes().last().map(String::as_str) == Some("delete g")).then_some(())
    })
    .await;
    has_event(
        direct,
        ns,
        "g",
        "Normal",
        "DeletedExternalResource",
        "Successfully requested deletion of external resource",
        controller,
    )
    .await;
    assert_eq!(
        world.writes(),
        ["create g=x", "update g=y", "update g=z", "delete g"],
        "the external writes, in order"
    );
}
