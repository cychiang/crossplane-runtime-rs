use crossplane_core::v2::{ConnectionDetails, LocalSecretReference};
use crossplane_test::fake::ModernManaged;
use k8s_openapi::api::core::v1::Secret;

use super::*;

// go: pkg/resource TestLocalConnectionSecretFor/Success
#[test]
fn local_connection_secret_for_success() {
    let mut mg = ModernManaged::default();
    mg.metadata.namespace = Some("coolns".into());
    mg.metadata.name = Some("cool".into());
    mg.metadata.uid = Some("definitely-a-uuid".into());
    mg.set_write_connection_secret_to_reference(Some(LocalSecretReference {
        name: "coolsecret".into(),
    }));

    let want: Secret = serde_json::from_value(serde_json::json!({
        "metadata": {
            "namespace": "coolns",
            "name": "coolsecret",
            "ownerReferences": [{
                "apiVersion": "fake.crossplane.io/v1",
                "kind": "ModernManaged",
                "name": "cool",
                "uid": "definitely-a-uuid",
                "controller": true,
                "blockOwnerDeletion": true,
            }],
        },
        "type": "connection.crossplane.io/v1alpha1",
    }))
    .unwrap();
    let got = local_connection_secret_for(&mg, &ConnectionDetails::new()).unwrap();
    crossplane_test::assert_object_eq(&want, &got, "LocalConnectionSecretFor()");
}

#[test]
fn the_secret_carries_the_data_and_none_without_a_reference() {
    let mut mg = ModernManaged::default();
    assert!(local_connection_secret_for(&mg, &ConnectionDetails::new()).is_none());
    mg.set_write_connection_secret_to_reference(Some(LocalSecretReference { name: "s".into() }));
    let data = ConnectionDetails::from([("k".to_owned(), b"v".to_vec())]);
    let got = local_connection_secret_for(&mg, &data).unwrap();
    assert_eq!(got.data.unwrap()["k"].0, b"v");
}
