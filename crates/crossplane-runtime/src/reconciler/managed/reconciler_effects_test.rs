//! Tests with no Go counterpart. Go's tables assert the result and the status written; these
//! assert what the parity audit compares between the two stacks and no Go test pins: the order
//! of every effect of a reconcile, the events, the conflict short-circuits, and the two
//! deadlines.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::client::{Client, ListOptions, Patch};
use crossplane_core::errors::{Error, Result};
use crossplane_core::meta;
use crossplane_core::resource::{Managed, Object, ObjectKey, ObjectType};
use crossplane_core::v2::{Condition, ConditionType, ManagementAction};
use crossplane_test::fake::{as_modern_managed, new_modern_managed};
use crossplane_test::{self as test, MockClient};
use futures::future::BoxFuture;
use k8s_openapi::jiff::Timestamp;

use super::reconciler_modern_test::{FixedClock, Mg, boom, set_deleted, set_policies};
use super::*;
use crate::event::{Event, Recorder};
use crate::reconciler::{Outcome, Reconcile, Request};
use crate::resource::FinalizerFns;

/// Every effect of one reconcile, in the order it happened.
#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    fn push(&self, entry: impl Into<String>) {
        self.0.lock().unwrap().push(entry.into());
    }
    fn entries(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// A collaborator call that a scenario can fail or hang.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Site {
    Unpublish,
    RemoveFinalizer,
    Initialize,
    Resolve,
    Connect,
    Observe,
    PublishObserved,
    AddFinalizer,
    /// `client.update`: the create-pending annotation and the late-init write.
    ClientUpdate,
    Create,
    CriticalAnnotations,
    PublishCreated,
    Delete,
    Update,
    PublishUpdated,
    Status,
    DisconnectConnector,
    DisconnectClient,
}

#[derive(Clone)]
struct Scenario {
    /// What the API server holds, on top of generation 42.
    object: fn(&mut Mg),
    observation: ExternalObservation,
    fail: Vec<(Site, Error)>,
    /// The external call that never returns.
    hang: Option<Site>,
    options: fn(ReconcilerBuilder<Mg>) -> ReconcilerBuilder<Mg>,
}

impl Default for Scenario {
    fn default() -> Self {
        Self {
            object: |_| {},
            observation: ExternalObservation::default(),
            fail: Vec::new(),
            hang: None,
            options: |b| b,
        }
    }
}

impl Scenario {
    fn failing(mut self, site: Site, err: Error) -> Self {
        self.fail.push((site, err));
        self
    }
}

fn now() -> Timestamp {
    Timestamp::from_second(1_700_000_010).unwrap()
}

fn deleted(mg: &mut Mg) {
    set_deleted(mg, Timestamp::from_second(1_700_000_000).unwrap());
}

/// The journal and the scenario, shared by every double.
#[derive(Clone)]
struct Probe {
    journal: Journal,
    scenario: Arc<Scenario>,
}

impl Probe {
    /// Journals the call, then fails it when the scenario says so.
    fn call(&self, entry: &str, site: Site) -> Result<()> {
        self.journal.push(entry);
        match self.scenario.fail.iter().find(|(s, _)| *s == site) {
            Some((_, err)) => Err(err.clone()),
            None => Ok(()),
        }
    }

    async fn external(&self, entry: &str, site: Site) -> Result<()> {
        self.call(entry, site)?;
        if self.scenario.hang == Some(site) {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

impl ExternalConnector<Mg> for Probe {
    type Client = Probe;

    async fn connect(&self, _: &Mg) -> Result<Probe> {
        self.external("connect", Site::Connect).await?;
        Ok(self.clone())
    }
    async fn disconnect(&self) -> Result<()> {
        self.call("disconnect connector", Site::DisconnectConnector)
    }
}

impl ExternalClient<Mg> for Probe {
    async fn observe(&self, _: &mut Mg) -> Result<ExternalObservation> {
        self.external("observe", Site::Observe).await?;
        Ok(self.scenario.observation.clone())
    }
    /// Sets the external name, as a provider whose API assigns names does.
    async fn create(&self, mg: &mut Mg) -> Result<ExternalCreation> {
        self.external("create", Site::Create).await?;
        meta::set_external_name(mg, "created");
        Ok(ExternalCreation {
            connection_details: [("create".to_owned(), Vec::new())].into(),
            ..Default::default()
        })
    }
    async fn update(&self, _: &mut Mg) -> Result<ExternalUpdate> {
        self.external("update external", Site::Update).await?;
        Ok(ExternalUpdate {
            connection_details: [("update".to_owned(), Vec::new())].into(),
            ..Default::default()
        })
    }
    async fn delete(&self, _: &mut Mg) -> Result<ExternalDelete> {
        self.external("delete", Site::Delete).await?;
        Ok(ExternalDelete::default())
    }
    async fn disconnect(&self) -> Result<()> {
        self.call("disconnect client", Site::DisconnectClient)
    }
}

struct JournalRecorder {
    journal: Journal,
    annotations: BTreeMap<String, String>,
}

impl Recorder for JournalRecorder {
    fn event(&self, _: &dyn Object, event: Event) {
        let mut annotations = self.annotations.clone();
        annotations.extend(event.annotations);
        let annotations: Vec<String> = annotations.iter().map(|(k, v)| format!("{k}={v}")).collect();
        self.journal.push(format!(
            "event {:?} {} {:?} [{}]",
            event.type_,
            event.reason.0,
            event.message,
            annotations.join(" ")
        ));
    }
    fn with_annotations(&self, annotations: &[(&str, &str)]) -> Arc<dyn Recorder> {
        let mut merged = self.annotations.clone();
        merged.extend(annotations.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
        Arc::new(JournalRecorder {
            journal: self.journal.clone(),
            annotations: merged,
        })
    }
}

impl MetricRecorder for Journal {
    fn record_unchanged(&self, _: &str) {
        self.push("metric unchanged");
    }
    fn record_first_time_reconciled(&self, _: &MetricSubject<'_>) {
        self.push("metric first time reconciled");
    }
    fn record_first_time_ready(&self, _: &MetricSubject<'_>) {
        self.push("metric first time ready");
    }
    fn record_drift(&self, _: &MetricSubject<'_>) {
        self.push("metric drift");
    }
    fn record_deleted(&self, _: &MetricSubject<'_>) {
        self.push("metric deleted");
    }
}

impl ChangeLogger for Journal {
    fn log(&self, entry: ChangeLogEntry) -> BoxFuture<'_, Result<()>> {
        self.push(format!(
            "change log {:?} error={:?} external-name={:?}",
            entry.operation, entry.error_message, entry.external_name
        ));
        Box::pin(async { Ok(()) })
    }
}

/// `Type=Reason` of every condition, in the order the status carries them.
fn conditions(obj: &mut dyn Object) -> String {
    let mg = as_modern_managed(obj, 42);
    let all: Vec<String> = [ConditionType::READY, ConditionType::SYNCED]
        .iter()
        .filter(|t| mg.status.conditioned.conditions.iter().any(|c| &c.type_ == *t))
        .map(|t| format!("{}={}", t.0, mg.get_condition(t).reason.0))
        .collect();
    all.join(",")
}

fn reconciler(scenario: Scenario) -> (Reconciler<Mg>, Journal) {
    let journal = Journal::default();
    let probe = Probe {
        journal: journal.clone(),
        scenario: Arc::new(scenario),
    };
    let object = probe.scenario.object;

    let client = MockClient {
        get: Some({
            let journal = journal.clone();
            Box::new(move |_, _| {
                journal.push("get");
                let mut mg = new_modern_managed(42);
                object(&mut mg);
                Ok(Box::new(mg))
            })
        }),
        update: Some({
            let p = probe.clone();
            Box::new(move |_| p.call("update", Site::ClientUpdate))
        }),
        update_status: Some({
            let p = probe.clone();
            Box::new(move |obj| p.call(&format!("status {}", conditions(obj)), Site::Status))
        }),
        ..Default::default()
    };

    let fns = |entry: &'static str, site: Site| {
        let p = probe.clone();
        move || p.call(entry, site)
    };
    let initialize = fns("initialize", Site::Initialize);
    let resolve = fns("resolve", Site::Resolve);
    let add = fns("add finalizer", Site::AddFinalizer);
    let remove = fns("remove finalizer", Site::RemoveFinalizer);
    let annotate = fns("critical annotations", Site::CriticalAnnotations);
    let unpublish = fns("unpublish", Site::Unpublish);
    let publish = probe.clone();

    let builder = Reconciler::<Mg>::builder(Arc::new(client))
        .clock(Arc::new(FixedClock(now())))
        .initializers([InitializerFn::new(move |_| initialize())])
        .reference_resolver(ReferenceResolverFn::new(move |_| resolve()))
        .external_connector(probe.clone())
        .finalizer(FinalizerFns {
            add_finalizer: Some(Box::new(move |_| add())),
            remove_finalizer: Some(Box::new(move |_| remove())),
        })
        .critical_annotation_updater(CriticalAnnotationUpdateFn::new(move |_| annotate()))
        // Called after observe, create and update; the details say which.
        .connection_publisher(ConnectionPublisherFns {
            publish_connection: Some(Box::new(move |_, details| {
                let site = if details.contains_key("create") {
                    Site::PublishCreated
                } else if details.contains_key("update") {
                    Site::PublishUpdated
                } else {
                    Site::PublishObserved
                };
                publish.call("publish", site).map(|()| true)
            })),
            unpublish_connection: Some(Box::new(move |_, _| unpublish())),
        })
        .recorder(Arc::new(JournalRecorder {
            journal: journal.clone(),
            annotations: BTreeMap::new(),
        }))
        .metric_recorder(Arc::new(journal.clone()))
        .change_logger(Arc::new(journal.clone()));
    ((probe.scenario.options)(builder).build(), journal)
}

fn exists() -> ExternalObservation {
    ExternalObservation {
        resource_exists: true,
        ..Default::default()
    }
}

fn up_to_date() -> ExternalObservation {
    ExternalObservation {
        resource_up_to_date: true,
        ..exists()
    }
}

const POLL: Outcome = Outcome::RequeueAfter(Duration::from_secs(60));

/// One row per branch that has effects worth ordering. The expected journals follow
/// `reconciler.go` line by line: within a branch Go emits the event, then sets the conditions,
/// then writes status, and the deferred disconnect runs last.
#[tokio::test]
async fn effects_happen_in_go_order() {
    struct Case {
        name: &'static str,
        scenario: Scenario,
        want: Result<Outcome>,
        journal: &'static [&'static str],
    }

    let cases = [
        Case {
            name: "B03 paused",
            scenario: Scenario {
                object: |mg| {
                    mg.metadata.annotations =
                        Some([(meta::ANNOTATION_KEY_RECONCILIATION_PAUSED.to_owned(), "true".to_owned())].into());
                },
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            journal: &[
                "get",
                "metric first time reconciled",
                "event Normal ReconciliationPaused \"Reconciliation is paused either through the `spec.managementPolicies` or the pause annotation\" [annotation=crossplane.io/paused external-name=]",
                "status Synced=ReconcilePaused",
            ],
        },
        Case {
            name: "B05 invalid policy",
            scenario: Scenario {
                object: |mg| set_policies(mg, &[ManagementAction::Create]),
                options: |b| b.management_policies(),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            journal: &[
                "get",
                "metric first time reconciled",
                "event Warning CannotUseInvalidManagementPolicy \"`spec.managementPolicies` is set to a value([Create]) which is not supported. Check docs for supported policies\" [external-name=]",
                "status Synced=ReconcileError",
            ],
        },
        Case {
            name: "B07 orphaned delete cannot unpublish",
            scenario: Scenario {
                object: |mg| {
                    deleted(mg);
                    set_policies(mg, &[ManagementAction::Observe]);
                },
                options: |b| b.management_policies(),
                ..Default::default()
            }
            .failing(Site::Unpublish, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "unpublish",
                "event Warning CannotUnpublishConnectionDetails \"boom\" [external-name=]",
                "status Ready=Deleting,Synced=ReconcileError",
            ],
        },
        Case {
            name: "B09 orphaned delete cannot remove the finalizer",
            scenario: Scenario {
                object: |mg| {
                    deleted(mg);
                    set_policies(mg, &[ManagementAction::Observe]);
                },
                options: |b| b.management_policies(),
                ..Default::default()
            }
            .failing(Site::RemoveFinalizer, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "unpublish",
                "remove finalizer",
                "status Ready=Deleting,Synced=ReconcileError",
            ],
        },
        Case {
            name: "B10 orphaned delete",
            scenario: Scenario {
                object: |mg| {
                    deleted(mg);
                    set_policies(mg, &[ManagementAction::Observe]);
                },
                options: |b| b.management_policies(),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            journal: &[
                "get",
                "metric first time reconciled",
                "unpublish",
                "remove finalizer",
                "metric deleted",
            ],
        },
        Case {
            name: "B12 cannot initialize",
            scenario: Scenario::default().failing(Site::Initialize, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "event Warning CannotInitializeManagedResource \"boom\" [external-name=]",
                "status Synced=ReconcileError",
            ],
        },
        Case {
            name: "B13 creation result unknown",
            scenario: Scenario {
                object: |mg| meta::set_external_create_pending(mg, now()),
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "event Warning CannotInitializeManagedResource \"cannot determine creation result - remove the crossplane.io/external-create-pending annotation if it is safe to proceed\" [external-name=]",
                "status Ready=Creating,Synced=ReconcileError",
            ],
        },
        Case {
            name: "B15 cannot resolve references",
            scenario: Scenario::default().failing(Site::Resolve, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "event Warning CannotResolveResourceReferences \"boom\" [external-name=]",
                "status Synced=ReconcileError",
            ],
        },
        Case {
            name: "B17 cannot connect",
            scenario: Scenario::default().failing(Site::Connect, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "event Warning CannotConnectToProvider \"boom\" [external-name=]",
                "status Synced=ReconcileError",
            ],
        },
        Case {
            name: "B19 cannot observe",
            scenario: Scenario::default().failing(Site::Observe, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "event Warning CannotObserveExternalResource \"boom\" [external-name=]",
                "status Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B20 observe-only resource does not exist",
            scenario: Scenario {
                object: |mg| set_policies(mg, &[ManagementAction::Observe]),
                options: |b| b.management_policies(),
                ..Default::default()
            },
            want: Ok(POLL),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "event Warning CannotObserveExternalResource \"external resource does not exist\" [external-name=]",
                "status Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B21 inside the creation grace period",
            scenario: Scenario {
                object: |mg| meta::set_external_create_succeeded(mg, now()),
                ..Default::default()
            },
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "event Normal PendingExternalResource \"Waiting for external resource existence to be confirmed\" [external-name=]",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B23 cannot delete",
            scenario: Scenario {
                object: deleted,
                observation: exists(),
                ..Default::default()
            }
            .failing(Site::Delete, boom()),
            want: Ok(Outcome::Requeue),
            // S3: a deleted resource does not resolve references.
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "connect",
                "observe",
                "delete",
                "change log Delete error=Some(\"boom\") external-name=\"\"",
                "event Warning CannotDeleteExternalResource \"boom\" [external-name=]",
                "status Ready=Deleting,Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B24 delete requested",
            scenario: Scenario {
                object: deleted,
                observation: exists(),
                ..Default::default()
            },
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "connect",
                "observe",
                "delete",
                "change log Delete error=None external-name=\"\"",
                "event Normal DeletedExternalResource \"Successfully requested deletion of external resource\" [external-name=]",
                "status Ready=Deleting,Synced=ReconcileSuccess",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B29 external resource is gone",
            scenario: Scenario {
                object: deleted,
                ..Default::default()
            },
            want: Ok(Outcome::Done),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "connect",
                "observe",
                "unpublish",
                "remove finalizer",
                "metric deleted",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B31 cannot publish what observe returned",
            scenario: Scenario {
                observation: up_to_date(),
                ..Default::default()
            }
            .failing(Site::PublishObserved, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "event Warning CannotPublishConnectionDetails \"boom\" [external-name=]",
                "status Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B33 cannot add the finalizer",
            scenario: Scenario::default().failing(Site::AddFinalizer, boom()),
            want: Ok(Outcome::Requeue),
            // No event: Go only sets the condition here.
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "status Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B35 cannot mark the create pending",
            scenario: Scenario::default().failing(Site::ClientUpdate, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update",
                "event Warning CannotUpdateManagedResource \"cannot update managed resource: boom\" [external-name=]",
                "status Ready=Creating,Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B36 cannot create, and S4 cannot record that",
            scenario: Scenario::default()
                .failing(Site::Create, boom())
                .failing(Site::CriticalAnnotations, Error::new("annotations")),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update",
                "create",
                "event Warning CannotCreateExternalResource \"boom\" [external-name=]",
                "critical annotations",
                "event Warning CannotUpdateManagedResource \"cannot update managed resource annotations: annotations\" [external-name=]",
                "change log Create error=Some(\"boom\") external-name=\"\"",
                "status Ready=Creating,Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B38 created, cannot record that",
            scenario: Scenario::default().failing(Site::CriticalAnnotations, boom()),
            want: Ok(Outcome::Requeue),
            // X7: events after Create carry the external name Create set. The change log entry
            // is the object as it was before the operation.
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update",
                "create",
                "change log Create error=None external-name=\"\"",
                "critical annotations",
                "event Warning CannotUpdateManagedResource \"cannot update managed resource annotations: boom\" [external-name=created]",
                "status Ready=Creating,Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B40 created, cannot publish what create returned",
            scenario: Scenario::default().failing(Site::PublishCreated, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update",
                "create",
                "change log Create error=None external-name=\"\"",
                "critical annotations",
                "publish",
                "event Warning CannotPublishConnectionDetails \"boom\" [external-name=created]",
                "status Ready=Creating,Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B41 created",
            scenario: Scenario::default(),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update",
                "create",
                "change log Create error=None external-name=\"\"",
                "critical annotations",
                "publish",
                "event Normal CreatedExternalResource \"Successfully requested creation of external resource\" [external-name=created]",
                "status Ready=Creating,Synced=ReconcileSuccess",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B42 cannot persist late-initialized fields",
            scenario: Scenario {
                observation: ExternalObservation {
                    resource_late_initialized: true,
                    ..up_to_date()
                },
                ..Default::default()
            }
            .failing(Site::ClientUpdate, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update",
                "event Warning CannotUpdateManagedResource \"boom\" [external-name=]",
                "status Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "S5 and B43 late-initialized and up to date",
            scenario: Scenario {
                observation: ExternalObservation {
                    resource_late_initialized: true,
                    ..up_to_date()
                },
                ..Default::default()
            },
            want: Ok(POLL),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update",
                "metric first time ready",
                "metric unchanged",
                "status Synced=ReconcileSuccess",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B44 missing and not allowed to create",
            scenario: Scenario {
                object: |mg| set_policies(mg, &[ManagementAction::Observe, ManagementAction::Update]),
                options: |b| b.management_policies(),
                ..Default::default()
            },
            want: Ok(POLL),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "status Synced=ReconcileSuccess",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B46 cannot update",
            scenario: Scenario {
                observation: exists(),
                ..Default::default()
            }
            .failing(Site::Update, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update external",
                "change log Update error=Some(\"boom\") external-name=\"\"",
                "event Warning CannotUpdateExternalResource \"boom\" [external-name=]",
                "status Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B47 updated, cannot publish what update returned",
            scenario: Scenario {
                observation: exists(),
                ..Default::default()
            }
            .failing(Site::PublishUpdated, boom()),
            want: Ok(Outcome::Requeue),
            journal: &[
                "get",
                "metric first time reconciled",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update external",
                "metric drift",
                "change log Update error=None external-name=\"\"",
                "publish",
                "event Warning CannotPublishConnectionDetails \"boom\" [external-name=]",
                "status Synced=ReconcileError",
                "disconnect connector",
                "disconnect client",
            ],
        },
        Case {
            name: "B48 updated, with S1 and S7",
            scenario: Scenario {
                object: |mg| {
                    meta::set_external_name(mg, "named");
                    meta::set_reconcile_request(mg, "token-1");
                },
                observation: exists(),
                ..Default::default()
            }
            .failing(Site::DisconnectConnector, Error::new("connector"))
            .failing(Site::DisconnectClient, Error::new("client")),
            want: Ok(POLL),
            journal: &[
                "get",
                "metric first time reconciled",
                "event Normal ReconcileRequestHandled \"Handling reconcile request\" [external-name=named token=token-1]",
                "initialize",
                "resolve",
                "connect",
                "observe",
                "publish",
                "add finalizer",
                "update external",
                "metric drift",
                "change log Update error=None external-name=\"named\"",
                "publish",
                "event Normal UpdatedExternalResource \"Successfully requested update of external resource\" [external-name=named]",
                "status Synced=ReconcileSuccess",
                "disconnect connector",
                "event Warning CannotDisconnectFromProvider \"connector\" [external-name=named]",
                "disconnect client",
                "event Warning CannotDisconnectFromProvider \"client\" [external-name=named]",
            ],
        },
    ];

    for tc in cases {
        let (r, journal) = reconciler(tc.scenario);
        let got = r.reconcile(&Request::default()).await;
        test::assert_result(tc.name, "the outcome", &tc.want, &got);
        let want: Vec<String> = tc.journal.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(want, journal.entries(), "\n{}\n-want effects, +got effects", tc.name);
    }
}

/// A conflict from a collaborator means the object changed under the reconcile. At thirteen
/// sites Go requeues at once: no event, no condition, no status write. At the others a conflict
/// is an error like any other.
#[tokio::test]
async fn conflicts() {
    struct Case {
        name: &'static str,
        site: Site,
        scenario: Scenario,
        /// Whether Go short-circuits here.
        silent: bool,
    }

    let orphaned = || Scenario {
        object: |mg| {
            deleted(mg);
            set_policies(mg, &[ManagementAction::Observe]);
        },
        options: |b| b.management_policies(),
        ..Default::default()
    };
    let gone = || Scenario {
        object: deleted,
        ..Default::default()
    };
    let deleting = || Scenario {
        object: deleted,
        observation: exists(),
        ..Default::default()
    };
    let creating = Scenario::default;
    let current = || Scenario {
        observation: ExternalObservation {
            resource_late_initialized: true,
            ..up_to_date()
        },
        ..Default::default()
    };
    let drifted = || Scenario {
        observation: exists(),
        ..Default::default()
    };

    #[rustfmt::skip]
    let cases = [
        Case { name: "B06 unpublish, orphaned", site: Site::Unpublish, scenario: orphaned(), silent: true },
        Case { name: "B08 remove finalizer, orphaned", site: Site::RemoveFinalizer, scenario: orphaned(), silent: true },
        Case { name: "B11 initialize", site: Site::Initialize, scenario: creating(), silent: true },
        Case { name: "B14 resolve references", site: Site::Resolve, scenario: creating(), silent: true },
        Case { name: "B16 connect", site: Site::Connect, scenario: creating(), silent: true },
        Case { name: "B18 observe", site: Site::Observe, scenario: creating(), silent: true },
        Case { name: "B25 unpublish, gone", site: Site::Unpublish, scenario: gone(), silent: true },
        Case { name: "B27 remove finalizer, gone", site: Site::RemoveFinalizer, scenario: gone(), silent: true },
        Case { name: "B30 publish observed", site: Site::PublishObserved, scenario: creating(), silent: true },
        Case { name: "B32 add finalizer", site: Site::AddFinalizer, scenario: creating(), silent: true },
        Case { name: "B34 create pending", site: Site::ClientUpdate, scenario: creating(), silent: true },
        Case { name: "B37 critical annotations", site: Site::CriticalAnnotations, scenario: creating(), silent: true },
        Case { name: "B39 publish created", site: Site::PublishCreated, scenario: creating(), silent: true },
        Case { name: "B23 delete", site: Site::Delete, scenario: deleting(), silent: false },
        Case { name: "B36 create", site: Site::Create, scenario: creating(), silent: false },
        Case { name: "B42 late-init update", site: Site::ClientUpdate, scenario: current(), silent: false },
        Case { name: "B46 update", site: Site::Update, scenario: drifted(), silent: false },
        Case { name: "B47 publish updated", site: Site::PublishUpdated, scenario: drifted(), silent: false },
    ];

    for tc in cases {
        let (r, journal) = reconciler(tc.scenario.failing(tc.site, Error::conflict()));
        let got = r.reconcile(&Request::default()).await;
        test::assert_result(tc.name, "a conflict requeues", &Ok(Outcome::Requeue), &got);

        let entries = journal.entries();
        let wrote_status = entries.iter().any(|e| e.starts_with("status "));
        let conflict_events = entries
            .iter()
            .filter(|e| e.starts_with("event ") && e.contains("the object has been modified"))
            .count();
        assert_eq!(!tc.silent, wrote_status, "\n{}: status write\n{entries:#?}", tc.name);
        // B36 is the odd one: it reports the conflict in status but emits no event for it.
        let want_events = usize::from(!tc.silent && tc.site != Site::Create);
        assert_eq!(want_events, conflict_events, "\n{}: events\n{entries:#?}", tc.name);
    }
}

/// X1: a conflict on the status write itself is not an error either.
#[tokio::test]
async fn a_conflict_writing_status_requeues() {
    let scenario = Scenario {
        observation: up_to_date(),
        ..Default::default()
    };
    let (r, _) = reconciler(scenario.failing(Site::Status, Error::conflict()));
    let got = r.reconcile(&Request::default()).await;
    test::assert_result("X1", "a conflict requeues", &Ok(Outcome::Requeue), &got);
}

/// B03 returns before the reconcile-request token is taken, so a paused resource keeps its
/// request for when it resumes.
#[tokio::test]
async fn a_paused_resource_does_not_handle_a_reconcile_request() {
    let client = MockClient {
        get: Some(test::new_mock_get_fn(|| {
            let mut mg = new_modern_managed(42);
            meta::set_reconcile_request(&mut mg, "token-1");
            meta::add_annotations(
                &mut mg,
                [(meta::ANNOTATION_KEY_RECONCILIATION_PAUSED.to_owned(), "true".to_owned())],
            );
            mg
        })),
        update_status: Some(test::mock_status_update_fn(|obj| {
            assert_eq!("", as_modern_managed(obj, 42).last_handled_reconcile_at());
            Ok(())
        })),
        ..Default::default()
    };
    let got = Reconciler::<Mg>::builder(Arc::new(client))
        .build()
        .reconcile(&Request::default())
        .await;
    test::assert_result("paused", "the pause ends the reconcile", &Ok(Outcome::Done), &got);
}

/// The token is recorded by every branch that writes status, not only the successful ones.
#[tokio::test]
async fn a_failed_reconcile_still_handles_the_reconcile_request() {
    let client = MockClient {
        get: Some(test::new_mock_get_fn(|| {
            let mut mg = new_modern_managed(42);
            meta::set_reconcile_request(&mut mg, "token-1");
            mg
        })),
        update_status: Some(test::mock_status_update_fn(|obj| {
            assert_eq!("token-1", as_modern_managed(obj, 42).last_handled_reconcile_at());
            Ok(())
        })),
        ..Default::default()
    };
    let got = Reconciler::<Mg>::builder(Arc::new(client))
        .initializers([InitializerFn::new(|_| Err(boom()))])
        .build()
        .reconcile(&Request::default())
        .await;
    test::assert_result("failed", "the initializer fails", &Ok(Outcome::Requeue), &got);
}

/// Go gives every external call one deadline, 60 seconds after the reconcile starts, and the
/// reconcile as a whole 30 seconds more. An external call that hangs fails at the first; the
/// status write still has the grace period to land.
#[tokio::test(start_paused = true)]
async fn a_hung_external_call_fails_at_the_timeout_and_status_is_still_written() {
    let scenario = Scenario {
        hang: Some(Site::Observe),
        ..Default::default()
    };
    let (r, journal) = reconciler(scenario);
    let started = tokio::time::Instant::now();
    let got = r.reconcile(&Request::default()).await;

    test::assert_result("hang", "observe never returns", &Ok(Outcome::Requeue), &got);
    assert_eq!(Duration::from_secs(60), started.elapsed());
    let entries = journal.entries();
    assert_eq!(
        [
            "observe",
            "event Warning CannotObserveExternalResource \"context deadline exceeded\" [external-name=]",
            "status Synced=ReconcileError",
            "disconnect connector",
            "disconnect client",
        ]
        .as_slice(),
        &entries[entries.len() - 5..]
    );
}

/// The configured timeout is the one that applies.
#[tokio::test(start_paused = true)]
async fn the_external_deadline_follows_the_configured_timeout() {
    let scenario = Scenario {
        hang: Some(Site::Create),
        options: |b| b.timeout(Duration::from_secs(5)),
        ..Default::default()
    };
    let (r, journal) = reconciler(scenario);
    let started = tokio::time::Instant::now();
    let got = r.reconcile(&Request::default()).await;

    test::assert_result("hang", "create never returns", &Ok(Outcome::Requeue), &got);
    assert_eq!(Duration::from_secs(5), started.elapsed());
    assert!(
        journal
            .entries()
            .contains(&"change log Create error=Some(\"context deadline exceeded\") external-name=\"\"".to_owned())
    );
}

/// A client whose status write never returns.
struct HungStatus(MockClient);

impl Client for HungStatus {
    fn get<'a>(&'a self, t: &'a ObjectType, key: &'a ObjectKey) -> BoxFuture<'a, Result<Box<dyn Object>>> {
        self.0.get(t, key)
    }
    fn list<'a>(&'a self, t: &'a ObjectType, o: &'a ListOptions) -> BoxFuture<'a, Result<Vec<Box<dyn Object>>>> {
        self.0.list(t, o)
    }
    fn create<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        self.0.create(obj)
    }
    fn update<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        self.0.update(obj)
    }
    fn update_status<'a>(&'a self, _: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        Box::pin(std::future::pending())
    }
    fn patch<'a>(&'a self, obj: &'a mut dyn Object, patch: Patch) -> BoxFuture<'a, Result<()>> {
        self.0.patch(obj, patch)
    }
    fn delete<'a>(&'a self, obj: &'a dyn Object) -> BoxFuture<'a, Result<()>> {
        self.0.delete(obj)
    }
}

#[tokio::test(start_paused = true)]
async fn a_hung_status_write_fails_at_the_outer_deadline() {
    let client = HungStatus(MockClient {
        get: Some(test::new_mock_get_fn(|| new_modern_managed(42))),
        ..Default::default()
    });
    let r = Reconciler::<Mg>::builder(Arc::new(client))
        .initializers([InitializerFn::new(|_| Err(boom()))])
        .build();
    let started = tokio::time::Instant::now();
    let got = r.reconcile(&Request::default()).await;

    let want = Err(Error::deadline_exceeded().wrap(ERR_UPDATE_MANAGED_STATUS));
    test::assert_result("hang", "the status write never returns", &want, &got);
    assert_eq!(
        "cannot update managed resource status: context deadline exceeded",
        got.unwrap_err().to_string()
    );
    assert_eq!(Duration::from_secs(90), started.elapsed());
}

/// What the parity audit measured on the Go stack for an observe-only resource: the first
/// reconcile writes `Synced` without the `Ready` condition Observe just set, and requeues after
/// the poll interval. Adding the finalizer PUTs the object, the API server's response to a PUT
/// of the main resource carries the stored status, and the client decodes the response over the
/// object. The second reconcile finds the finalizer, PUTs nothing, and keeps `Ready`.
#[tokio::test]
async fn adding_the_finalizer_drops_the_conditions_observe_set() {
    fn observes_available() -> ExternalConnectorFn<Mg> {
        ExternalConnectorFn::new(|_| {
            Ok(ExternalClientFns {
                observe: Some(Box::new(|mg: &mut Mg| {
                    mg.set_conditions([Condition::available()]);
                    Ok(up_to_date())
                })),
                ..Default::default()
            })
        })
    }
    fn stored(finalized: bool) -> Mg {
        let mut mg = new_modern_managed(42);
        meta::set_external_name(&mut mg, "seeded");
        if finalized {
            meta::add_finalizer(&mut mg, FINALIZER_NAME);
        }
        mg
    }
    fn reconciler(finalized: bool, want_status: &'static str) -> Reconciler<Mg> {
        let client = MockClient {
            get: Some(test::new_mock_get_fn(move || stored(finalized))),
            // The server's answer to the finalizer PUT: the stored object, whose status is
            // still empty. `None` on the second pass: nothing may be PUT.
            update: (!finalized).then(|| test::new_mock_response_fn(|| stored(true))),
            update_status: Some(test::mock_status_update_fn(move |obj| {
                assert_eq!(want_status, conditions(obj));
                Ok(())
            })),
            ..Default::default()
        };
        // The default finalizer and initializer; the external name is preset, as in the audit
        // scenario, so the initializer writes nothing.
        Reconciler::<Mg>::builder(Arc::new(client))
            .external_connector(observes_available())
            .build()
    }

    let first = reconciler(false, "Synced=ReconcileSuccess")
        .reconcile(&Request::default())
        .await;
    test::assert_result("first", "the first reconcile polls", &Ok(POLL), &first);

    let second = reconciler(true, "Ready=Available,Synced=ReconcileSuccess")
        .reconcile(&Request::default())
        .await;
    test::assert_result("second", "the second reconcile polls", &Ok(POLL), &second);
}

/// client-go's `retry.DefaultRetry`: five attempts, ten milliseconds apart with up to ten
/// percent of jitter. After a conflict the next attempt writes the saved annotations onto the
/// object the server holds.
#[tokio::test(start_paused = true)]
async fn the_critical_annotation_updater_retries_onto_the_latest_object() {
    let updates = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
    let client = MockClient {
        get: Some(test::new_mock_get_fn(|| {
            let mut latest = new_modern_managed(43);
            latest.metadata.labels = Some([("latest".to_owned(), "true".to_owned())].into());
            latest
        })),
        update: Some({
            let updates = updates.clone();
            Box::new(move |obj| {
                let mut updates = updates.lock().unwrap();
                updates.push(obj.to_json()?);
                // Two conflicts, then the write lands.
                if updates.len() <= 2 {
                    Err(Error::conflict())
                } else {
                    Ok(())
                }
            })
        }),
        ..Default::default()
    };
    let mut mg = new_modern_managed(42);
    meta::set_external_name(&mut mg, "critical");

    let started = tokio::time::Instant::now();
    let got = RetryingCriticalAnnotationUpdater::new(Arc::new(client))
        .update_critical_annotations(&mut mg)
        .await;

    test::assert_result("retry", "the third attempt lands", &Ok(()), &got);
    let waited = started.elapsed();
    assert!(
        (Duration::from_millis(20)..=Duration::from_millis(22)).contains(&waited),
        "two waits of 10ms plus jitter, got {waited:?}"
    );
    let updates = updates.lock().unwrap();
    assert_eq!(3, updates.len());
    // The retries carry the annotation and are the server's object, not the stale one.
    for update in &updates[1..] {
        assert_eq!(
            "critical",
            update["metadata"]["annotations"]["crossplane.io/external-name"]
        );
        assert_eq!("true", update["metadata"]["labels"]["latest"]);
        assert_eq!(43, update["metadata"]["generation"]);
    }
}

#[tokio::test(start_paused = true)]
async fn the_critical_annotation_updater_gives_up_after_five_attempts() {
    let attempts = Arc::new(Mutex::new(0));
    let client = MockClient {
        update: Some({
            let attempts = attempts.clone();
            Box::new(move |_| {
                *attempts.lock().unwrap() += 1;
                Err(boom())
            })
        }),
        ..Default::default()
    };
    let got = RetryingCriticalAnnotationUpdater::new(Arc::new(client))
        .update_critical_annotations(&mut new_modern_managed(42))
        .await;

    let want = Err(boom().wrap(ERR_UPDATE_CRITICAL_ANNOTATIONS));
    test::assert_result("retry", "every attempt fails", &want, &got);
    assert_eq!(5, *attempts.lock().unwrap());
}

/// What Go's `log.WithValues` puts on every line of a request, as tracing span fields.
const GO_REQUEST_FIELDS: &str = r#"request={"name":"mg","namespace":"ns"}"#;

fn logged_lines(logged: &str) -> Vec<&str> {
    logged.lines().filter(|l| l.contains("reconcile{")).collect()
}

fn assert_logged(logged: &str, message: &str, fields: &[&str]) {
    let line = logged_lines(logged)
        .into_iter()
        .find(|l| l.ends_with(&format!(": {message}")) || l.contains(&format!(": {message} ")))
        .unwrap_or_else(|| panic!("no line for {message:?} in:\n{logged}"));
    assert!(line.contains(" DEBUG "), "{message:?} is a debug line in Go: {line}");
    assert!(
        line.contains(GO_REQUEST_FIELDS),
        "{message:?} lacks the request: {line}"
    );
    for field in fields {
        assert!(line.contains(field), "{message:?} lacks {field:?}: {line}");
    }
}

#[tokio::test]
async fn an_up_to_date_resource_logs_reconciling_and_up_to_date_with_the_request_fields() {
    let scenario = Scenario {
        observation: up_to_date(),
        ..Default::default()
    };
    let (r, _) = reconciler(scenario);
    let (got, logged) = crate::logging::capture(r.reconcile(&Request::namespaced("ns", "mg"))).await;
    test::assert_result("B43", "up to date polls", &Ok(POLL), &got);

    assert_logged(&logged, "Reconciling", &[]);
    assert_logged(
        &logged,
        "External resource is up to date",
        &[
            "uid=",
            "version=",
            "external-name=",
            "requeue-after=2023-11-14T22:14:30Z",
        ],
    );
}

#[tokio::test]
async fn a_create_logs_the_request_and_the_external_name_the_create_set() {
    let (r, _) = reconciler(Scenario::default());
    let (got, logged) = crate::logging::capture(r.reconcile(&Request::namespaced("ns", "mg"))).await;
    test::assert_result("B41", "create requeues", &Ok(Outcome::Requeue), &got);

    assert_logged(
        &logged,
        "Successfully requested creation of external resource",
        &["uid=", r#"external-name="created""#],
    );
}

#[tokio::test]
async fn a_missing_managed_resource_logs_cannot_get_managed_resource() {
    let client = MockClient {
        get: Some(Box::new(|_, _| Err(Error::not_found()))),
        ..Default::default()
    };
    let r = Reconciler::<Mg>::builder(Arc::new(client)).build();
    let (got, logged) = crate::logging::capture(r.reconcile(&Request::namespaced("ns", "mg"))).await;
    test::assert_result("B02", "not found is done", &Ok(Outcome::Done), &got);

    assert_logged(&logged, "Reconciling", &[]);
    assert_logged(&logged, "Cannot get managed resource", &["error="]);
}
