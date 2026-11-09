//! Go `pkg/resource/fake`: kinds that are not CRDs, for runtime tables.
//!
//! Go's fakes embed one small struct per capability (`Manageable`, `TypedProviderConfigReferencer`
//! and so on). Here each fake has the wire shape of a real kind, and the accessors those embeds
//! provide in Go are the provided methods of the `crossplane_core::resource` traits. All fakes are
//! in group `fake.crossplane.io`, version `v1`; Go's `fake.GVK` and `fake.SchemeWith` are
//! `ObjectType::of::<K>()`.

use std::borrow::Cow;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Resource;
use kube::core::NamespaceResourceScope;
use serde::{Deserialize, Serialize};

use crossplane_core::resource::{self, Managed};
use crossplane_core::v2::{ManagedResourceSpec, ManagedResourceStatus, ProviderConfigStatus, TypedProviderConfigUsage};

macro_rules! fake_kind {
    ($ty:ident, $plural:literal) => {
        impl Resource for $ty {
            type DynamicType = ();
            type Scope = NamespaceResourceScope;

            fn kind(_: &()) -> Cow<'_, str> {
                stringify!($ty).into()
            }
            fn group(_: &()) -> Cow<'_, str> {
                "fake.crossplane.io".into()
            }
            fn version(_: &()) -> Cow<'_, str> {
                "v1".into()
            }
            fn plural(_: &()) -> Cow<'_, str> {
                $plural.into()
            }
            fn meta(&self) -> &ObjectMeta {
                &self.metadata
            }
            fn meta_mut(&mut self) -> &mut ObjectMeta {
                &mut self.metadata
            }
        }
    };
}

/// Go `fake.Object`: metadata and nothing else.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Object {
    pub metadata: ObjectMeta,
}

fake_kind!(Object, "objects");

/// Go `fake.ModernManaged`. It has `ObservedStatus`, so it tracks reconcile requests. It also
/// serves where a Go table uses `fake.Managed`, which is the same minus the two references.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModernManaged {
    pub metadata: ObjectMeta,
    #[serde(default)]
    pub spec: ManagedResourceSpec,
    #[serde(default)]
    pub status: ManagedResourceStatus,
}

fake_kind!(ModernManaged, "modernmanageds");

impl Managed for ModernManaged {
    const TRACKS_RECONCILE_REQUESTS: bool = true;

    fn managed_spec(&self) -> &ManagedResourceSpec {
        &self.spec
    }
    fn managed_spec_mut(&mut self) -> &mut ManagedResourceSpec {
        &mut self.spec
    }
    fn managed_status(&self) -> Option<&ManagedResourceStatus> {
        Some(&self.status)
    }
    fn managed_status_mut(&mut self) -> &mut ManagedResourceStatus {
        &mut self.status
    }
}

/// Go `asModernManaged(obj, generation)`.
pub fn as_modern_managed(obj: &mut dyn resource::Object, generation: i64) -> &mut ModernManaged {
    let mg = obj
        .downcast_mut::<ModernManaged>()
        .expect("object is a fake::ModernManaged");
    mg.metadata.generation = Some(generation);
    mg
}

/// Go `newModernManaged(generation)`.
pub fn new_modern_managed(generation: i64) -> ModernManaged {
    let mut mg = ModernManaged::default();
    mg.metadata.generation = Some(generation);
    mg
}

/// Go `fake.ProviderConfig`: a user count and conditions. Namespaced or cluster scoped by whether
/// `metadata.namespace` is set, as the Go fake is.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub metadata: ObjectMeta,
    #[serde(default)]
    pub status: ProviderConfigStatus,
}

fake_kind!(ProviderConfig, "providerconfigs");

impl resource::ProviderConfig for ProviderConfig {
    fn provider_config_status(&self) -> Option<&ProviderConfigStatus> {
        Some(&self.status)
    }
    fn provider_config_status_mut(&mut self) -> &mut ProviderConfigStatus {
        &mut self.status
    }
}

/// Go `fake.ProviderConfigUsage`: the typed provider config and resource references, inlined at
/// the root as a real usage kind has them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfigUsage {
    pub metadata: ObjectMeta,
    #[serde(flatten)]
    pub usage: TypedProviderConfigUsage,
}

fake_kind!(ProviderConfigUsage, "providerconfigusages");

impl resource::ProviderConfigUsage for ProviderConfigUsage {
    fn from_usage(metadata: ObjectMeta, usage: TypedProviderConfigUsage) -> Self {
        Self { metadata, usage }
    }
    fn usage(&self) -> &TypedProviderConfigUsage {
        &self.usage
    }
    fn usage_mut(&mut self) -> &mut TypedProviderConfigUsage {
        &mut self.usage
    }
}

/// Go `pkg/resource/interfaces_test.go`: the fakes satisfy the traits. The legacy and
/// composition fakes have no Rust counterpart.
const _: () = {
    const fn managed<T: Managed>() {}
    const fn provider_config<T: resource::ProviderConfig>() {}
    const fn usage<T: resource::ProviderConfigUsage>() {}
    managed::<ModernManaged>();
    provider_config::<ProviderConfig>();
    usage::<ProviderConfigUsage>();
};
