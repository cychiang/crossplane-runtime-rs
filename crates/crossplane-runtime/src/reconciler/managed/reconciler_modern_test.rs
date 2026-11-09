//! Port of `pkg/reconciler/managed/reconciler_modern_test.go` `TestModernReconciler`, case for
//! case. Each row carries a `// go:` marker that `hack/port-coverage.sh` counts.

use std::sync::Arc;

use crossplane_core::errors::{Error, Result};
use crossplane_core::meta;
use crossplane_core::resource::Managed;
use crossplane_core::v2::Condition;
use crossplane_test::fake::{ModernManaged, new_modern_managed};
use crossplane_test::{self as test, MockClient};

use super::*;
use crate::reconciler::{Outcome, Reconcile, Request};

type Opts = fn(ReconcilerBuilder<ModernManaged>) -> ReconcilerBuilder<ModernManaged>;

struct Case {
    name: &'static str,
    reason: &'static str,
    client: MockClient,
    opts: Opts,
    want: Result<Outcome>,
}

#[tokio::test]
async fn modern_reconciler() {
    let err_boom = || Error::new("boom");
    let now = test::now();

    let cases: Vec<Case> = vec![
        // go: pkg/reconciler/managed TestModernReconciler/GetManagedError
        Case {
            name: "GetManagedError",
            reason: "Any error (except not found) encountered while getting the resource under reconciliation should be returned.",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(err_boom())),
                ..Default::default()
            },
            opts: |b| b,
            want: Err(err_boom().wrap(ERR_GET_MANAGED)),
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalCreatePending
        Case {
            name: "ExternalCreatePending",
            reason: "We should return early if the managed resource appears to be pending creation. We might have leaked a resource and don't want to create another.",
            client: MockClient {
                get: Some(test::new_mock_get_fn(move || {
                    let mut mg = new_modern_managed(42);
                    meta::set_external_create_pending(&mut mg, now);
                    mg
                })),
                update_status: Some(test::mock_status_update_fn(move |obj| {
                    let mut want = new_modern_managed(42);
                    meta::set_external_create_pending(&mut want, now);
                    want.set_conditions([
                        Condition::creating().with_observed_generation(42),
                        Condition::reconcile_error(&Error::new(ERR_CREATE_INCOMPLETE)).with_observed_generation(42),
                    ]);
                    test::assert_object_eq(
                        &want,
                        obj,
                        "We should update our status when we're asked to reconcile a managed resource that is pending creation.",
                    );
                    Ok(())
                })),
                ..Default::default()
            },
            opts: |b| b.initializers([InitializerFn::new(|_| Ok(()))]),
            want: Ok(Outcome::Done),
        },
    ];

    for tc in cases {
        let r = (tc.opts)(Reconciler::<ModernManaged>::builder(Arc::new(tc.client))).build();
        let got = r.reconcile(&Request::default()).await;
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}
