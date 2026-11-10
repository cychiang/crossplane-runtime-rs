//! Port of `pkg/reconciler/managed/reconciler_modern_test.go` `TestModernReconciler`, case for
//! case. Each row carries a `// go:` marker that `hack/port-coverage.sh` counts. Rows are in the
//! Go file's order, split by phase of the reconcile.

use std::sync::Arc;
use std::time::Duration;

use crossplane_core::errors::{Error, Result};
use crossplane_core::meta;
use crossplane_core::resource::Managed;
use crossplane_core::v2::{Condition, ManagementAction, ManagementPolicies};
use crossplane_test::fake::{ModernManaged, new_modern_managed};
use crossplane_test::{self as test, MockClient, MockGetFn, MockObjectFn};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::jiff::Timestamp;

use super::*;
use crate::reconciler::{Outcome, Reconcile, Request};
use crate::resource::FinalizerFns;

pub(super) type Mg = ModernManaged;
pub(super) type Opts = Box<dyn FnOnce(ReconcilerBuilder<Mg>) -> ReconcilerBuilder<Mg>>;

struct Case {
    name: &'static str,
    reason: &'static str,
    client: MockClient,
    opts: Opts,
    want: Result<Outcome>,
    /// Go's `resultCmpOpts`: how far a `RequeueAfter` may be from the wanted one.
    tolerance: Duration,
}

pub(super) fn boom() -> Error {
    Error::new("boom")
}

/// Go `test.NewMockGetFn(nil, fn)` over `asModernManaged(obj, 42)`.
pub(super) fn get_with(fill: impl Fn(&mut Mg) + Send + Sync + 'static) -> MockGetFn {
    test::new_mock_get_fn(move || {
        let mut mg = new_modern_managed(42);
        fill(&mut mg);
        mg
    })
}

/// Go `modernManagedMockGetFn(nil, 42)`.
pub(super) fn get_managed() -> MockGetFn {
    get_with(|_| {})
}

/// Go's `MockStatusUpdate` closures: build `want` from `newModernManaged(42)` and diff it against
/// the object written.
pub(super) fn status_is(reason: &'static str, fill: impl Fn(&mut Mg) + Send + Sync + 'static) -> MockObjectFn {
    test::mock_status_update_fn(move |obj| {
        let mut want = new_modern_managed(42);
        fill(&mut want);
        test::assert_object_eq(&want, obj, reason);
        Ok(())
    })
}

/// Go `.WithObservedGeneration(42)`.
pub(super) fn og(c: Condition) -> Condition {
    c.with_observed_generation(42)
}

pub(super) fn set_deleted(mg: &mut Mg, at: Timestamp) {
    mg.metadata.deletion_timestamp = Some(Time(at));
}

pub(super) fn set_policies(mg: &mut Mg, actions: &[ManagementAction]) {
    mg.set_management_policies(ManagementPolicies(actions.to_vec()));
}

/// Go `WithInitializers()`.
pub(super) fn no_initializers() -> Vec<InitializerFn<Mg>> {
    Vec::new()
}

pub(super) fn resolver_ok() -> ReferenceResolverFn<Mg> {
    ReferenceResolverFn::new(|_| Ok(()))
}

/// An `ExternalConnectorFn` whose client is built by `client`. Go's tables always set a
/// `DisconnectFn` returning nil, which is the Rust default.
pub(super) fn connector(client: impl Fn() -> ExternalClientFns<Mg> + Send + Sync + 'static) -> ExternalConnectorFn<Mg> {
    ExternalConnectorFn::new(move |_| Ok(client()))
}

pub(super) fn observes(observation: ExternalObservation) -> ExternalClientFns<Mg> {
    ExternalClientFns {
        observe: Some(Box::new(move |_| Ok(observation.clone()))),
        ..Default::default()
    }
}

pub(super) fn exists() -> ExternalObservation {
    ExternalObservation {
        resource_exists: true,
        ..Default::default()
    }
}

pub(super) fn up_to_date() -> ExternalObservation {
    ExternalObservation {
        resource_exists: true,
        resource_up_to_date: true,
        ..Default::default()
    }
}

pub(super) fn add_finalizer(result: fn() -> Result<()>) -> FinalizerFns<Mg> {
    FinalizerFns {
        add_finalizer: Some(Box::new(move |_| result())),
        ..Default::default()
    }
}

pub(super) fn remove_finalizer(result: fn() -> Result<()>) -> FinalizerFns<Mg> {
    FinalizerFns {
        remove_finalizer: Some(Box::new(move |_| result())),
        ..Default::default()
    }
}

pub(super) fn publish(result: fn() -> Result<bool>) -> ConnectionPublisherFns<Mg> {
    ConnectionPublisherFns {
        publish_connection: Some(Box::new(move |_, _| result())),
        ..Default::default()
    }
}

pub(super) fn unpublish(result: fn() -> Result<()>) -> ConnectionPublisherFns<Mg> {
    ConnectionPublisherFns {
        unpublish_connection: Some(Box::new(move |_, _| result())),
        ..Default::default()
    }
}

fn case() -> Case {
    Case {
        name: "",
        reason: "",
        client: MockClient::default(),
        opts: Box::new(|b| b),
        want: Ok(Outcome::Done),
        tolerance: Duration::ZERO,
    }
}

const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(60);

fn get_and_delete_cases(now: Timestamp) -> Vec<Case> {
    use ManagementAction::*;
    vec![
        // go: pkg/reconciler/managed TestModernReconciler/GetManagedError
        Case {
            name: "GetManagedError",
            reason: "Any error (except not found) encountered while getting the resource under reconciliation should be returned.",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(boom())),
                ..Default::default()
            },
            want: Err(boom().wrap(ERR_GET_MANAGED)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagedNotFound
        Case {
            name: "ManagedNotFound",
            reason: "Not found errors encountered while getting the resource under reconciliation should be ignored.",
            client: MockClient {
                get: Some(test::new_mock_get_err_fn(Error::not_found())),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/UnpublishConnectionDetailsDeletionPolicyOrphan
        Case {
            name: "UnpublishConnectionDetailsDeletionPolicyOrphan",
            reason: "Errors unpublishing connection details should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| {
                    set_deleted(mg, now);
                    set_policies(mg, &[Observe, Create, Update, LateInitialize]);
                })),
                update_status: Some(status_is(
                    "Errors unpublishing connection details should be reported as a conditioned status.",
                    move |want| {
                        set_deleted(want, now);
                        set_policies(want, &[Observe, Create, Update, LateInitialize]);
                        want.set_conditions([og(Condition::deleting())]);
                        want.set_conditions([og(Condition::reconcile_error(&boom()))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| b.management_policies().connection_publisher(unpublish(|| Err(boom())))),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/RemoveFinalizerErrorDeletionPolicyOrphan
        Case {
            name: "RemoveFinalizerErrorDeletionPolicyOrphan",
            reason: "Errors removing the managed resource finalizer should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| {
                    set_deleted(mg, now);
                    set_policies(mg, &[Observe, Create, Update, LateInitialize]);
                })),
                update_status: Some(status_is(
                    "Errors removing the managed resource finalizer should be reported as a conditioned status.",
                    move |want| {
                        set_deleted(want, now);
                        set_policies(want, &[Observe, Create, Update, LateInitialize]);
                        want.set_conditions([og(Condition::deleting())]);
                        want.set_conditions([og(Condition::reconcile_error(&boom()))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| b.management_policies().finalizer(remove_finalizer(|| Err(boom())))),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/DeleteSuccessfulDeletionPolicyOrphan
        Case {
            name: "DeleteSuccessfulDeletionPolicyOrphan",
            reason: "Successful managed resource deletion with no-delete management policy should not trigger a requeue or status update.",
            client: MockClient {
                get: Some(get_with(move |mg| {
                    set_deleted(mg, now);
                    set_policies(mg, &[Observe]);
                })),
                ..Default::default()
            },
            opts: Box::new(|b| b.management_policies().finalizer(remove_finalizer(|| Ok(())))),
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/InitializeError
        Case {
            name: "InitializeError",
            reason: "Errors initializing the managed resource should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors initializing the managed resource should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_error(&boom()))]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| b.initializers([InitializerFn::new(|_| Err(boom()))])),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExtraFinalizersDelayDelete
        Case {
            name: "ExtraFinalizersDelayDelete",
            reason: "The existence of multiple finalizers should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| {
                    set_deleted(mg, now);
                    mg.metadata.finalizers = Some(vec![FINALIZER_NAME.to_owned(), "finalizer2".to_owned()]);
                })),
                update: Some(test::new_mock_update_fn(None)),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(exists())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalCreatePending
        Case {
            name: "ExternalCreatePending",
            reason: "We should return early if the managed resource appears to be pending creation. We might have leaked a resource and don't want to create another.",
            client: MockClient {
                get: Some(get_with(move |mg| meta::set_external_create_pending(mg, now))),
                update_status: Some(status_is(
                    "We should update our status when we're asked to reconcile a managed resource that is pending creation.",
                    move |want| {
                        meta::set_external_create_pending(want, now);
                        want.set_conditions([
                            og(Condition::creating()),
                            og(Condition::reconcile_error(&Error::new(ERR_CREATE_INCOMPLETE))),
                        ]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| b.initializers([InitializerFn::new(|_| Ok(()))])),
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ResolveReferencesError
        Case {
            name: "ResolveReferencesError",
            reason: "Errors during reference resolution references should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors during reference resolution should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_error(&boom()))]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(ReferenceResolverFn::new(|_| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalConnectError
        Case {
            name: "ExternalConnectError",
            reason: "Errors connecting to the provider should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors connecting to the provider should be reported as a conditioned status.",
                    |want| {
                        want.set_conditions([og(Condition::reconcile_error(&boom().wrap(ERR_RECONCILE_CONNECT)))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .external_connector(ExternalConnectorFn::new(|_| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalDisconnectError
        Case {
            name: "ExternalDisconnectError",
            reason: "Error disconnecting from the provider should not trigger requeue.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "A successful no-op reconcile should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_success())]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| ExternalClientFns {
                        disconnect: Some(Box::new(|| Err(boom()))),
                        ..observes(up_to_date())
                    }))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalObserveError
        Case {
            name: "ExternalObserveError",
            reason: "Errors observing the external resource should trigger a rate limited requeue, even when a poll interval is configured.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors observing the managed resource should be reported as a conditioned status.",
                    |want| {
                        want.set_conditions([og(Condition::reconcile_error(&boom().wrap(ERR_RECONCILE_OBSERVE)))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.poll_interval(Duration::from_secs(600))
                    .initializers(no_initializers())
                    .external_connector(connector(|| ExternalClientFns {
                        observe: Some(Box::new(|_| Err(boom()))),
                        ..Default::default()
                    }))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/CreationGracePeriod
        Case {
            name: "CreationGracePeriod",
            reason: "If our resource appears not to exist during the creation grace period we should return early.",
            client: MockClient {
                get: Some(get_with(|mg| meta::set_external_create_succeeded(mg, test::now()))),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .creation_grace_period(Duration::from_secs(60))
                    .external_connector(connector(|| observes(ExternalObservation::default())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalDeleteError
        Case {
            name: "ExternalDeleteError",
            reason: "Errors deleting the external resource should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| set_deleted(mg, now))),
                update_status: Some(status_is(
                    "An error deleting an external resource should be reported as a conditioned status.",
                    move |want| {
                        set_deleted(want, now);
                        want.set_conditions([og(Condition::reconcile_error(&boom().wrap(ERR_RECONCILE_DELETE)))]);
                        want.set_conditions([og(Condition::deleting())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| ExternalClientFns {
                        delete: Some(Box::new(|_| Err(boom()))),
                        ..observes(exists())
                    }))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalDeleteSuccessful
        Case {
            name: "ExternalDeleteSuccessful",
            reason: "A deleted managed resource with the 'delete' reclaim policy should delete its external resource then requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| set_deleted(mg, now))),
                update_status: Some(status_is(
                    "A deleted external resource should be reported as a conditioned status.",
                    move |want| {
                        set_deleted(want, now);
                        want.set_conditions([og(Condition::reconcile_success())]);
                        want.set_conditions([og(Condition::deleting())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| ExternalClientFns {
                        delete: Some(Box::new(|_| Ok(ExternalDelete::default()))),
                        ..observes(exists())
                    }))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/UnpublishConnectionDetailsDeletionPolicyDeleteError
        Case {
            name: "UnpublishConnectionDetailsDeletionPolicyDeleteError",
            reason: "Errors unpublishing connection details should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| set_deleted(mg, now))),
                update_status: Some(status_is(
                    "Errors unpublishing connection details should be reported as a conditioned status.",
                    move |want| {
                        set_deleted(want, now);
                        want.set_conditions([og(Condition::deleting())]);
                        want.set_conditions([og(Condition::reconcile_error(&boom()))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(ExternalObservation::default())))
                    .connection_publisher(unpublish(|| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/RemoveFinalizerErrorDeletionPolicyDelete
        Case {
            name: "RemoveFinalizerErrorDeletionPolicyDelete",
            reason: "Errors removing the managed resource finalizer should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| set_deleted(mg, now))),
                update_status: Some(status_is(
                    "Errors removing the managed resource finalizer should be reported as a conditioned status.",
                    move |want| {
                        set_deleted(want, now);
                        want.set_conditions([og(Condition::deleting())]);
                        want.set_conditions([og(Condition::reconcile_error(&boom()))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(ExternalObservation::default())))
                    .finalizer(remove_finalizer(|| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/DeleteSuccessfulDeletionPolicyDelete
        Case {
            name: "DeleteSuccessfulDeletionPolicyDelete",
            reason: "Successful managed resource deletion with deletion policy Delete should not trigger a requeue or status update.",
            client: MockClient {
                get: Some(get_with(move |mg| set_deleted(mg, now))),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(ExternalObservation::default())))
                    .finalizer(remove_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/PublishObservationConnectionDetailsError
        Case {
            name: "PublishObservationConnectionDetailsError",
            reason: "Errors publishing connection details after observation should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors publishing connection details after observation should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_error(&boom()))]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(NopConnector)
                    .connection_publisher(publish(|| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/AddFinalizerError
        Case {
            name: "AddFinalizerError",
            reason: "Errors adding a finalizer should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors adding a finalizer should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_error(&boom()))]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(NopConnector)
                    .finalizer(add_finalizer(|| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
    ]
}

/// Go compares results exactly, except where a case passes `resultCmpOpts` (the jitter case).
fn assert_outcome(tc_name: &str, reason: &str, want: &Result<Outcome>, got: &Result<Outcome>, tolerance: Duration) {
    if let (Ok(Outcome::RequeueAfter(w)), Ok(Outcome::RequeueAfter(g))) = (want, got)
        && !tolerance.is_zero()
    {
        assert!(
            w.abs_diff(*g) <= tolerance,
            "\n{tc_name}\nReason: {reason}\n-want RequeueAfter({w:?}) within {tolerance:?}, +got RequeueAfter({g:?})"
        );
        return;
    }
    test::assert_result(tc_name, reason, want, got);
}

#[tokio::test]
async fn modern_reconciler() {
    let now = test::now();
    let cases = get_and_delete_cases(now);
    for tc in cases {
        let r = (tc.opts)(Reconciler::<Mg>::builder(Arc::new(tc.client))).build();
        let got = r.reconcile(&Request::default()).await;
        assert_outcome(tc.name, tc.reason, &tc.want, &got, tc.tolerance);
    }
}
