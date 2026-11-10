//! `#[crossplane(reference(...))]` against what angryjet's `ResolveReferences` does for one
//! single and one list reference, and for the pointer and rename variants.

use crossplane_core::client::{Client, ListOptions};
use crossplane_core::errors::Error;
use crossplane_core::meta;
use crossplane_core::resource::Managed as _;
use crossplane_core::v2::{
    ManagedResourceSpec, ManagedResourceStatus, NamespacedReference, NamespacedSelector, Policy, ResolutionPolicy,
    ResolvePolicy,
};
use crossplane_core::{Inline, Managed};
use crossplane_test::MockClient;
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "test.example.com",
    version = "v1",
    kind = "Vpc",
    namespaced,
    status = "ParentStatus"
)]
#[serde(rename_all = "camelCase")]
struct VpcSpec {
    #[serde(flatten)]
    managed: ManagedResourceSpec,
}

#[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "test.example.com",
    version = "v1",
    kind = "Subnet",
    namespaced,
    status = "ParentStatus"
)]
#[serde(rename_all = "camelCase")]
struct SubnetSpec {
    #[serde(flatten)]
    managed: ManagedResourceSpec,
}

#[derive(Inline, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
struct ParentStatus {
    #[serde(flatten)]
    managed: ManagedResourceStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Params {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vpc_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vpc_id_ref: Option<NamespacedReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vpc_id_selector: Option<NamespacedSelector>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subnet_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    subnet_id_refs: Vec<NamespacedReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subnet_ids_selector: Option<NamespacedSelector>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight_ref: Option<NamespacedReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight_selector: Option<NamespacedSelector>,

    #[serde(default, skip_serializing_if = "String::is_empty")]
    zone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    zone_reference: Option<NamespacedReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    zone_picker: Option<NamespacedSelector>,
}

#[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "test.example.com",
    version = "v1",
    kind = "Instance",
    namespaced,
    status = "ParentStatus"
)]
#[crossplane(reference(path = "for_provider.vpc_id", to = Vpc))]
#[crossplane(reference(path = "for_provider.subnet_ids", to = Subnet, list, ref_field = "subnet_id_refs", selector_field = "subnet_ids_selector"))]
#[crossplane(reference(path = "for_provider.weight", to = Vpc, extractor = "weight_annotation()"))]
#[crossplane(reference(path = "for_provider.zone", to = Vpc, ref_field = "zone_reference", selector_field = "zone_picker"))]
#[serde(rename_all = "camelCase")]
struct InstanceSpec {
    #[serde(flatten)]
    managed: ManagedResourceSpec,
    for_provider: Params,
}

fn weight_annotation() -> crossplane_runtime::reference::ExtractValueFn<Vpc> {
    Box::new(|vpc| {
        vpc.metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get("test/weight"))
            .cloned()
            .unwrap_or_default()
    })
}

fn instance(params: Params) -> Instance {
    let mut i = Instance::new(
        "i",
        InstanceSpec {
            managed: ManagedResourceSpec::default(),
            for_provider: params,
        },
    );
    i.metadata.namespace = Some("ns".into());
    i
}

fn vpc(name: &str, external_name: &str) -> Vpc {
    let mut v = Vpc::new(
        name,
        VpcSpec {
            managed: ManagedResourceSpec::default(),
        },
    );
    meta::set_external_name(&mut v, external_name);
    v
}

fn subnet(name: &str, external_name: &str) -> Subnet {
    let mut s = Subnet::new(
        name,
        SubnetSpec {
            managed: ManagedResourceSpec::default(),
        },
    );
    meta::set_external_name(&mut s, external_name);
    s
}

fn r(name: &str) -> NamespacedReference {
    NamespacedReference {
        name: name.into(),
        ..Default::default()
    }
}

async fn resolve(i: &mut Instance, client: &dyn Client) -> Result<(), Error> {
    i.resolve_references(client)
        .expect("a kind with reference attributes has a resolver")
        .await
}

fn client_serving(vpcs: Vec<Vpc>, subnets: Vec<Subnet>) -> MockClient {
    MockClient {
        get: Some(Box::new(move |t, key| {
            let name = key.name.as_str();
            assert_eq!(
                key.namespace.as_deref(),
                Some("ns"),
                "references default to the referrer's namespace"
            );
            match t.gvk().kind.as_str() {
                "Vpc" => vpcs
                    .iter()
                    .find(|v| v.metadata.name.as_deref() == Some(name))
                    .cloned()
                    .map(|v| Box::new(v) as _),
                "Subnet" => subnets
                    .iter()
                    .find(|s| s.metadata.name.as_deref() == Some(name))
                    .cloned()
                    .map(|s| Box::new(s) as _),
                other => panic!("unexpected kind {other}"),
            }
            .ok_or_else(Error::not_found)
        })),
        list: Some(Box::new(move |t, options: &ListOptions| {
            assert_eq!(options.namespace.as_deref(), Some("ns"));
            Ok(match t.gvk().kind.as_str() {
                "Subnet" => vec![
                    Box::new(subnet("sn-b", "subnet-b")) as _,
                    Box::new(subnet("sn-a", "subnet-a")) as _,
                ],
                "Vpc" => vec![Box::new(vpc("vpc-picked", "vpc-1")) as _],
                other => panic!("unexpected kind {other}"),
            })
        })),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_kind_without_reference_attributes_has_no_resolver() {
    let mut v = vpc("v", "x");
    let client = MockClient::default();
    assert!(v.resolve_references(&client).is_none());
}

#[tokio::test]
async fn a_single_reference_sets_the_value_and_keeps_the_reference() {
    let mut i = instance(Params {
        vpc_id_ref: Some(r("my-vpc")),
        ..Default::default()
    });
    let client = client_serving(vec![vpc("my-vpc", "vpc-123")], vec![]);
    resolve(&mut i, &client).await.unwrap();
    assert_eq!(i.spec.for_provider.vpc_id.as_deref(), Some("vpc-123"));
    assert_eq!(i.spec.for_provider.vpc_id_ref, Some(r("my-vpc")));
}

#[tokio::test]
async fn a_selector_picks_a_match_and_records_the_reference_it_chose() {
    let mut i = instance(Params {
        vpc_id_selector: Some(NamespacedSelector::default()),
        ..Default::default()
    });
    resolve(&mut i, &client_serving(vec![], vec![])).await.unwrap();
    assert_eq!(i.spec.for_provider.vpc_id.as_deref(), Some("vpc-1"));
    assert_eq!(
        i.spec.for_provider.vpc_id_ref,
        Some(NamespacedReference {
            name: "vpc-picked".into(),
            namespace: "ns".into(),
            policy: None
        })
    );
}

#[tokio::test]
async fn a_value_that_is_already_set_is_not_resolved_again() {
    let mut i = instance(Params {
        vpc_id: Some("already".into()),
        vpc_id_ref: Some(r("my-vpc")),
        ..Default::default()
    });
    resolve(&mut i, &MockClient::default()).await.unwrap();
    assert_eq!(i.spec.for_provider.vpc_id.as_deref(), Some("already"));
}

#[tokio::test]
async fn the_always_policy_resolves_over_an_existing_value() {
    let mut reference = r("my-vpc");
    reference.policy = Some(Policy {
        resolve: Some(ResolvePolicy::Always),
        ..Default::default()
    });
    let mut i = instance(Params {
        vpc_id: Some("stale".into()),
        vpc_id_ref: Some(reference),
        ..Default::default()
    });
    resolve(&mut i, &client_serving(vec![vpc("my-vpc", "fresh")], vec![]))
        .await
        .unwrap();
    assert_eq!(i.spec.for_provider.vpc_id.as_deref(), Some("fresh"));
}

#[tokio::test]
async fn a_list_reference_resolves_in_order_and_a_list_selector_sorts_by_value() {
    let mut i = instance(Params {
        subnet_id_refs: vec![r("sn-b"), r("sn-a")],
        ..Default::default()
    });
    let client = client_serving(vec![], vec![subnet("sn-a", "subnet-a"), subnet("sn-b", "subnet-b")]);
    resolve(&mut i, &client).await.unwrap();
    assert_eq!(i.spec.for_provider.subnet_ids, ["subnet-b", "subnet-a"]);
    assert_eq!(i.spec.for_provider.subnet_id_refs, [r("sn-b"), r("sn-a")]);

    let mut i = instance(Params {
        subnet_ids_selector: Some(NamespacedSelector::default()),
        ..Default::default()
    });
    resolve(&mut i, &client).await.unwrap();
    assert_eq!(i.spec.for_provider.subnet_ids, ["subnet-a", "subnet-b"]);
    assert_eq!(
        i.spec
            .for_provider
            .subnet_id_refs
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["sn-a", "sn-b"]
    );
}

#[tokio::test]
async fn pointer_values_and_custom_extractors_and_renamed_fields_resolve_like_the_default() {
    let mut weighted = vpc("w", "ignored");
    weighted
        .metadata
        .annotations
        .get_or_insert_default()
        .insert("test/weight".into(), "42".into());
    let mut i = instance(Params {
        weight_ref: Some(r("w")),
        zone_reference: Some(r("w")),
        ..Default::default()
    });
    resolve(&mut i, &client_serving(vec![weighted], vec![])).await.unwrap();
    assert_eq!(
        i.spec.for_provider.weight,
        Some(42.0),
        "an Option<f64> field resolves through its string form"
    );
    assert_eq!(
        i.spec.for_provider.zone, "ignored",
        "the default extractor is the external name"
    );
}

#[tokio::test]
async fn an_error_names_the_field_and_stops_the_later_references() {
    let mut i = instance(Params {
        vpc_id_ref: Some(r("gone")),
        subnet_id_refs: vec![r("never-looked-up")],
        ..Default::default()
    });
    let err = resolve(&mut i, &client_serving(vec![], vec![])).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "spec.for_provider.vpc_id: cannot get referenced resource: not found"
    );
    assert!(i.spec.for_provider.subnet_ids.is_empty());
}

#[tokio::test]
async fn an_optional_reference_that_is_missing_resolves_to_nothing_without_an_error() {
    let mut reference = r("gone");
    reference.policy = Some(Policy {
        resolution: Some(ResolutionPolicy::Optional),
        ..Default::default()
    });
    let mut i = instance(Params {
        vpc_id_ref: Some(reference),
        ..Default::default()
    });
    resolve(&mut i, &client_serving(vec![], vec![])).await.unwrap();
    assert_eq!(i.spec.for_provider.vpc_id, None);
}

#[tokio::test]
async fn nothing_to_resolve_makes_no_client_calls() {
    let mut i = instance(Params::default());
    resolve(&mut i, &MockClient::default()).await.unwrap();
}
