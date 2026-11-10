//! Port of `TestEffectivePollInterval`, `TestReconcilePollIntervalAnnotation` and
//! `TestReconcileRequestAnnotation` from `reconciler_modern_test.go`.

use std::sync::Arc;
use std::time::Duration;

use crossplane_core::meta;
use crossplane_core::resource::Managed;
use crossplane_test::fake::as_modern_managed;
use crossplane_test::{self as test, MockClient};

use super::reconciler_modern_test::{
    Mg, add_finalizer, connector, get_with, no_initializers, observes, resolver_ok, up_to_date,
};
use super::*;
use crate::reconciler::{Outcome, Reconcile, Request};

const SECOND: Duration = Duration::from_secs(1);
const MINUTE: Duration = Duration::from_secs(60);
const HOUR: Duration = Duration::from_secs(3600);

fn annotate(mg: &mut Mg, key: &str, value: &str) {
    mg.metadata.annotations = Some([(key.to_owned(), value.to_owned())].into());
}

/// The reconciler every case here builds: an existing, up to date external resource.
fn up_to_date_reconciler(client: MockClient) -> ReconcilerBuilder<Mg> {
    Reconciler::<Mg>::builder(Arc::new(client))
        .initializers(no_initializers())
        .reference_resolver(resolver_ok())
        .external_connector(connector(|| observes(up_to_date())))
        .finalizer(add_finalizer(|| Ok(())))
}

struct Case {
    name: &'static str,
    reason: &'static str,
    poll_interval: Duration,
    min_poll_interval: Duration,
    annotation: &'static str,
    want: Duration,
}

#[test]
fn effective_poll_interval() {
    let cases = [
        // go: pkg/reconciler/managed TestEffectivePollInterval/NoAnnotationReturnsDefault
        Case {
            name: "NoAnnotationReturnsDefault",
            reason: "When no annotation is set, the default poll interval is returned.",
            poll_interval: 10 * MINUTE,
            min_poll_interval: SECOND,
            annotation: "",
            want: 10 * MINUTE,
        },
        // go: pkg/reconciler/managed TestEffectivePollInterval/ValidAnnotationAboveMinimumReturnsAnnotation
        Case {
            name: "ValidAnnotationAboveMinimumReturnsAnnotation",
            reason: "When the annotation is at or above the minimum, it overrides the default.",
            poll_interval: 10 * MINUTE,
            min_poll_interval: SECOND,
            annotation: "24h",
            want: 24 * HOUR,
        },
        // go: pkg/reconciler/managed TestEffectivePollInterval/ValidAnnotationAtMinimumReturnsAnnotation
        Case {
            name: "ValidAnnotationAtMinimumReturnsAnnotation",
            reason: "When the annotation equals the minimum exactly, it is returned as-is.",
            poll_interval: 10 * MINUTE,
            min_poll_interval: 30 * SECOND,
            annotation: "30s",
            want: 30 * SECOND,
        },
        // go: pkg/reconciler/managed TestEffectivePollInterval/ValidAnnotationBelowMinimumReturnsMinimum
        Case {
            name: "ValidAnnotationBelowMinimumReturnsMinimum",
            reason: "When the annotation is below the minimum, the minimum is returned.",
            poll_interval: 10 * MINUTE,
            min_poll_interval: 30 * SECOND,
            annotation: "1s",
            want: 30 * SECOND,
        },
        // go: pkg/reconciler/managed TestEffectivePollInterval/InvalidAnnotationReturnsDefault
        Case {
            name: "InvalidAnnotationReturnsDefault",
            reason: "When the annotation cannot be parsed, the default poll interval is returned.",
            poll_interval: 5 * MINUTE,
            min_poll_interval: SECOND,
            annotation: "not-a-duration",
            want: 5 * MINUTE,
        },
    ];
    for tc in cases {
        let r = Reconciler::<Mg>::builder(Arc::new(MockClient::default()))
            .poll_interval(tc.poll_interval)
            .min_poll_interval(tc.min_poll_interval)
            .build();
        let mut mg = Mg::default();
        if !tc.annotation.is_empty() {
            annotate(&mut mg, meta::ANNOTATION_KEY_POLL_INTERVAL, tc.annotation);
        }
        assert_eq!(tc.want, r.effective_poll_interval(&mg), "\n{}\n{}", tc.name, tc.reason);
    }
}

#[tokio::test]
async fn reconcile_poll_interval_annotation() {
    let cases = [
        // go: pkg/reconciler/managed TestReconcilePollIntervalAnnotation/AnnotationOverridesPollInterval
        Case {
            name: "AnnotationOverridesPollInterval",
            reason: "When a valid poll interval annotation is set, it should override the controller-level poll interval.",
            poll_interval: MINUTE,
            min_poll_interval: SECOND,
            annotation: "24h",
            want: 24 * HOUR,
        },
        // go: pkg/reconciler/managed TestReconcilePollIntervalAnnotation/InvalidAnnotationFallsBack
        Case {
            name: "InvalidAnnotationFallsBack",
            reason: "When an invalid poll interval annotation is set, the controller-level poll interval should be used.",
            poll_interval: 5 * MINUTE,
            min_poll_interval: SECOND,
            annotation: "not-a-duration",
            want: 5 * MINUTE,
        },
        // go: pkg/reconciler/managed TestReconcilePollIntervalAnnotation/AnnotationBelowMinimumClampsToMin
        Case {
            name: "AnnotationBelowMinimumClampsToMin",
            reason: "When a poll interval annotation is below the configured minimum, the minimum should be used.",
            poll_interval: 10 * MINUTE,
            min_poll_interval: 30 * SECOND,
            annotation: "1s",
            want: 30 * SECOND,
        },
        // go: pkg/reconciler/managed TestReconcilePollIntervalAnnotation/NoAnnotationUsesDefault
        Case {
            name: "NoAnnotationUsesDefault",
            reason: "When no poll interval annotation is set, the controller-level poll interval should be used.",
            poll_interval: 10 * MINUTE,
            min_poll_interval: SECOND,
            annotation: "",
            want: 10 * MINUTE,
        },
    ];
    for tc in cases {
        let client = MockClient {
            get: Some(get_with(move |mg| {
                if !tc.annotation.is_empty() {
                    annotate(mg, meta::ANNOTATION_KEY_POLL_INTERVAL, tc.annotation);
                }
            })),
            update_status: Some(test::new_mock_sub_resource_update_fn(None)),
            ..Default::default()
        };
        let r = up_to_date_reconciler(client)
            .poll_interval(tc.poll_interval)
            .min_poll_interval(tc.min_poll_interval)
            .build();
        // Go allows one second either way; no jitter hook is set, so the interval is exact.
        let got = r.reconcile(&Request::default()).await;
        test::assert_result(tc.name, tc.reason, &Ok(Outcome::RequeueAfter(tc.want)), &got);
    }
}

#[tokio::test]
async fn reconcile_request_annotation() {
    struct Case {
        name: &'static str,
        reason: &'static str,
        annotation: &'static str,
        handled: &'static str,
        want_token: &'static str,
    }
    let cases = [
        // go: pkg/reconciler/managed TestReconcileRequestAnnotation/NewReconcileRequestRecordsToken
        Case {
            name: "NewReconcileRequestRecordsToken",
            reason: "A new reconcile-requested-at token should be recorded in status.lastHandledReconcileAt.",
            annotation: "1705312200",
            handled: "",
            want_token: "1705312200",
        },
        // go: pkg/reconciler/managed TestReconcileRequestAnnotation/AlreadyHandledReconcileRequestIsNoOp
        Case {
            name: "AlreadyHandledReconcileRequestIsNoOp",
            reason: "When the token matches lastHandledReconcileAt, status should remain unchanged.",
            annotation: "already-handled",
            handled: "already-handled",
            want_token: "already-handled",
        },
    ];
    for tc in cases {
        let client = MockClient {
            get: Some(get_with(move |mg| {
                annotate(mg, meta::ANNOTATION_KEY_RECONCILE_REQUESTED_AT, tc.annotation);
                if !tc.handled.is_empty() {
                    mg.set_last_handled_reconcile_at(tc.handled);
                }
            })),
            update_status: Some(test::mock_status_update_fn(move |obj| {
                let got = as_modern_managed(obj, 42).last_handled_reconcile_at().to_owned();
                assert_eq!(tc.want_token, got, "\n{}\nstatus.lastHandledReconcileAt", tc.reason);
                Ok(())
            })),
            ..Default::default()
        };
        let r = up_to_date_reconciler(client).build();
        let got = r.reconcile(&Request::default()).await;
        test::assert_result(
            tc.name,
            tc.reason,
            &Ok(Outcome::RequeueAfter(Duration::from_secs(60))),
            &got,
        );
    }
}
