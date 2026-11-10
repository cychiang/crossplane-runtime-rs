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

use super::policy::ActionSet;
use super::*;
use crate::clock::Clock;
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

pub(super) struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
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

fn creates(result: fn() -> Result<ExternalCreation>) -> ExternalClientFns<Mg> {
    ExternalClientFns {
        create: Some(Box::new(move |_| result())),
        ..observes(ExternalObservation::default())
    }
}

fn updates(result: fn() -> Result<ExternalUpdate>) -> ExternalClientFns<Mg> {
    ExternalClientFns {
        update: Some(Box::new(move |_| result())),
        ..observes(exists())
    }
}

fn critical_annotations(result: fn() -> Result<()>) -> CriticalAnnotationUpdateFn<Mg> {
    CriticalAnnotationUpdateFn::new(move |_| result())
}

/// Go's `TypedExternalConnectorFn[*fake.ModernManaged]`. Every Rust connector is typed, so this
/// is one written against the traits directly instead of through the closure adapters.
struct TypedConnector;
struct TypedClient;

impl ExternalConnector<Mg> for TypedConnector {
    type Client = TypedClient;
    async fn connect(&self, _: &Mg) -> Result<TypedClient> {
        Ok(TypedClient)
    }
}

impl ExternalClient<Mg> for TypedClient {
    async fn observe(&self, _: &mut Mg) -> Result<ExternalObservation> {
        Ok(exists())
    }
    async fn create(&self, _: &mut Mg) -> Result<ExternalCreation> {
        panic!("unexpected call to TypedClient::create")
    }
    async fn update(&self, _: &mut Mg) -> Result<ExternalUpdate> {
        Ok(ExternalUpdate::default())
    }
    async fn delete(&self, _: &mut Mg) -> Result<ExternalDelete> {
        panic!("unexpected call to TypedClient::delete")
    }
}

/// The create annotations are stamped from the injected clock, fixed at `now`, where Go's
/// assertions call `time.Now()` again and rely on second precision.
fn create_and_update_cases(now: Timestamp) -> Vec<Case> {
    vec![
        // go: pkg/reconciler/managed TestModernReconciler/UpdateCreatePendingError
        Case {
            name: "UpdateCreatePendingError",
            reason: "Errors while updating our external-create-pending annotation should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update: Some(test::new_mock_update_fn(Some(boom()))),
                update_status: Some(status_is(
                    "Errors while creating an external resource should be reported as a conditioned status.",
                    move |want| {
                        meta::set_external_create_pending(want, now);
                        want.set_conditions([
                            og(Condition::creating()),
                            og(Condition::reconcile_error(&boom().wrap(ERR_UPDATE_MANAGED))),
                        ]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| creates(|| Err(boom()))))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/CreateExternalError
        Case {
            name: "CreateExternalError",
            reason: "Errors while creating an external resource should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update: Some(test::new_mock_update_fn(None)),
                update_status: Some(status_is(
                    "Errors while creating an external resource should be reported as a conditioned status.",
                    move |want| {
                        meta::set_external_create_pending(want, now);
                        meta::set_external_create_failed(want, now);
                        want.set_conditions([og(Condition::reconcile_error(&boom().wrap(ERR_RECONCILE_CREATE)))]);
                        want.set_conditions([og(Condition::creating())]);
                    },
                )),
                ..Default::default()
            },
            // The critical annotation update fails too, which only emits an event.
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| creates(|| Err(boom()))))
                    .critical_annotation_updater(critical_annotations(|| Err(boom())))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/UpdateCriticalAnnotationsError
        Case {
            name: "UpdateCriticalAnnotationsError",
            reason: "Errors updating critical annotations after creation should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update: Some(test::new_mock_update_fn(None)),
                update_status: Some(status_is(
                    "Errors updating critical annotations after creation should be reported as a conditioned status.",
                    move |want| {
                        meta::set_external_create_pending(want, now);
                        meta::set_external_create_succeeded(want, now);
                        want.set_conditions([og(Condition::reconcile_error(
                            &boom().wrap(ERR_UPDATE_MANAGED_ANNOTATIONS),
                        ))]);
                        want.set_conditions([og(Condition::creating())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| creates(|| Ok(ExternalCreation::default()))))
                    .finalizer(add_finalizer(|| Ok(())))
                    .critical_annotation_updater(critical_annotations(|| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/PublishCreationConnectionDetailsError
        Case {
            name: "PublishCreationConnectionDetailsError",
            reason: "Errors publishing connection details after creation should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update: Some(test::new_mock_update_fn(None)),
                update_status: Some(status_is(
                    "Errors publishing connection details after creation should be reported as a conditioned status.",
                    move |want| {
                        meta::set_external_create_pending(want, now);
                        meta::set_external_create_succeeded(want, now);
                        want.set_conditions([og(Condition::reconcile_error(&boom()))]);
                        want.set_conditions([og(Condition::creating())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| {
                        creates(|| {
                            Ok(ExternalCreation {
                                connection_details: [("create".to_owned(), Vec::new())].into(),
                                ..Default::default()
                            })
                        })
                    }))
                    .critical_annotation_updater(critical_annotations(|| Ok(())))
                    // Called after observe and after create; fails only for the creation details.
                    .connection_publisher(ConnectionPublisherFns {
                        publish_connection: Some(Box::new(|_, cd| {
                            if cd.contains_key("create") {
                                return Err(boom());
                            }
                            Ok(true)
                        })),
                        ..Default::default()
                    })
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/CreateSuccessful
        Case {
            name: "CreateSuccessful",
            reason: "Successful managed resource creation should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update: Some(test::new_mock_update_fn(None)),
                update_status: Some(status_is(
                    "Successful managed resource creation should be reported as a conditioned status.",
                    move |want| {
                        meta::set_external_create_pending(want, now);
                        meta::set_external_create_succeeded(want, now);
                        want.set_conditions([og(Condition::reconcile_success())]);
                        want.set_conditions([og(Condition::creating())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(NopConnector)
                    .critical_annotation_updater(critical_annotations(|| Ok(())))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/CreateSuccessfulAfterExternalCreatePendingAndDeterministicName
        Case {
            name: "CreateSuccessfulAfterExternalCreatePendingAndDeterministicName",
            reason: "Successful managed resource creation which was previously pending and has a deterministic external name should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(move |mg| meta::set_external_create_pending(mg, now))),
                update: Some(test::new_mock_update_fn(None)),
                update_status: Some(status_is(
                    "Successful managed resource creation should be reported as a conditioned status.",
                    move |want| {
                        meta::set_external_create_pending(want, now);
                        meta::set_external_create_succeeded(want, now);
                        want.set_conditions([og(Condition::reconcile_success())]);
                        want.set_conditions([og(Condition::creating())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(NopConnector)
                    .critical_annotation_updater(critical_annotations(|| Ok(())))
                    .finalizer(add_finalizer(|| Ok(())))
                    .deterministic_external_name(true)
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/LateInitializeUpdateError
        Case {
            name: "LateInitializeUpdateError",
            reason: "Errors updating a managed resource to persist late initialized fields should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update: Some(test::new_mock_update_fn(Some(boom()))),
                update_status: Some(status_is(
                    "Errors updating a managed resource should be reported as a conditioned status.",
                    |want| {
                        want.set_conditions([og(Condition::reconcile_error(&boom().wrap(ERR_UPDATE_MANAGED)))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| {
                        observes(ExternalObservation {
                            resource_late_initialized: true,
                            ..up_to_date()
                        })
                    }))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalResourceUpToDate
        Case {
            name: "ExternalResourceUpToDate",
            reason: "When the external resource exists and is up to date a requeue should be triggered after a long wait.",
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
                    .external_connector(connector(|| observes(up_to_date())))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalResourceUpToDateWithJitter
        Case {
            name: "ExternalResourceUpToDateWithJitter",
            reason: "When the external resource exists and is up to date a requeue should be triggered after a long wait with jitter added.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(test::new_mock_sub_resource_update_fn(None)),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(up_to_date())))
                    .finalizer(add_finalizer(|| Ok(())))
                    .poll_jitter_hook(Duration::from_secs(1))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            tolerance: Duration::from_secs(1),
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalResourceUpToDateWithPollIntervalHook
        Case {
            name: "ExternalResourceUpToDateWithPollIntervalHook",
            reason: "When the external resource exists and is up to date a requeue should be triggered after a long wait processed by the interval hook.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(test::new_mock_sub_resource_update_fn(None)),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(up_to_date())))
                    .finalizer(add_finalizer(|| Ok(())))
                    .poll_interval_hook(|_, poll| 2 * poll)
            }),
            want: Ok(Outcome::RequeueAfter(2 * DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ExternalResourceUpToDateWithMultiplePollIntervalHooks
        Case {
            name: "ExternalResourceUpToDateWithMultiplePollIntervalHooks",
            reason: "When the external resource exists and is up to date a requeue should be triggered after a long wait processed by the latest interval hook.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(test::new_mock_sub_resource_update_fn(None)),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(up_to_date())))
                    .finalizer(add_finalizer(|| Ok(())))
                    .poll_jitter_hook(Duration::from_secs(1))
                    .poll_interval_hook(|_, poll| 2 * poll)
                    .poll_interval_hook(|_, poll| 3 * poll)
            }),
            want: Ok(Outcome::RequeueAfter(3 * DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/UpdateExternalError
        Case {
            name: "UpdateExternalError",
            reason: "Errors while updating an external resource should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors while updating an external resource should be reported as a conditioned status.",
                    |want| {
                        want.set_conditions([og(Condition::reconcile_error(&boom().wrap(ERR_RECONCILE_UPDATE)))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| updates(|| Err(boom()))))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/PublishUpdateConnectionDetailsError
        Case {
            name: "PublishUpdateConnectionDetailsError",
            reason: "Errors publishing connection details after an update should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "Errors publishing connection details after an update should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_error(&boom()))]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| {
                        updates(|| {
                            Ok(ExternalUpdate {
                                connection_details: [("update".to_owned(), Vec::new())].into(),
                                ..Default::default()
                            })
                        })
                    }))
                    // Called after observe and after update; fails only for the update details.
                    .connection_publisher(ConnectionPublisherFns {
                        publish_connection: Some(Box::new(|_, cd| {
                            if cd.contains_key("update") {
                                return Err(boom());
                            }
                            Ok(false)
                        })),
                        ..Default::default()
                    })
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/UpdateSuccessful
        Case {
            name: "UpdateSuccessful",
            reason: "A successful managed resource update should trigger a requeue after a long wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "A successful managed resource update should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_success())]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| updates(|| Ok(ExternalUpdate::default()))))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/TypedReconcilerUpdateSuccessful
        Case {
            name: "TypedReconcilerUpdateSuccessful",
            reason: "A successful managed resource update should trigger a requeue after a long wait.",
            client: MockClient {
                get: Some(get_managed()),
                update_status: Some(status_is(
                    "A successful managed resource update should be reported as a conditioned status.",
                    |want| want.set_conditions([og(Condition::reconcile_success())]),
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(TypedConnector)
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
    ]
}

fn set_paused(mg: &mut Mg, value: &str) {
    mg.metadata.annotations = Some([(meta::ANNOTATION_KEY_RECONCILIATION_PAUSED.to_owned(), value.to_owned())].into());
}

/// Go `fmt.Errorf(errFmtManagementPolicyNonDefault, policies)`.
fn policy_non_default(policies: &str) -> Error {
    Error::new(format!(
        "`spec.managementPolicies` is set to a non-default value but the feature is not enabled: {policies}"
    ))
}

/// Go `fmt.Errorf(errFmtManagementPolicyNotSupported, policies)`.
fn policy_not_supported(policies: &str) -> Error {
    Error::new(format!(
        "`spec.managementPolicies` is set to a value({policies}) which is not supported. Check docs for supported policies"
    ))
}

fn pause_and_policy_cases(now: Timestamp) -> Vec<Case> {
    use ManagementAction::*;
    vec![
        // go: pkg/reconciler/managed TestModernReconciler/ReconciliationPausedSuccessful
        Case {
            name: "ReconciliationPausedSuccessful",
            reason: r#"If a managed resource has the pause annotation with value "true", there should be no further requeue requests."#,
            client: MockClient {
                get: Some(get_with(|mg| set_paused(mg, "true"))),
                update_status: Some(status_is(
                    r#"If managed resource has the pause annotation with value "true", it should acquire "Synced" status condition with the status "False" and the reason "ReconcilePaused"."#,
                    |want| {
                        set_paused(want, "true");
                        want.set_conditions([og(Condition::reconcile_paused())]);
                    },
                )),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicyReconciliationPausedSuccessful
        Case {
            name: "ManagementPolicyReconciliationPausedSuccessful",
            reason: r#"If a managed resource has the pause annotation with value "true", there should be no further requeue requests."#,
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[]))),
                update_status: Some(status_is(
                    r#"If managed resource has the pause annotation with value "true", it should acquire "Synced" status condition with the status "False" and the reason "ReconcilePaused"."#,
                    |want| {
                        set_policies(want, &[]);
                        want.set_conditions([og(Condition::reconcile_paused())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .initializers(no_initializers())
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ReconciliationResumes
        Case {
            name: "ReconciliationResumes",
            reason: r#"If a managed resource has the pause annotation with some value other than "true" and the Synced=False/ReconcilePaused status condition, reconciliation should resume with requeueing."#,
            client: MockClient {
                get: Some(get_with(|mg| {
                    set_paused(mg, "false");
                    mg.set_conditions([Condition::reconcile_paused()]);
                })),
                update_status: Some(status_is(
                    "Managed resource should acquire Synced=False/ReconcileSuccess status condition after a resume.",
                    |want| {
                        set_paused(want, "false");
                        want.set_conditions([og(Condition::reconcile_success())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| observes(up_to_date())))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ReconciliationPausedError
        Case {
            name: "ReconciliationPausedError",
            reason: r#"If a managed resource has the pause annotation with value "true" and the status update due to reconciliation being paused fails, error should be reported causing an exponentially backed-off requeue."#,
            client: MockClient {
                get: Some(get_with(|mg| set_paused(mg, "true"))),
                update_status: Some(test::new_mock_sub_resource_update_fn(Some(boom()))),
                ..Default::default()
            },
            want: Err(boom().wrap(ERR_UPDATE_MANAGED_STATUS)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPoliciesUsedButNotEnabled
        Case {
            name: "ManagementPoliciesUsedButNotEnabled",
            reason: "If management policies tried to be used without enabling the feature, we should throw an error.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[Create]))),
                update_status: Some(status_is(
                    "If managed resource has a non default management policy but feature not enabled, it should return a proper error.",
                    |want| {
                        set_policies(want, &[Create]);
                        want.set_conditions([og(Condition::reconcile_error(&policy_non_default("[Create]")))]);
                    },
                )),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicyNotSupported
        Case {
            name: "ManagementPolicyNotSupported",
            reason: "If an unsupported management policy is used, we should throw an error.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[Create]))),
                update_status: Some(status_is(
                    "If managed resource has non supported management policy, it should return a proper error.",
                    |want| {
                        set_policies(want, &[Create]);
                        want.set_conditions([og(Condition::reconcile_error(&policy_not_supported("[Create]")))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| b.management_policies()),
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/CustomManagementPolicyNotSupported
        Case {
            name: "CustomManagementPolicyNotSupported",
            reason: "If a custom unsupported management policy is used, we should throw an error.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[All]))),
                update_status: Some(status_is(
                    "If managed resource has non supported management policy, it should return a proper error.",
                    |want| {
                        set_policies(want, &[All]);
                        want.set_conditions([og(Condition::reconcile_error(&policy_not_supported("[*]")))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .supported_management_policies(vec![ActionSet::of(&[Observe])])
            }),
            want: Ok(Outcome::Done),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ObserveOnlyResourceDoesNotExist
        Case {
            name: "ObserveOnlyResourceDoesNotExist",
            reason: "With only Observe management action, observing a resource that does not exist should be reported as a conditioned status error and requeued after the poll interval, not via the error rate limiter.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[Observe]))),
                update_status: Some(status_is(
                    "Resource does not exist should be reported as a conditioned status when ObserveOnly.",
                    |want| {
                        set_policies(want, &[Observe]);
                        want.set_conditions([og(Condition::reconcile_error(
                            &Error::new(ERR_EXTERNAL_RESOURCE_NOT_EXIST).wrap(ERR_RECONCILE_OBSERVE),
                        ))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.poll_interval(Duration::from_secs(600))
                    .initializers(no_initializers())
                    .management_policies()
                    .external_connector(connector(|| observes(ExternalObservation::default())))
            }),
            want: Ok(Outcome::RequeueAfter(Duration::from_secs(600))),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ObserveOnlyPublishConnectionDetailsError
        Case {
            name: "ObserveOnlyPublishConnectionDetailsError",
            reason: "With Observe, errors publishing connection details after observation should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[Observe]))),
                update_status: Some(status_is(
                    "Errors publishing connection details after observation should be reported as a conditioned status.",
                    |want| {
                        set_policies(want, &[Observe]);
                        want.set_conditions([og(Condition::reconcile_error(&boom()))]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .management_policies()
                    .external_connector(connector(|| observes(exists())))
                    .connection_publisher(publish(|| Err(boom())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ObserveOnlySuccessfulObserve
        Case {
            name: "ObserveOnlySuccessfulObserve",
            reason: "With Observe, a successful managed resource observe should trigger a requeue after a long wait.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[Observe]))),
                update_status: Some(status_is(
                    "With ObserveOnly, a successful managed resource observation should be reported as a conditioned status.",
                    |want| {
                        set_policies(want, &[Observe]);
                        want.set_conditions([og(Condition::reconcile_success())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.initializers(no_initializers())
                    .management_policies()
                    .external_connector(connector(|| observes(exists())))
                    .connection_publisher(publish(|| Ok(false)))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ObserveUpdateResourceDoesNotExist
        Case {
            name: "ObserveUpdateResourceDoesNotExist",
            reason: "With only Observe and Update management actions, observing a resource that does not exist should not call the Update external functions and should be reported as a conditioned status and requeued after the poll interval, not via the error rate limiter.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[Observe, Update]))),
                update_status: Some(status_is(
                    "Resource does not exist should be reported as a conditioned status when only Observe and Update actions are allowed.",
                    |want| {
                        set_policies(want, &[Observe, Update]);
                        want.set_conditions([og(Condition::reconcile_success())]);
                    },
                )),
                update: Some(test::new_mock_update_fn(None)),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.poll_interval(Duration::from_secs(600))
                    .initializers(no_initializers())
                    .management_policies()
                    .external_connector(connector(|| ExternalClientFns {
                        // Update should not be called in this scenario.
                        update: Some(Box::new(|_| Err(boom()))),
                        ..observes(ExternalObservation::default())
                    }))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(Duration::from_secs(600))),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicyAllCreateSuccessful
        Case {
            name: "ManagementPolicyAllCreateSuccessful",
            reason: "Successful managed resource creation using management policy all should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[All]))),
                update: Some(test::new_mock_update_fn(None)),
                update_status: Some(status_is(
                    "Successful managed resource creation should be reported as a conditioned status.",
                    move |want| {
                        set_policies(want, &[All]);
                        meta::set_external_create_pending(want, now);
                        meta::set_external_create_succeeded(want, now);
                        want.set_conditions([og(Condition::reconcile_success())]);
                        want.set_conditions([og(Condition::creating())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(NopConnector)
                    .critical_annotation_updater(critical_annotations(|| Ok(())))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // The Go fixture sets `*`, not Create, despite the case's name.
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicyCreateCreateSuccessful
        Case {
            name: "ManagementPolicyCreateCreateSuccessful",
            reason: "Successful managed resource creation using management policy Create should trigger a requeue after a short wait.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[All]))),
                update: Some(test::new_mock_update_fn(None)),
                update_status: Some(status_is(
                    "Successful managed resource creation should be reported as a conditioned status.",
                    move |want| {
                        set_policies(want, &[All]);
                        meta::set_external_create_pending(want, now);
                        meta::set_external_create_succeeded(want, now);
                        want.set_conditions([og(Condition::reconcile_success())]);
                        want.set_conditions([og(Condition::creating())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(NopConnector)
                    .critical_annotation_updater(critical_annotations(|| Ok(())))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::Requeue),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicyImmutable
        Case {
            name: "ManagementPolicyImmutable",
            reason: "Successful reconciliation skipping update should trigger a requeue after a long wait.",
            client: MockClient {
                get: Some(get_with(|mg| {
                    set_policies(mg, &[Observe, LateInitialize, Create, Delete])
                })),
                update: Some(test::new_mock_update_fn(Some(boom()))),
                update_status: Some(status_is(
                    "Managed resource should acquire Synced=False/ReconcileSuccess status condition.",
                    |want| {
                        set_policies(want, &[Observe, LateInitialize, Create, Delete]);
                        want.set_conditions([og(Condition::reconcile_success())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| updates(|| Err(boom()))))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicyAllUpdateSuccessful
        Case {
            name: "ManagementPolicyAllUpdateSuccessful",
            reason: "A successful managed resource update using management policies should trigger a requeue after a long wait.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[All]))),
                update_status: Some(status_is(
                    "A successful managed resource update should be reported as a conditioned status.",
                    |want| {
                        set_policies(want, &[All]);
                        want.set_conditions([og(Condition::reconcile_success())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| updates(|| Ok(ExternalUpdate::default()))))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // The Go fixture sets `*`, not Update, despite the case's name.
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicyUpdateUpdateSuccessful
        Case {
            name: "ManagementPolicyUpdateUpdateSuccessful",
            reason: "A successful managed resource update using management policies should trigger a requeue after a long wait.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[All]))),
                update_status: Some(status_is(
                    "A successful managed resource update should be reported as a conditioned status.",
                    |want| {
                        set_policies(want, &[All]);
                        want.set_conditions([og(Condition::reconcile_success())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| updates(|| Ok(ExternalUpdate::default()))))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
            ..case()
        },
        // go: pkg/reconciler/managed TestModernReconciler/ManagementPolicySkipLateInitialize
        Case {
            name: "ManagementPolicySkipLateInitialize",
            reason: "Should skip updating a managed resource to persist late initialized fields and should trigger a requeue after a long wait.",
            client: MockClient {
                get: Some(get_with(|mg| set_policies(mg, &[Observe, Update, Create, Delete]))),
                update: Some(test::new_mock_update_fn(Some(boom()))),
                update_status: Some(status_is(
                    "Errors updating a managed resource should be reported as a conditioned status.",
                    |want| {
                        set_policies(want, &[Observe, Update, Create, Delete]);
                        want.set_conditions([og(Condition::reconcile_success())]);
                    },
                )),
                ..Default::default()
            },
            opts: Box::new(|b| {
                b.management_policies()
                    .initializers(no_initializers())
                    .reference_resolver(resolver_ok())
                    .external_connector(connector(|| {
                        observes(ExternalObservation {
                            resource_late_initialized: true,
                            ..up_to_date()
                        })
                    }))
                    .finalizer(add_finalizer(|| Ok(())))
            }),
            want: Ok(Outcome::RequeueAfter(DEFAULT_POLL_INTERVAL)),
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
    let cases = get_and_delete_cases(now)
        .into_iter()
        .chain(create_and_update_cases(now))
        .chain(pause_and_policy_cases(now));
    for tc in cases {
        let builder = Reconciler::<Mg>::builder(Arc::new(tc.client)).clock(Arc::new(FixedClock(now)));
        let r = (tc.opts)(builder).build();
        let got = r.reconcile(&Request::default()).await;
        assert_outcome(tc.name, tc.reason, &tc.want, &got, tc.tolerance);
    }
}
