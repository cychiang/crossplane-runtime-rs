//! Go `apis/`. There is no `AddToScheme`: Rust has no scheme, kinds are types.

pub mod sample_v1alpha1;
pub mod v1alpha1;

use crossplane_core::v2::schema::controller_gen_compat;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::CustomResourceExt;

/// Go `package/crds`, in controller-gen's shape.
pub fn crds() -> Vec<CustomResourceDefinition> {
    [
        sample_v1alpha1::MyType::crd(),
        v1alpha1::ProviderConfig::crd(),
        v1alpha1::ProviderConfigUsage::crd(),
    ]
    .into_iter()
    .map(controller_gen_compat)
    .collect()
}
