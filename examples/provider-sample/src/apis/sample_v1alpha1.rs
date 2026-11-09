//! Go `apis/sample/v1alpha1/mytype_types.go` and its generated method sets.

use crossplane_core::v2::{ManagedResourceSpec, ManagedResourceStatus};
use crossplane_core::{Inline, Managed};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The configurable fields of a MyType.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MyTypeParameters {
    pub configurable_field: String,
}

/// The observable fields of a MyType.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MyTypeObservation {
    pub configurable_field: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
}

/// A MyTypeSpec defines the desired state of a MyType.
///
/// Neither the spec nor the generated `MyType` implements `Default`. The runtime never needs an
/// empty kind, so required fields stay required.
#[derive(CustomResource, Managed, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "sample.example.com",
    version = "v1alpha1",
    kind = "MyType",
    namespaced,
    status = "MyTypeStatus",
    shortname = "mytype",
    category = "crossplane",
    category = "managed",
    category = "provider-baseline",
    printcolumn = r#"{"name":"READY","type":"string","jsonPath":".status.conditions[?(@.type=='Ready')].status"}"#,
    printcolumn = r#"{"name":"SYNCED","type":"string","jsonPath":".status.conditions[?(@.type=='Synced')].status"}"#,
    printcolumn = r#"{"name":"EXTERNAL-NAME","type":"string","jsonPath":".metadata.annotations.crossplane\\.io/external-name"}"#,
    printcolumn = r#"{"name":"AGE","type":"date","jsonPath":".metadata.creationTimestamp"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct MyTypeSpec {
    #[serde(flatten)]
    pub managed: ManagedResourceSpec,
    pub for_provider: MyTypeParameters,
}

/// A MyTypeStatus represents the observed state of a MyType.
#[derive(Inline, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MyTypeStatus {
    #[serde(flatten)]
    pub managed: ManagedResourceStatus,
    #[serde(default)]
    #[schemars(transform = crossplane_core::v2::schema::no_default)]
    pub at_provider: MyTypeObservation,
}
