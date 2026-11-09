use crossplane_core::resource::Managed;
use crossplane_core::v2::ConnectionDetails;
use k8s_openapi::api::core::v1::Secret;

pub const CONNECTION_SECRET_TYPE: &str = "connection.crossplane.io/v1alpha1";

/// Go `resource.LocalConnectionSecretFor`: the Secret in the MR's namespace, named by
/// `writeConnectionSecretToRef`, controller-owned by the MR.
pub fn local_connection_secret_for<M: Managed>(mg: &M, data: &ConnectionDetails) -> Option<Secret> {
    let _ = (mg, data);
    todo!("pkg/resource/resource.go:96")
}
