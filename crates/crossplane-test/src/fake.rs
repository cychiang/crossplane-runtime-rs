//! Go `pkg/resource/fake`: kinds that are not CRDs, for runtime tables.

use std::borrow::Cow;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Resource;
use kube::core::NamespaceResourceScope;
use serde::{Deserialize, Serialize};

use crossplane_core::resource::{Managed, Object};
use crossplane_core::v2::{ManagedResourceSpec, ManagedResourceStatus};

/// Go `fake.ModernManaged`. It has `ObservedStatus`, so it tracks reconcile requests.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModernManaged {
    pub metadata: ObjectMeta,
    #[serde(default)]
    pub spec: ManagedResourceSpec,
    #[serde(default)]
    pub status: ManagedResourceStatus,
}

impl Resource for ModernManaged {
    type DynamicType = ();
    type Scope = NamespaceResourceScope;

    fn kind(_: &()) -> Cow<'_, str> {
        "ModernManaged".into()
    }
    fn group(_: &()) -> Cow<'_, str> {
        "fake.crossplane.io".into()
    }
    fn version(_: &()) -> Cow<'_, str> {
        "v1".into()
    }
    fn plural(_: &()) -> Cow<'_, str> {
        "modernmanageds".into()
    }
    fn meta(&self) -> &ObjectMeta {
        &self.metadata
    }
    fn meta_mut(&mut self) -> &mut ObjectMeta {
        &mut self.metadata
    }
}

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
pub fn as_modern_managed(obj: &mut dyn Object, generation: i64) -> &mut ModernManaged {
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
