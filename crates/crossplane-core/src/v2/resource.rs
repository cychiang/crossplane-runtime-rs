use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::condition::ConditionedStatus;
use super::policies::ManagementPolicies;

pub const LABEL_KEY_PROVIDER_KIND: &str = "crossplane.io/provider-config-kind";
pub const LABEL_KEY_PROVIDER_NAME: &str = "crossplane.io/provider-config";

/// `xpv2.ManagedResourceSpec` (modern, namespaced).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManagedResourceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_connection_secret_to_ref: Option<LocalSecretReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("default" = {"kind": "ClusterProviderConfig", "name": "default"}))]
    pub provider_config_ref: Option<ProviderConfigReference>,
    #[serde(default, skip_serializing_if = "ManagementPolicies::is_empty")]
    #[schemars(extend("default" = ["*"]))]
    pub management_policies: ManagementPolicies,
}

/// `xpv2.ObservedStatus`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ObservedStatus {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub observed_generation: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_handled_reconcile_at: String,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

/// `xpv2.ManagedResourceStatus`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ManagedResourceStatus {
    #[serde(flatten)]
    pub conditioned: ConditionedStatus,
    #[serde(flatten)]
    pub observed: ObservedStatus,
}

impl AsRef<ManagedResourceStatus> for ManagedResourceStatus {
    fn as_ref(&self) -> &ManagedResourceStatus {
        self
    }
}
impl AsMut<ManagedResourceStatus> for ManagedResourceStatus {
    fn as_mut(&mut self) -> &mut ManagedResourceStatus {
        self
    }
}

/// `xpv2.LocalSecretReference`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LocalSecretReference {
    pub name: String,
}

/// `xpv2.SecretReference`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretReference {
    pub name: String,
    pub namespace: String,
}

/// `xpv2.ProviderConfigReference`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderConfigReference {
    pub kind: String,
    pub name: String,
}

/// `xpv2.TypedReference`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypedReference {
    pub api_version: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub uid: String,
}

/// `xpv2.SecretKeySelector`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SecretKeySelector {
    #[serde(flatten)]
    pub secret: SecretReference,
    pub key: String,
}

/// `xpv2.LocalSecretKeySelector`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LocalSecretKeySelector {
    #[serde(flatten)]
    pub secret: LocalSecretReference,
    pub key: String,
}

impl LocalSecretKeySelector {
    pub fn to_secret_key_selector(&self, namespace: &str) -> SecretKeySelector {
        SecretKeySelector {
            secret: SecretReference {
                name: self.secret.name.clone(),
                namespace: namespace.to_owned(),
            },
            key: self.key.clone(),
        }
    }
}

/// `xpv2.CredentialsSource`. An open string type in Go, so a string newtype here: the CRD has no
/// `enum` unless the provider adds one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CredentialsSource(pub Cow<'static, str>);

impl CredentialsSource {
    pub const NONE: Self = Self(Cow::Borrowed("None"));
    pub const SECRET: Self = Self(Cow::Borrowed("Secret"));
    pub const INJECTED_IDENTITY: Self = Self(Cow::Borrowed("InjectedIdentity"));
    pub const ENVIRONMENT: Self = Self(Cow::Borrowed("Environment"));
    pub const FILESYSTEM: Self = Self(Cow::Borrowed("Filesystem"));
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnvSelector {
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FsSelector {
    pub path: String,
}

/// `xpv2.CommonCredentialSelectors`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommonCredentialSelectors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<FsSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<EnvSelector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<SecretKeySelector>,
}

/// `xpv2.ProviderConfigStatus`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProviderConfigStatus {
    #[serde(flatten)]
    pub conditioned: ConditionedStatus,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub users: i64,
}

impl AsRef<ProviderConfigStatus> for ProviderConfigStatus {
    fn as_ref(&self) -> &ProviderConfigStatus {
        self
    }
}
impl AsMut<ProviderConfigStatus> for ProviderConfigStatus {
    fn as_mut(&mut self) -> &mut ProviderConfigStatus {
        self
    }
}

/// `xpv2.TypedProviderConfigUsage`, inlined at the root of a usage kind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TypedProviderConfigUsage {
    pub provider_config_ref: ProviderConfigReference,
    pub resource_ref: TypedReference,
}

/// Go `managed.ConnectionDetails`. Ordered, so the Secret body is stable in the audit diff.
pub type ConnectionDetails = BTreeMap<String, Vec<u8>>;
