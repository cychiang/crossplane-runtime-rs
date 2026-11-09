//! Go `apis/v1alpha1/types.go` and its generated method sets.

use crossplane_core::v2::{self as xpv2, CredentialsSource, LocalSecretKeySelector, TypedProviderConfigUsage};
use crossplane_core::{Inline, ProviderConfig, ProviderConfigUsage};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A ProviderConfigStatus defines the status of a Provider.
#[derive(Inline, Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProviderConfigStatus {
    #[serde(flatten)]
    pub inner: xpv2::ProviderConfigStatus,
}

/// ProviderCredentials required to authenticate.
///
/// Credentials are confined to this ProviderConfig's own namespace: SecretRef
/// is a LocalSecretKeySelector, and the Filesystem/Environment sources are not
/// offered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCredentials {
    /// Source of the provider credentials.
    #[schemars(extend("enum" = ["None", "Secret", "InjectedIdentity"]))]
    pub source: CredentialsSource,
    /// A SecretRef is a reference to a secret key in this ProviderConfig's
    /// namespace that contains the credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<LocalSecretKeySelector>,
}

/// A ProviderConfig configures a provider-baseline provider.
#[derive(CustomResource, ProviderConfig, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "example.com",
    version = "v1alpha1",
    kind = "ProviderConfig",
    namespaced,
    status = "ProviderConfigStatus",
    category = "crossplane",
    category = "provider",
    category = "provider-baseline",
    printcolumn = r#"{"name":"AGE","type":"date","jsonPath":".metadata.creationTimestamp"}"#,
    printcolumn = r#"{"name":"SECRET-NAME","type":"string","jsonPath":".spec.credentials.secretRef.name","priority":1}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigSpec {
    /// Credentials required to authenticate to this provider.
    pub credentials: ProviderCredentials,
}

/// A ProviderConfigUsage indicates that a resource is using a ProviderConfig.
/// Usages live in the namespace of the managed resource that created them.
///
/// Go inlines `TypedProviderConfigUsage` at the root of the object. `no_spec` plus one flattened
/// field is the same wire shape.
#[derive(CustomResource, ProviderConfigUsage, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "example.com",
    version = "v1alpha1",
    kind = "ProviderConfigUsage",
    namespaced,
    no_spec,
    category = "crossplane",
    category = "provider",
    category = "provider-baseline",
    printcolumn = r#"{"name":"AGE","type":"date","jsonPath":".metadata.creationTimestamp"}"#,
    printcolumn = r#"{"name":"CONFIG-NAME","type":"string","jsonPath":".providerConfigRef.name"}"#,
    printcolumn = r#"{"name":"RESOURCE-KIND","type":"string","jsonPath":".resourceRef.kind"}"#,
    printcolumn = r#"{"name":"RESOURCE-NAME","type":"string","jsonPath":".resourceRef.name"}"#
)]
pub struct ProviderConfigUsageFields {
    #[serde(flatten)]
    pub usage: TypedProviderConfigUsage,
}
