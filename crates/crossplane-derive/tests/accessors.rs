//! The accessors angryjet generates for a managed resource, a provider config, a usage and their
//! lists (`zz_generated.managed.go`, `.pc.go`, `.pcu.go`, `.managedlist.go`, `.pculist.go` in the
//! Go baseline), reached on kinds the derives are applied to here.

use crossplane_core::resource::{Managed as _, ProviderConfig as _, ProviderConfigUsage as _};
use crossplane_core::v2::{
    Condition, ConditionStatus, ConditionType, ConditionedStatus, LocalSecretReference, ManagedResourceSpec,
    ManagedResourceStatus, ManagementAction, ManagementPolicies, ProviderConfigReference, ProviderConfigStatus,
    TypedProviderConfigUsage, TypedReference,
};
use crossplane_core::{Inline, Managed, ProviderConfig, ProviderConfigUsage};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "test.example.com",
    version = "v1",
    kind = "Widget",
    namespaced,
    status = "WidgetStatus"
)]
#[serde(rename_all = "camelCase")]
struct WidgetSpec {
    #[serde(flatten)]
    managed: ManagedResourceSpec,
    size: i64,
}

#[derive(Inline, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct WidgetStatus {
    #[serde(flatten)]
    managed: ManagedResourceStatus,
    phase: String,
}

#[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "test.example.com",
    version = "v1",
    kind = "Gadget",
    namespaced,
    status = "WidgetStatus"
)]
#[crossplane(track_reconcile_requests)]
#[serde(rename_all = "camelCase")]
struct GadgetSpec {
    size: i64,
    #[serde(flatten)]
    managed: ManagedResourceSpec,
}

fn widget() -> Widget {
    Widget::new(
        "w",
        WidgetSpec {
            managed: ManagedResourceSpec::default(),
            size: 1,
        },
    )
}

macro_rules! assert_cond {
    ($got:expr, $want:expr) => {{
        let (got, want) = (&$got, &$want);
        assert!(got.equal(want), "-want {want:?}, +got {got:?}");
    }};
}

fn ready() -> Condition {
    Condition::available()
}

#[test]
fn managed_get_condition_reads_the_embedded_status() {
    let mut w = widget();
    let unset = w.get_condition(&ConditionType::READY);
    assert_cond!(unset, ConditionedStatus::default().get_condition(&ConditionType::READY));
    assert_eq!(unset.status, ConditionStatus::Unknown);

    w.status = Some(WidgetStatus {
        managed: ManagedResourceStatus {
            conditioned: ConditionedStatus::new([ready()]),
            ..Default::default()
        },
        phase: "x".into(),
    });
    assert_cond!(w.get_condition(&ConditionType::READY), ready());
    assert_eq!(
        w.get_condition(&ConditionType::SYNCED).status,
        ConditionStatus::Unknown,
        "a type that was not set stays Unknown"
    );
}

#[test]
fn managed_set_conditions_writes_the_embedded_status_and_replaces_by_type() {
    let mut w = widget();
    assert!(w.status.is_none());
    w.set_conditions([ready()]);
    assert!(
        w.status
            .as_ref()
            .expect("set_conditions creates the status")
            .managed
            .conditioned
            .equal(&ConditionedStatus::new([ready()]))
    );
    w.set_conditions([Condition::unavailable(), Condition::reconcile_success()]);
    assert_cond!(w.get_condition(&ConditionType::READY), Condition::unavailable());
    assert_cond!(w.get_condition(&ConditionType::SYNCED), Condition::reconcile_success());
    assert_eq!(
        w.status.as_ref().unwrap().phase,
        "",
        "the kind's own status fields are untouched"
    );
}

#[test]
fn managed_provider_config_reference_round_trips_through_the_spec() {
    let mut w = widget();
    assert_eq!(w.provider_config_reference(), None);
    let r = ProviderConfigReference {
        kind: "ProviderConfig".into(),
        name: "default".into(),
    };
    w.set_provider_config_reference(Some(r.clone()));
    assert_eq!(w.provider_config_reference(), Some(&r));
    assert_eq!(w.spec.managed.provider_config_ref, Some(r.clone()));
    assert_eq!(
        serde_json::to_value(&w).unwrap()["spec"]["providerConfigRef"],
        json!({"kind": "ProviderConfig", "name": "default"})
    );
    w.set_provider_config_reference(None);
    assert_eq!(w.provider_config_reference(), None);
}

#[test]
fn managed_write_connection_secret_to_reference_round_trips_through_the_spec() {
    let mut w = widget();
    assert_eq!(w.write_connection_secret_to_reference(), None);
    let r = LocalSecretReference { name: "conn".into() };
    w.set_write_connection_secret_to_reference(Some(r.clone()));
    assert_eq!(w.write_connection_secret_to_reference(), Some(&r));
    assert_eq!(w.spec.managed.write_connection_secret_to_ref, Some(r));
    assert_eq!(
        serde_json::to_value(&w).unwrap()["spec"]["writeConnectionSecretToRef"],
        json!({"name": "conn"})
    );
    w.set_write_connection_secret_to_reference(None);
    assert_eq!(w.write_connection_secret_to_reference(), None);
}

#[test]
fn managed_management_policies_round_trip_through_the_spec() {
    let mut w = widget();
    assert_eq!(w.management_policies(), &ManagementPolicies::default());
    let p = ManagementPolicies(vec![ManagementAction::Observe, ManagementAction::Update]);
    w.set_management_policies(p.clone());
    assert_eq!(w.management_policies(), &p);
    assert_eq!(w.spec.managed.management_policies, p);
    assert_eq!(
        serde_json::to_value(&w).unwrap()["spec"]["managementPolicies"],
        json!(["Observe", "Update"])
    );
}

#[test]
fn managed_finds_the_embedded_spec_wherever_it_is_declared() {
    let mut g = Gadget::new(
        "g",
        GadgetSpec {
            size: 2,
            managed: ManagedResourceSpec::default(),
        },
    );
    g.set_write_connection_secret_to_reference(Some(LocalSecretReference { name: "c".into() }));
    assert_eq!(
        g.spec.managed.write_connection_secret_to_ref.as_ref().unwrap().name,
        "c"
    );
}

#[test]
fn managed_tracks_reconcile_requests_only_when_the_kind_says_so() {
    const WIDGET: bool = <Widget as crossplane_core::resource::Managed>::TRACKS_RECONCILE_REQUESTS;
    const GADGET: bool = <Gadget as crossplane_core::resource::Managed>::TRACKS_RECONCILE_REQUESTS;
    assert_eq!((false, true), (WIDGET, GADGET));

    let mut g = Gadget::new(
        "g",
        GadgetSpec {
            size: 2,
            managed: ManagedResourceSpec::default(),
        },
    );
    assert_eq!(g.last_handled_reconcile_at(), "");
    g.set_last_handled_reconcile_at("token");
    g.set_observed_generation(4);
    assert_eq!(g.last_handled_reconcile_at(), "token");
    assert_eq!(g.observed_generation(), 4);
    let status = serde_json::to_value(&g).unwrap()["status"].clone();
    assert_eq!(status["lastHandledReconcileAt"], "token");
    assert_eq!(status["observedGeneration"], 4);
}

#[test]
fn managed_status_accessor_does_not_create_a_status_on_read() {
    let w = widget();
    assert!(w.managed_status().is_none());
    assert_eq!(w.observed_generation(), 0);
    assert!(w.status.is_none());
}

#[derive(CustomResource, ProviderConfig, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "test.example.com",
    version = "v1",
    kind = "WidgetConfig",
    namespaced,
    status = "WidgetConfigStatus"
)]
struct WidgetConfigSpec {
    region: String,
}

#[derive(Inline, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
struct WidgetConfigStatus {
    #[serde(flatten)]
    inner: ProviderConfigStatus,
}

fn widget_config() -> WidgetConfig {
    WidgetConfig::new("c", WidgetConfigSpec { region: "r".into() })
}

#[test]
fn provider_config_get_and_set_users() {
    let mut c = widget_config();
    assert_eq!(c.users(), 0);
    c.set_users(3);
    assert_eq!(c.users(), 3);
    assert_eq!(c.status.as_ref().unwrap().inner.users, 3);
    assert_eq!(serde_json::to_value(&c).unwrap()["status"]["users"], 3);
}

#[test]
fn provider_config_get_and_set_conditions() {
    let mut c = widget_config();
    assert_cond!(
        c.get_condition(&ConditionType::READY),
        ConditionedStatus::default().get_condition(&ConditionType::READY)
    );
    c.set_conditions([ready()]);
    assert_cond!(c.get_condition(&ConditionType::READY), ready());
    assert!(
        c.status
            .as_ref()
            .unwrap()
            .inner
            .conditioned
            .equal(&ConditionedStatus::new([ready()]))
    );
    c.set_conditions([Condition::unavailable()]);
    assert_cond!(c.get_condition(&ConditionType::READY), Condition::unavailable());
}

#[test]
fn provider_config_reads_do_not_create_a_status() {
    let c = widget_config();
    assert!(c.provider_config_status().is_none());
    assert_eq!(c.users(), 0);
    assert!(c.status.is_none());
}

#[derive(CustomResource, ProviderConfigUsage, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "test.example.com",
    version = "v1",
    kind = "WidgetConfigUsage",
    namespaced,
    no_spec
)]
struct WidgetConfigUsageFields {
    #[serde(flatten)]
    usage: TypedProviderConfigUsage,
}

fn usage_value() -> TypedProviderConfigUsage {
    TypedProviderConfigUsage {
        provider_config_ref: ProviderConfigReference {
            kind: "WidgetConfig".into(),
            name: "c".into(),
        },
        resource_ref: TypedReference {
            api_version: "test.example.com/v1".into(),
            kind: "Widget".into(),
            name: "w".into(),
            uid: "u".into(),
        },
    }
}

#[test]
fn usage_getters_read_the_inlined_references() {
    let u = WidgetConfigUsage::from_usage(ObjectMeta::default(), usage_value());
    assert_eq!(u.provider_config_reference(), &usage_value().provider_config_ref);
    assert_eq!(u.resource_reference(), &usage_value().resource_ref);
}

#[test]
fn usage_setters_write_the_inlined_references() {
    let mut u = WidgetConfigUsage::from_usage(ObjectMeta::default(), TypedProviderConfigUsage::default());
    u.set_provider_config_reference(usage_value().provider_config_ref);
    u.set_resource_reference(usage_value().resource_ref);
    assert_eq!(u.spec.usage, usage_value());
    assert_eq!(
        serde_json::to_value(&u).unwrap()["providerConfigRef"],
        json!({"kind": "WidgetConfig", "name": "c"}),
        "the references sit at the root of the object, as Go's inline embed puts them"
    );
    assert_eq!(serde_json::to_value(&u).unwrap()["resourceRef"]["uid"], "u");
}

#[test]
fn usage_from_usage_keeps_the_metadata_it_is_given() {
    let metadata = ObjectMeta {
        name: Some("usage-1".into()),
        namespace: Some("ns".into()),
        ..Default::default()
    };
    let u = WidgetConfigUsage::from_usage(metadata.clone(), usage_value());
    assert_eq!(u.metadata, metadata);
    assert_eq!(u.usage(), &usage_value());
}

/// Go's `GetItems` on `MyTypeList` and `ProviderConfigUsageList` turns a list's `Items` into
/// `[]resource.Managed` and `[]resource.ProviderConfigUsage`. A Rust list is `Vec<K>` straight
/// from the client, and the derive has nothing to add.
#[tokio::test]
async fn lists_need_no_derive_because_the_client_returns_typed_items() {
    use crossplane_core::client::ListOptions;
    use crossplane_test::{MockClient, new_mock_list_fn};

    let client = MockClient {
        list: Some(new_mock_list_fn(|| vec![widget(), widget()])),
        ..Default::default()
    };
    let client: &dyn crossplane_core::client::Client = &client;
    let items: Vec<Widget> = client.list_as(&ListOptions::default()).await.unwrap();
    assert_eq!(items.len(), 2);
    for item in &items {
        assert_eq!(
            item.get_condition(&ConditionType::READY).status,
            ConditionStatus::Unknown
        );
    }
}

#[derive(Inline, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
struct Embedding {
    #[serde(flatten)]
    conditioned: ConditionedStatus,
    #[serde(flatten)]
    managed: ManagedResourceStatus,
    #[serde(default)]
    plain: ProviderConfigStatus,
}

#[test]
fn inline_exposes_every_flattened_field_and_only_those() {
    let mut e = Embedding::default();
    <Embedding as AsMut<ConditionedStatus>>::as_mut(&mut e).set_conditions([ready()]);
    <Embedding as AsMut<ManagedResourceStatus>>::as_mut(&mut e)
        .observed
        .observed_generation = 7;
    assert_cond!(
        <Embedding as AsRef<ConditionedStatus>>::as_ref(&e).get_condition(&ConditionType::READY),
        ready()
    );
    assert_eq!(e.managed.observed.observed_generation, 7);
    assert_eq!(
        <Embedding as AsRef<ManagedResourceStatus>>::as_ref(&e)
            .observed
            .observed_generation,
        7
    );
}

/// A type that already is the embedded struct satisfies the bound without `Inline`.
#[test]
fn the_embedded_structs_are_their_own_inline_target() {
    fn takes<S: AsRef<ManagedResourceStatus> + AsMut<ManagedResourceStatus>>(_: &S) {}
    takes(&ManagedResourceStatus::default());
}
