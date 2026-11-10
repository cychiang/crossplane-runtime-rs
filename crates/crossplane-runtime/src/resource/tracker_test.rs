use std::sync::Arc;

use crossplane_core::errors::Error;
use crossplane_core::resource::{Managed, ProviderConfigUsage as _};
use crossplane_core::v2::ProviderConfigReference;
use crossplane_test::fake::{ModernManaged, ProviderConfigUsage};
use crossplane_test::{self as test, MockClient};

use super::*;
use crate::resource::{ApplyFn, is_missing_reference};

const NAME: &str = "provisional";

fn pc_ref() -> ProviderConfigReference {
    ProviderConfigReference {
        name: NAME.into(),
        kind: "ProviderConfig".into(),
    }
}

fn managed() -> ModernManaged {
    let mut mg = ModernManaged::default();
    mg.set_provider_config_reference(Some(pc_ref()));
    mg
}

fn existing_usage() -> ProviderConfigUsage {
    let mut u = ProviderConfigUsage::default();
    u.set_provider_config_reference(pc_ref());
    u
}

#[tokio::test]
async fn track_modern() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        apply: ApplyFn,
        mg: ModernManaged,
        want: Result<(), Error>,
    }
    let boom = || Error::new("boom");
    let cases = vec![
        // go: pkg/resource TestTrackModern/MissingRef
        Case {
            name: "MissingRef",
            reason: "An error that satisfies IsMissingReference should be returned if the managed resource has no provider config reference",
            apply: ApplyFn(Box::new(|_, _| unreachable!("no apply without a reference"))),
            mg: ModernManaged::default(),
            want: Err(Error::with_kind(ErrorKind::MissingReference, ERR_MISSING_PC_REF)),
        },
        // go: pkg/resource TestTrackModern/NopUpdate
        Case {
            name: "NopUpdate",
            reason: "No error should be returned if the apply fails because it would be a no-op",
            apply: ApplyFn(Box::new(move |desired, opts| {
                // The first option passes because the current usage has no controller. The
                // second refuses because it has the same ProviderConfig reference.
                let mut current = existing_usage();
                for opt in opts {
                    opt.run(&mut current, desired)?;
                }
                Err(boom())
            })),
            mg: managed(),
            want: Ok(()),
        },
        // go: pkg/resource TestTrackModern/ApplyError
        Case {
            name: "ApplyError",
            reason: "Errors applying the ProviderConfigUsage should be returned",
            apply: ApplyFn(Box::new(move |_, _| Err(boom()))),
            mg: managed(),
            want: Err(boom().wrap(ERR_APPLY_PCU)),
        },
    ];
    for tc in cases {
        let tracker = ProviderConfigUsageTracker::<ProviderConfigUsage>::with_applicator(Arc::new(tc.apply));
        let got = tracker.track(&tc.mg).await;
        test::assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

#[tokio::test]
async fn missing_reference_errors_satisfy_the_predicate() {
    let tracker = ProviderConfigUsageTracker::<ProviderConfigUsage>::new(Arc::new(MockClient::default()));
    let err = tracker.track(&ModernManaged::default()).await.unwrap_err();
    assert!(is_missing_reference(&err));

    let mut mg = managed();
    mg.set_provider_config_reference(Some(ProviderConfigReference {
        name: NAME.into(),
        kind: String::new(),
    }));
    let err = tracker.track(&mg).await.unwrap_err();
    assert_eq!(err, Error::with_kind(ErrorKind::MissingReference, ERR_MISSING_PC_KIND));
}

#[tokio::test]
async fn an_unchanged_reference_costs_one_get_and_no_write() {
    let client = MockClient {
        get: Some(test::new_mock_get_fn(existing_usage)),
        ..Default::default()
    };
    let tracker = ProviderConfigUsageTracker::<ProviderConfigUsage>::new(Arc::new(client));
    tracker.track(&managed()).await.unwrap();
}

#[tokio::test]
async fn a_new_usage_is_created_with_go_s_shape() {
    let mut mg = managed();
    mg.metadata.name = Some("mr".into());
    mg.metadata.namespace = Some("ns".into());
    mg.metadata.uid = Some("uid-1".into());
    let client = MockClient {
        get: Some(test::new_mock_get_err_fn(Error::not_found())),
        create: Some(Box::new(|o| {
            let want = serde_json::json!({
                "apiVersion": "fake.crossplane.io/v1",
                "kind": "ProviderConfigUsage",
                "metadata": {
                    "name": "uid-1",
                    "namespace": "ns",
                    "labels": {
                        "crossplane.io/provider-config": NAME,
                        "crossplane.io/provider-config-kind": "ProviderConfig",
                    },
                    "ownerReferences": [{
                        "apiVersion": "fake.crossplane.io/v1", "kind": "ModernManaged", "name": "mr",
                        "uid": "uid-1", "controller": true, "blockOwnerDeletion": true,
                    }],
                },
                "providerConfigRef": {"kind": "ProviderConfig", "name": NAME},
                "resourceRef": {"apiVersion": "fake.crossplane.io/v1", "kind": "ModernManaged", "name": "mr"},
            });
            assert_eq!(want, o.to_json().unwrap());
            Ok(())
        })),
        ..Default::default()
    };
    ProviderConfigUsageTracker::<ProviderConfigUsage>::new(Arc::new(client))
        .track(&mg)
        .await
        .unwrap();
}
