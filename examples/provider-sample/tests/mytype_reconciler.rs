//! A table case against the provider's own kind, from outside the runtime crate. `MyType` has no
//! `Default`, so this also proves the seam needs none.

use std::sync::Arc;

use crossplane_core::errors::{Error, Result};
use crossplane_core::resource::ObjectType;
use crossplane_runtime::reconciler::managed::{ERR_GET_MANAGED, Reconciler};
use crossplane_runtime::reconciler::{Outcome, Reconcile, Request};
use crossplane_test::{self as test, MockClient};
use provider_sample::apis::sample_v1alpha1::MyType;

#[tokio::test]
async fn get_managed_error() {
    let client = MockClient {
        get: Some(Box::new(|object_type, _| {
            assert_eq!(
                object_type,
                &ObjectType::of::<MyType>(),
                "the reconciler fetches a MyType"
            );
            Err(Error::new("boom"))
        })),
        ..Default::default()
    };
    let r = Reconciler::<MyType>::builder(Arc::new(client)).build();
    let got = r.reconcile(&Request::default()).await;
    let want: Result<Outcome> = Err(Error::new("boom").wrap(ERR_GET_MANAGED));
    test::assert_result(
        "GetManagedError",
        "errors other than not found are returned",
        &want,
        &got,
    );
}
