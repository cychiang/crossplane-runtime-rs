use crossplane_core::meta;
use crossplane_core::resource::{Managed, ObjectType};
use crossplane_core::v2::ConnectionDetails;
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

pub const CONNECTION_SECRET_TYPE: &str = "connection.crossplane.io/v1alpha1";

/// Go `resource.LocalConnectionSecretFor`: the Secret in the MR's namespace, named by
/// `writeConnectionSecretToRef`, controller-owned by the MR, holding `data`. `None` when the MR
/// has no reference, where Go would dereference nil.
pub fn local_connection_secret_for<M: Managed>(mg: &M, data: &ConnectionDetails) -> Option<Secret> {
    let secret_ref = mg.write_connection_secret_to_reference()?;
    let owner = meta::typed_reference_to(mg, &ObjectType::of::<M>().gvk());
    Some(Secret {
        metadata: ObjectMeta {
            namespace: mg.meta().namespace.clone(),
            name: Some(secret_ref.name.clone()),
            owner_references: Some(vec![meta::as_controller(&owner)]),
            ..Default::default()
        },
        type_: Some(CONNECTION_SECRET_TYPE.to_owned()),
        data: (!data.is_empty()).then(|| data.iter().map(|(k, v)| (k.clone(), ByteString(v.clone()))).collect()),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "connection_test.rs"]
mod tests;
