//! `pkg/reconciler/managed`: the managed resource reconciler.
//!
//! [`Reconciler<M>`] is generic over the provider's kind, so collaborators and the external
//! client receive `&mut MyType`, not an interface to assert. [`ReconcilerBuilder`] is Go's
//! `ReconcilerOption` list: every collaborator a Go table replaces is a builder method, and
//! `build()` with no options gives Go's defaults.
//!
//! The 48 return branches of `docs/phase0/02-parity-spec.md` section 1.3 live in
//! [`Reconciler::run`] (B01-B03) and [`Run`] (B05-B48), one method per phase, each documented
//! with the branch IDs it owns and the Go lines it ports. `R:n` and `A:n` are that document's
//! aliases for `reconciler.go:n` and `api.go:n` in the Go package. Within a branch the order of
//! effects is Go's: change log, event, condition, status write.

pub mod changelog;
mod collaborators;
mod external;
mod metrics;
pub mod policy;

#[cfg(test)]
mod api_test;
#[cfg(test)]
mod changelogger_test;
#[cfg(test)]
mod policy_test;
#[cfg(test)]
mod reconciler_annotations_test;
#[cfg(test)]
mod reconciler_effects_test;
#[cfg(test)]
mod reconciler_modern_test;

use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::Client;
use crossplane_core::errors::{Error, Result, ResultExt};
use crossplane_core::meta;
use crossplane_core::resource::{Kind, Managed, ObjectType, mark_conditions};
use crossplane_core::v2::{Condition, ConnectionDetails};
use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::time::Instant;
use tracing::Instrument;
use tracing::field::Empty;

pub use self::changelog::{ChangeLogEntry, ChangeLogger, GrpcChangeLogger, NopChangeLogger, OperationType};
pub use self::collaborators::{
    ApiLocalSecretPublisher, ApiSimpleReferenceResolver, ConnectionPublisher, ConnectionPublisherFns,
    CriticalAnnotationUpdateFn, CriticalAnnotationUpdater, Initializer, InitializerFn, NameAsExternalName,
    ReferenceResolver, ReferenceResolverFn, RetryingCriticalAnnotationUpdater,
};
pub use self::external::{
    AdditionalDetails, ExternalClient, ExternalClientFns, ExternalConnector, ExternalConnectorFn, ExternalCreation,
    ExternalDelete, ExternalObservation, ExternalUpdate, NopClient, NopConnector,
};
use self::external::{DynConnector, DynExternal};
pub use self::metrics::{MetricRecorder, MetricSubject, MrMetricRecorder, NopMetricRecorder};
use self::policy::{ActionSet, PolicyResolver, default_supported_management_policies};
use crate::clock::{Clock, SystemClock};
use crate::event::{Event, NopRecorder, Recorder};
use crate::logging;
use crate::reconciler::deadline::bounded;
use crate::reconciler::{Outcome, Reconcile, Request, silently_requeue_on_conflict};
use crate::resource::{ApiFinalizer, Finalizer};

pub const FINALIZER_NAME: &str = "finalizer.managedresource.crossplane.io";
pub const REFERENCE_RESOLVER_FIELD_OWNER: &str = "managed.crossplane.io/api-simple-reference-resolver";

/// Every duration the reconciler runs on, in one place. `Default` is Go's, from
/// `pkg/reconciler/managed/reconciler.go` at v2.4.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// How long until an up-to-date resource is observed again. `defaultPollInterval`, line 52.
    pub poll_interval: Duration,
    /// The floor for the per-object poll-interval annotation. Unset in `NewReconciler` (lines
    /// 884-916), so zero.
    pub min_poll_interval: Duration,
    /// The deadline shared by every external call of one reconcile. `reconcileTimeout`, line 50.
    pub timeout: Duration,
    /// What the whole reconcile gets on top of `timeout`, so the status write survives an
    /// external timeout. `reconcileGracePeriod`, line 49; used at line 927. Go has no option
    /// for it, and neither does the builder.
    pub grace: Duration,
    /// How long after a successful create a missing external resource is tolerated.
    /// `defaultGracePeriod`, line 53.
    pub creation_grace: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(60),
            min_poll_interval: Duration::ZERO,
            timeout: Duration::from_secs(60),
            grace: Duration::from_secs(30),
            creation_grace: Duration::from_secs(30),
        }
    }
}

pub const ERR_GET_MANAGED: &str = "cannot get managed resource";
pub const ERR_UPDATE_MANAGED_ANNOTATIONS: &str = "cannot update managed resource annotations";
pub const ERR_CREATE_INCOMPLETE: &str = "cannot determine creation result - remove the crossplane.io/external-create-pending annotation if it is safe to proceed";
pub const ERR_RECONCILE_CONNECT: &str = "connect failed";
pub const ERR_RECONCILE_OBSERVE: &str = "observe failed";
pub const ERR_RECONCILE_CREATE: &str = "create failed";
pub const ERR_RECONCILE_UPDATE: &str = "update failed";
pub const ERR_RECONCILE_DELETE: &str = "delete failed";
pub const ERR_RECORD_CHANGE_LOG: &str = "cannot record change log entry";
pub const ERR_EXTERNAL_RESOURCE_NOT_EXIST: &str = "external resource does not exist";
pub const ERR_UPDATE_MANAGED: &str = "cannot update managed resource";
pub const ERR_PATCH_MANAGED: &str = "cannot patch the managed resource via server-side apply";
pub const ERR_MARSHAL_EXISTING: &str = "cannot marshal the existing object into JSON";
pub const ERR_MARSHAL_RESOLVED: &str = "cannot marshal the object with the resolved references into JSON";
pub const ERR_UPDATE_MANAGED_STATUS: &str = "cannot update managed resource status";
pub const ERR_RESOLVE_REFERENCES: &str = "cannot resolve references";
pub const ERR_UPDATE_CRITICAL_ANNOTATIONS: &str = "cannot update critical annotations";
pub const ERR_CREATE_OR_UPDATE_SECRET: &str = "cannot create or update connection secret";

pub const MSG_RECONCILIATION_PAUSED: &str =
    "Reconciliation is paused either through the `spec.managementPolicies` or the pause annotation";
pub const MSG_RECONCILE_REQUEST_HANDLED: &str = "Handling reconcile request";
pub const MSG_PENDING: &str = "Waiting for external resource existence to be confirmed";
pub const MSG_DELETED: &str = "Successfully requested deletion of external resource";
pub const MSG_CREATED: &str = "Successfully requested creation of external resource";
pub const MSG_UPDATED: &str = "Successfully requested update of external resource";

pub mod reason {
    use std::borrow::Cow::Borrowed;

    use crate::event::Reason;

    pub const CANNOT_CONNECT: Reason = Reason(Borrowed("CannotConnectToProvider"));
    pub const CANNOT_DISCONNECT: Reason = Reason(Borrowed("CannotDisconnectFromProvider"));
    pub const CANNOT_INITIALIZE: Reason = Reason(Borrowed("CannotInitializeManagedResource"));
    pub const CANNOT_RESOLVE_REFS: Reason = Reason(Borrowed("CannotResolveResourceReferences"));
    pub const CANNOT_OBSERVE: Reason = Reason(Borrowed("CannotObserveExternalResource"));
    pub const CANNOT_CREATE: Reason = Reason(Borrowed("CannotCreateExternalResource"));
    pub const CANNOT_DELETE: Reason = Reason(Borrowed("CannotDeleteExternalResource"));
    pub const CANNOT_PUBLISH: Reason = Reason(Borrowed("CannotPublishConnectionDetails"));
    pub const CANNOT_UNPUBLISH: Reason = Reason(Borrowed("CannotUnpublishConnectionDetails"));
    pub const CANNOT_UPDATE: Reason = Reason(Borrowed("CannotUpdateExternalResource"));
    pub const CANNOT_UPDATE_MANAGED: Reason = Reason(Borrowed("CannotUpdateManagedResource"));
    pub const CANNOT_USE_INVALID_POLICY: Reason = Reason(Borrowed("CannotUseInvalidManagementPolicy"));
    pub const DELETED: Reason = Reason(Borrowed("DeletedExternalResource"));
    pub const CREATED: Reason = Reason(Borrowed("CreatedExternalResource"));
    pub const UPDATED: Reason = Reason(Borrowed("UpdatedExternalResource"));
    pub const PENDING: Reason = Reason(Borrowed("PendingExternalResource"));
    pub const RECONCILIATION_PAUSED: Reason = Reason(Borrowed("ReconciliationPaused"));
    pub const RECONCILE_REQUEST_HANDLED: Reason = Reason(Borrowed("ReconcileRequestHandled"));
}

/// Go `ControllerName(kind)`: `"managed/" + lower(<Kind>.<group>)`.
pub fn controller_name<K: Kind>() -> String {
    format!("managed/{}", ObjectType::of::<K>().group_kind()).to_lowercase()
}

/// Go `PollIntervalHook`.
pub type PollIntervalHook<M> = Arc<dyn Fn(&M, Duration) -> Duration + Send + Sync>;

/// Built only through [`Reconciler::builder`].
pub struct Reconciler<M: Managed> {
    client: Arc<dyn Client>,
    gvk: String,

    timing: Timing,
    poll_interval_hook: PollIntervalHook<M>,

    management_policies: bool,
    supported_policies: Vec<ActionSet>,
    deterministic_external_name: bool,

    external: Arc<dyn DynConnector<M>>,
    initializers: Vec<Arc<dyn Initializer<M>>>,
    reference_resolver: Arc<dyn ReferenceResolver<M>>,
    finalizer: Arc<dyn Finalizer<M>>,
    critical_annotations: Arc<dyn CriticalAnnotationUpdater<M>>,
    publisher: Arc<dyn ConnectionPublisher<M>>,

    recorder: Arc<dyn Recorder>,
    metrics: Arc<dyn MetricRecorder>,
    change_logger: Arc<dyn ChangeLogger>,
    clock: Arc<dyn Clock>,
}

impl<M: Managed> Reconciler<M> {
    /// Go `NewReconciler(mgr, kind, opts...)`. The client is the only required input; the scheme
    /// and `ManagedKind` disappear because `M` is the kind.
    pub fn builder(client: Arc<dyn Client>) -> ReconcilerBuilder<M> {
        ReconcilerBuilder::new(client)
    }
}

/// Go `ReconcilerOption`s. Every `Option` field left `None` takes the Go default in
/// [`ReconcilerBuilder::build`].
pub struct ReconcilerBuilder<M: Managed> {
    client: Arc<dyn Client>,
    timing: Timing,
    poll_interval_hook: Option<PollIntervalHook<M>>,
    management_policies: bool,
    supported_policies: Option<Vec<ActionSet>>,
    deterministic_external_name: bool,
    external: Option<Arc<dyn DynConnector<M>>>,
    initializers: Option<Vec<Arc<dyn Initializer<M>>>>,
    reference_resolver: Option<Arc<dyn ReferenceResolver<M>>>,
    finalizer: Option<Arc<dyn Finalizer<M>>>,
    critical_annotations: Option<Arc<dyn CriticalAnnotationUpdater<M>>>,
    publisher: Option<Arc<dyn ConnectionPublisher<M>>>,
    recorder: Option<Arc<dyn Recorder>>,
    metrics: Option<Arc<dyn MetricRecorder>>,
    change_logger: Option<Arc<dyn ChangeLogger>>,
    clock: Option<Arc<dyn Clock>>,
}

impl<M: Managed> ReconcilerBuilder<M> {
    fn new(client: Arc<dyn Client>) -> Self {
        Self {
            client,
            timing: Timing::default(),
            poll_interval_hook: None,
            management_policies: false,
            supported_policies: None,
            deterministic_external_name: false,
            external: None,
            initializers: None,
            reference_resolver: None,
            finalizer: None,
            critical_annotations: None,
            publisher: None,
            recorder: None,
            metrics: None,
            change_logger: None,
            clock: None,
        }
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timing.timeout = d;
        self
    }
    pub fn poll_interval(mut self, d: Duration) -> Self {
        self.timing.poll_interval = d;
        self
    }
    pub fn min_poll_interval(mut self, d: Duration) -> Self {
        self.timing.min_poll_interval = d;
        self
    }
    /// `WithPollIntervalHook`. The last call wins, as in Go.
    pub fn poll_interval_hook(mut self, hook: impl Fn(&M, Duration) -> Duration + Send + Sync + 'static) -> Self {
        self.poll_interval_hook = Some(Arc::new(hook));
        self
    }
    /// `WithPollJitterHook`: uniform in `[-jitter, +jitter)`.
    pub fn poll_jitter_hook(self, jitter: Duration) -> Self {
        self.poll_interval_hook(move |_, poll| {
            let offset = (rand::random::<f64>() - 0.5) * 2.0 * jitter.as_secs_f64();
            Duration::try_from_secs_f64(poll.as_secs_f64() + offset).unwrap_or(Duration::ZERO)
        })
    }
    pub fn creation_grace_period(mut self, d: Duration) -> Self {
        self.timing.creation_grace = d;
        self
    }
    /// `WithManagementPolicies`: the beta feature is on for this reconciler.
    pub fn management_policies(mut self) -> Self {
        self.management_policies = true;
        self
    }
    pub fn supported_management_policies(mut self, s: Vec<ActionSet>) -> Self {
        self.supported_policies = Some(s);
        self
    }
    pub fn deterministic_external_name(mut self, b: bool) -> Self {
        self.deterministic_external_name = b;
        self
    }
    /// `WithExternalConnector`. The connector's own `disconnect` defaults to a no-op, which is
    /// Go's `NopDisconnector` wrap.
    pub fn external_connector(mut self, c: impl ExternalConnector<M>) -> Self {
        self.external = Some(Arc::new(c));
        self
    }
    /// `WithInitializers`: replaces the whole chain.
    pub fn initializers(mut self, i: impl IntoIterator<Item = impl Initializer<M>>) -> Self {
        self.initializers = Some(i.into_iter().map(|i| Arc::new(i) as Arc<dyn Initializer<M>>).collect());
        self
    }
    pub fn reference_resolver(mut self, r: impl ReferenceResolver<M>) -> Self {
        self.reference_resolver = Some(Arc::new(r));
        self
    }
    pub fn finalizer(mut self, f: impl Finalizer<M>) -> Self {
        self.finalizer = Some(Arc::new(f));
        self
    }
    pub fn critical_annotation_updater(mut self, u: impl CriticalAnnotationUpdater<M>) -> Self {
        self.critical_annotations = Some(Arc::new(u));
        self
    }
    /// Go keeps this unexported (`withLocalConnectionPublishers`) and only its tests call it.
    #[cfg(test)]
    pub(crate) fn connection_publisher(mut self, p: impl ConnectionPublisher<M>) -> Self {
        self.publisher = Some(Arc::new(p));
        self
    }
    pub fn recorder(mut self, r: Arc<dyn Recorder>) -> Self {
        self.recorder = Some(r);
        self
    }
    pub fn metric_recorder(mut self, m: Arc<dyn MetricRecorder>) -> Self {
        self.metrics = Some(m);
        self
    }
    pub fn change_logger(mut self, c: Arc<dyn ChangeLogger>) -> Self {
        self.change_logger = Some(c);
        self
    }
    /// Not in Go: injects `now()` for the create annotations and the creation grace period.
    pub fn clock(mut self, c: Arc<dyn Clock>) -> Self {
        self.clock = Some(c);
        self
    }

    pub fn build(self) -> Reconciler<M> {
        let c = self.client;
        Reconciler {
            gvk: format!("{:?}", ObjectType::of::<M>()),
            timing: self.timing,
            poll_interval_hook: self.poll_interval_hook.unwrap_or_else(|| Arc::new(|_, d| d)),
            management_policies: self.management_policies,
            supported_policies: self
                .supported_policies
                .unwrap_or_else(default_supported_management_policies),
            deterministic_external_name: self.deterministic_external_name,
            external: self.external.unwrap_or_else(|| Arc::new(NopConnector)),
            initializers: self
                .initializers
                .unwrap_or_else(|| vec![Arc::new(NameAsExternalName::new(c.clone())) as Arc<dyn Initializer<M>>]),
            reference_resolver: self
                .reference_resolver
                .unwrap_or_else(|| Arc::new(ApiSimpleReferenceResolver::new(c.clone()))),
            finalizer: self
                .finalizer
                .unwrap_or_else(|| Arc::new(ApiFinalizer::new(c.clone(), FINALIZER_NAME))),
            critical_annotations: self
                .critical_annotations
                .unwrap_or_else(|| Arc::new(RetryingCriticalAnnotationUpdater::new(c.clone()))),
            publisher: self
                .publisher
                .unwrap_or_else(|| Arc::new(ApiLocalSecretPublisher::new(c.clone()))),
            recorder: self.recorder.unwrap_or_else(|| Arc::new(NopRecorder)),
            metrics: self.metrics.unwrap_or_else(|| Arc::new(NopMetricRecorder)),
            change_logger: self.change_logger.unwrap_or_else(|| Arc::new(NopChangeLogger)),
            clock: self.clock.unwrap_or_else(|| Arc::new(SystemClock)),
            client: c,
        }
    }
}

/// Go's two contexts (X2, `R:927-931`). Connect, Observe, Create, Update and Delete end at
/// `external`; every Kubernetes call, publish, change log and disconnect ends at `kubernetes`,
/// `Timing::grace` later, so the status write survives an external timeout.
#[derive(Clone, Copy)]
struct Deadlines {
    external: Instant,
    kubernetes: Instant,
}

impl<M: Managed> Reconcile for Reconciler<M> {
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        async move {
            let start = Instant::now();
            let deadlines = Deadlines {
                external: start + self.timing.timeout,
                kubernetes: start + self.timing.timeout + self.timing.grace,
            };
            let span = tracing::info_span!(
                "reconcile",
                request = %logging::RequestFields(req),
                uid = Empty,
                version = Empty,
                "external-name" = Empty,
                "deletion-timestamp" = Empty,
            );
            async {
                tracing::debug!("Reconciling");
                // X1 (R:922).
                silently_requeue_on_conflict(self.run(req, deadlines).await)
            }
            .instrument(span)
            .await
        }
        .boxed()
    }
}

impl<M: Managed> Reconciler<M> {
    /// B01-B03 and S1 (`R:933-994`), then the rest of the run.
    async fn run(&self, req: &Request, deadlines: Deadlines) -> Result<Outcome> {
        // B01, B02
        let mut mg = match bounded(deadlines.kubernetes, self.client.get_as::<M>(req)).await {
            Ok(mg) => mg,
            Err(e) => {
                tracing::debug!(error = %e, "Cannot get managed resource");
                return if e.is_not_found() {
                    Ok(Outcome::Done)
                } else {
                    Err(e.wrap(ERR_GET_MANAGED))
                };
            }
        };
        let span = tracing::Span::current();
        span.record("uid", mg.meta().uid.as_deref().unwrap_or_default());
        span.record("version", mg.meta().resource_version.as_deref().unwrap_or_default());
        span.record("external-name", meta::external_name(&mg));
        // X6
        self.metrics.record_first_time_reconciled(&self.subject(&mg));
        // X7
        let record = self
            .recorder
            .with_annotations(&[("external-name", meta::external_name(&mg))]);
        let policy = PolicyResolver::new(
            self.management_policies,
            mg.management_policies(),
            &self.supported_policies,
        );

        // B03. The status write does not go through `Run::finish`, so a paused resource never
        // records a reconcile-request token.
        if meta::is_paused(&mg) || policy.is_paused() {
            tracing::debug!(
                annotation = meta::ANNOTATION_KEY_RECONCILIATION_PAUSED,
                "{MSG_RECONCILIATION_PAUSED}"
            );
            record.event(
                &mg,
                Event::normal(reason::RECONCILIATION_PAUSED, MSG_RECONCILIATION_PAUSED)
                    .with_annotation("annotation", meta::ANNOTATION_KEY_RECONCILIATION_PAUSED),
            );
            mark_conditions(&mut mg, [Condition::reconcile_paused()]);
            return bounded(deadlines.kubernetes, self.client.update_status(&mut mg))
                .await
                .map(|()| Outcome::Done)
                .wrap_err(ERR_UPDATE_MANAGED_STATUS);
        }

        // S1. The token reaches status only in `Run::finish`, so it survives the full-object
        // updates in between.
        let token = match meta::reconcile_request(&mg) {
            Some(t) if M::TRACKS_RECONCILE_REQUESTS && mg.last_handled_reconcile_at() != t => {
                tracing::debug!(token = t, "Processing reconcile request");
                record.event(
                    &mg,
                    Event::normal(reason::RECONCILE_REQUEST_HANDLED, MSG_RECONCILE_REQUEST_HANDLED)
                        .with_annotation("token", t),
                );
                Some(t.to_owned())
            }
            _ => None,
        };

        Run {
            r: self,
            mg,
            record,
            policy,
            token,
            deadlines,
        }
        .reconcile()
        .await
    }

    fn subject<'a>(&'a self, mg: &'a M) -> MetricSubject<'a> {
        MetricSubject {
            gvk: &self.gvk,
            meta: mg.meta(),
            conditions: mg.managed_status().map(|s| &s.conditioned),
        }
    }

    /// Go `effectivePollInterval` (`R:867-875`): the annotation, floored at the minimum; else the
    /// configured interval, which the floor does not apply to.
    fn effective_poll_interval(&self, mg: &M) -> Duration {
        match meta::poll_interval(mg) {
            Some(d) => d.max(self.timing.min_poll_interval),
            None => self.timing.poll_interval,
        }
    }

    /// `pollIntervalHook(managed, effectivePollInterval(managed))`.
    fn poll(&self, mg: &M) -> Duration {
        (self.poll_interval_hook)(mg, self.effective_poll_interval(mg))
    }

    /// Go logs `time.Now().Add(reconcileAfter)` as `requeue-after`.
    fn requeue_after(&self, poll: Duration) -> String {
        let now = self.clock.now();
        logging::rfc3339(now.checked_add(poll).unwrap_or(now))
    }
}

/// One reconcile after the pause check: the per-run state Go keeps in locals of `Reconcile`.
/// Each phase either continues or ends the run with its branch's result.
struct Run<'a, M: Managed> {
    r: &'a Reconciler<M>,
    mg: M,
    /// Rebuilt with the new external name after a successful Create (X7).
    record: Arc<dyn Recorder>,
    policy: PolicyResolver<'a>,
    /// S1's pending reconcile-request token.
    token: Option<String>,
    deadlines: Deadlines,
}

type Step<T = ()> = ControlFlow<Result<Outcome>, T>;

/// Unwraps a phase's `Continue` or returns its branch result.
macro_rules! step {
    ($phase:expr) => {
        match $phase {
            ControlFlow::Continue(v) => v,
            ControlFlow::Break(result) => return result,
        }
    };
}

impl<M: Managed> Run<'_, M> {
    async fn reconcile(mut self) -> Result<Outcome> {
        step!(self.validate_policy().await);
        if meta::was_deleted(&self.mg) && !self.policy.should_delete() {
            self.record_deletion_timestamp();
            return self.unpublish_and_remove_finalizer(&ConnectionDetails::new()).await;
        }
        step!(self.initialize().await);
        // S3
        if !meta::was_deleted(&self.mg) {
            step!(self.resolve_references().await);
        }
        let external = step!(self.connect().await);
        let result = self.with_external(&*external).await;
        self.disconnect(&*external).await;
        result
    }

    /// B18-B48: everything between a successful connect and the deferred disconnect.
    async fn with_external(&mut self, ext: &dyn DynExternal<M>) -> Result<Outcome> {
        let obs = step!(self.observe(ext).await);
        // R:1229. Taken once, before any external operation or metadata update, and used by
        // every change log entry of this run.
        let pre_op = self.mg.clone();
        if meta::was_deleted(&self.mg) {
            self.record_deletion_timestamp();
            return self.delete_external(ext, &obs, &pre_op).await;
        }
        step!(self.publish_observed_and_add_finalizer(&obs).await);
        if !obs.resource_exists && self.policy.should_create() {
            return self.create_external(ext, &pre_op).await;
        }
        step!(self.late_initialize(&obs).await);
        if obs.resource_up_to_date {
            return self.up_to_date().await;
        }
        if !obs.resource_exists {
            // B44 (R:1517-1523)
            return self.succeed_and_poll("Resource does not exist so skip update").await;
        }
        // S6
        if !obs.diff.is_empty() {
            tracing::debug!(diff = %obs.diff, "External resource differs from desired state");
        }
        if !self.policy.should_update() {
            // B45 (R:1531-1536)
            return self
                .succeed_and_poll("Skipping update due to managementPolicies. Reconciliation succeeded")
                .await;
        }
        self.update_external(ext, &pre_op).await
    }

    /// X3 (`R:999-1006`): stamp the pending token, PUT status, return `outcome` or the wrapped
    /// status error (X4). Every branch that writes status ends here.
    async fn finish(&mut self, outcome: Outcome) -> Result<Outcome> {
        if let Some(token) = &self.token {
            self.mg.set_last_handled_reconcile_at(token.clone());
        }
        bounded(self.deadlines.kubernetes, self.r.client.update_status(&mut self.mg))
            .await
            .map(|()| outcome)
            .wrap_err(ERR_UPDATE_MANAGED_STATUS)
    }

    /// Go's order at the end of a branch: event, conditions, status write.
    async fn report(&mut self, event: Option<Event>, conditions: Vec<Condition>, outcome: Outcome) -> Result<Outcome> {
        if let Some(event) = event {
            self.record.event(&self.mg, event);
        }
        mark_conditions(&mut self.mg, conditions);
        self.finish(outcome).await
    }

    /// The shape of every collaborator failure that has a conflict short-circuit: a conflict
    /// requeues with no event and no status write (B06, B08, B11, B14, B16, B18, B25, B27, B30,
    /// B32, B34, B37, B39); anything else is reported and requeued.
    async fn failed(&mut self, cause: &Error, event: Option<Event>, conditions: Vec<Condition>) -> Result<Outcome> {
        if cause.is_conflict() {
            return Ok(Outcome::Requeue);
        }
        self.report(event, conditions, Outcome::Requeue).await
    }

    async fn succeed_and_poll(&mut self, log: &str) -> Result<Outcome> {
        let poll = self.r.poll(&self.mg);
        tracing::debug!("requeue-after" = %self.r.requeue_after(poll), "{log}");
        self.report(None, vec![Condition::reconcile_success()], Outcome::RequeueAfter(poll))
            .await
    }

    /// Go rebinds its logger with `deletion-timestamp` on both delete paths (`R:1033`, `R:1232`).
    fn record_deletion_timestamp(&self) {
        if let Some(ts) = &self.mg.meta().deletion_timestamp {
            tracing::Span::current().record("deletion-timestamp", logging::rfc3339(ts.0));
        }
    }

    /// A failed send is logged and never changes the outcome (`R:1252-1254`).
    async fn log_change(
        &self,
        pre_op: &M,
        operation: OperationType,
        change_error: Option<&Error>,
        additional_details: AdditionalDetails,
    ) {
        let entry = ChangeLogEntry::new(pre_op, operation, change_error, additional_details, self.r.clock.now());
        let sent = match entry {
            Ok(entry) => bounded(self.deadlines.kubernetes, self.r.change_logger.log(entry)).await,
            Err(e) => Err(e),
        };
        if let Err(e) = sent {
            tracing::info!(error = %e, "{ERR_RECORD_CHANGE_LOG}");
        }
    }

    /// B05 (`R:1016-1026`). B04, the conflict arm at `R:1019-1020`, has no counterpart:
    /// `validate` builds its errors itself and none is a conflict.
    async fn validate_policy(&mut self) -> Step {
        let Err(message) = self.policy.validate() else {
            return ControlFlow::Continue(());
        };
        tracing::debug!("{message}");
        let e = Error::new(message);
        ControlFlow::Break(
            self.report(
                Some(Event::warning(reason::CANNOT_USE_INVALID_POLICY, &e)),
                vec![Condition::reconcile_error(&e)],
                Outcome::Done,
            )
            .await,
        )
    }

    /// The tail of both delete paths: B06-B10 (`R:1032-1080`, deleted and the policy forbids
    /// Delete, with empty details and no connect) and B25-B29 (`R:1281-1321`, the external
    /// resource is gone).
    async fn unpublish_and_remove_finalizer(&mut self, details: &ConnectionDetails) -> Result<Outcome> {
        let k8s = self.deadlines.kubernetes;
        // B06, B07 / B25, B26
        if let Err(e) = bounded(k8s, self.r.publisher.unpublish_connection(&self.mg, details)).await {
            tracing::debug!(error = %e, "Cannot unpublish connection details");
            return self
                .failed(
                    &e,
                    Some(Event::warning(reason::CANNOT_UNPUBLISH, &e)),
                    vec![Condition::deleting(), Condition::reconcile_error(&e)],
                )
                .await;
        }
        // B08, B09 / B27, B28: no event.
        if let Err(e) = bounded(k8s, self.r.finalizer.remove_finalizer(&mut self.mg)).await {
            tracing::debug!(error = %e, "Cannot remove managed resource finalizer");
            return self
                .failed(&e, None, vec![Condition::deleting(), Condition::reconcile_error(&e)])
                .await;
        }
        // B10 / B29: the object is gone, so no status write.
        self.r.metrics.record_deleted(&self.r.subject(&self.mg));
        tracing::debug!("Successfully deleted managed resource");
        Ok(Outcome::Done)
    }

    /// B11-B13 and S2 (`R:1083-1114`): the initializer chain, then the create-pending guard.
    async fn initialize(&mut self) -> Step {
        for initializer in &self.r.initializers {
            // B11, B12
            if let Err(e) = bounded(self.deadlines.kubernetes, initializer.initialize(&mut self.mg)).await {
                tracing::debug!(error = %e, "Cannot initialize managed resource");
                return ControlFlow::Break(
                    self.failed(
                        &e,
                        Some(Event::warning(reason::CANNOT_INITIALIZE, &e)),
                        vec![Condition::reconcile_error(&e)],
                    )
                    .await,
                );
            }
        }
        if meta::external_create_incomplete(&self.mg) {
            if !self.r.deterministic_external_name {
                // B13
                tracing::debug!("{ERR_CREATE_INCOMPLETE}");
                let e = Error::new(ERR_CREATE_INCOMPLETE);
                return ControlFlow::Break(
                    self.report(
                        Some(Event::warning(reason::CANNOT_INITIALIZE, &e)),
                        vec![Condition::creating(), Condition::reconcile_error(&e)],
                        Outcome::Done,
                    )
                    .await,
                );
            }
            // S2
            tracing::debug!("Cannot determine creation result, but proceeding due to deterministic external name");
        }
        ControlFlow::Continue(())
    }

    /// B14, B15 (`R:1125-1142`).
    async fn resolve_references(&mut self) -> Step {
        let resolved = bounded(
            self.deadlines.kubernetes,
            self.r.reference_resolver.resolve_references(&mut self.mg),
        )
        .await;
        let Err(e) = resolved else {
            return ControlFlow::Continue(());
        };
        tracing::debug!(error = %e, "Cannot resolve managed resource references");
        ControlFlow::Break(
            self.failed(
                &e,
                Some(Event::warning(reason::CANNOT_RESOLVE_REFS, &e)),
                vec![Condition::reconcile_error(&e)],
            )
            .await,
        )
    }

    /// B16, B17 (`R:1145-1162`). The event carries the raw error, the condition the wrapped one.
    async fn connect(&mut self) -> Step<Box<dyn DynExternal<M>>> {
        match bounded(self.deadlines.external, self.r.external.connect(&self.mg)).await {
            Ok(external) => ControlFlow::Continue(external),
            Err(e) => {
                tracing::debug!(error = %e, "Cannot connect to provider");
                ControlFlow::Break(
                    self.failed(
                        &e,
                        Some(Event::warning(reason::CANNOT_CONNECT, &e)),
                        vec![Condition::reconcile_error(&e.clone().wrap(ERR_RECONCILE_CONNECT))],
                    )
                    .await,
                )
            }
        }
    }

    /// X5, S7, S8 (`R:1164-1176`): Go's deferred disconnect, so it runs after the branch's status
    /// write. Connector first, then client; a failure only emits an event.
    async fn disconnect(&mut self, ext: &dyn DynExternal<M>) {
        if let Err(e) = bounded(self.deadlines.kubernetes, self.r.external.disconnect()).await {
            tracing::debug!(error = %e, "Cannot disconnect from provider");
            self.record
                .event(&self.mg, Event::warning(reason::CANNOT_DISCONNECT, &e));
        }
        if let Err(e) = bounded(self.deadlines.kubernetes, ext.disconnect()).await {
            tracing::debug!(error = %e, "Cannot disconnect from provider");
            self.record
                .event(&self.mg, Event::warning(reason::CANNOT_DISCONNECT, &e));
        }
    }

    /// B18-B21 (`R:1178-1223`).
    async fn observe(&mut self, ext: &dyn DynExternal<M>) -> Step<ExternalObservation> {
        let obs = match bounded(self.deadlines.external, ext.observe(&mut self.mg)).await {
            Ok(obs) => obs,
            // B18, B19
            Err(e) => {
                tracing::debug!(error = %e, "Cannot observe external resource");
                return ControlFlow::Break(
                    self.failed(
                        &e,
                        Some(Event::warning(reason::CANNOT_OBSERVE, &e)),
                        vec![Condition::reconcile_error(&e.clone().wrap(ERR_RECONCILE_OBSERVE))],
                    )
                    .await,
                );
            }
        };
        if obs.resource_exists {
            return ControlFlow::Continue(obs);
        }
        // B20: a stable state, so the poll interval and not the error backoff.
        if self.policy.should_only_observe() {
            let e = Error::new(ERR_EXTERNAL_RESOURCE_NOT_EXIST);
            let poll = self.r.poll(&self.mg);
            tracing::debug!("requeue-after" = %self.r.requeue_after(poll), "External resource does not exist");
            return ControlFlow::Break(
                self.report(
                    Some(Event::warning(reason::CANNOT_OBSERVE, &e)),
                    vec![Condition::reconcile_error(&e.clone().wrap(ERR_RECONCILE_OBSERVE))],
                    Outcome::RequeueAfter(poll),
                )
                .await,
            );
        }
        // B21: no status write.
        if meta::external_create_succeeded_during(&self.mg, self.r.timing.creation_grace, self.r.clock.now()) {
            tracing::debug!("{MSG_PENDING}");
            self.record.event(&self.mg, Event::normal(reason::PENDING, MSG_PENDING));
            return ControlFlow::Break(Ok(Outcome::Requeue));
        }
        ControlFlow::Continue(obs)
    }

    /// B22-B29 (`R:1231-1321`).
    async fn delete_external(
        &mut self,
        ext: &dyn DynExternal<M>,
        obs: &ExternalObservation,
        pre_op: &M,
    ) -> Result<Outcome> {
        // B22: other controllers still hold the object, so keep the external resource.
        if self.mg.meta().finalizers.as_ref().is_some_and(|f| f.len() > 1) {
            tracing::debug!("Delay external deletion until all finalizers have been removed");
            return Ok(Outcome::Requeue);
        }
        if obs.resource_exists && self.policy.should_delete() {
            return match bounded(self.deadlines.external, ext.delete(&mut self.mg)).await {
                // B23: no conflict short-circuit.
                Err(e) => {
                    tracing::debug!(error = %e, "Cannot delete external resource");
                    self.log_change(pre_op, OperationType::Delete, Some(&e), AdditionalDetails::new())
                        .await;
                    self.report(
                        Some(Event::warning(reason::CANNOT_DELETE, &e)),
                        vec![
                            Condition::deleting(),
                            Condition::reconcile_error(&e.clone().wrap(ERR_RECONCILE_DELETE)),
                        ],
                        Outcome::Requeue,
                    )
                    .await
                }
                // B24: requeue to confirm the deletion; the finalizer stays.
                Ok(deletion) => {
                    tracing::debug!("{MSG_DELETED}");
                    self.log_change(pre_op, OperationType::Delete, None, deletion.additional_details)
                        .await;
                    self.report(
                        Some(Event::normal(reason::DELETED, MSG_DELETED)),
                        vec![Condition::deleting(), Condition::reconcile_success()],
                        Outcome::Requeue,
                    )
                    .await
                }
            };
        }
        self.unpublish_and_remove_finalizer(&obs.connection_details).await
    }

    /// B30-B33 (`R:1324-1353`).
    async fn publish_observed_and_add_finalizer(&mut self, obs: &ExternalObservation) -> Step {
        let k8s = self.deadlines.kubernetes;
        // B30, B31
        let published = bounded(
            k8s,
            self.r.publisher.publish_connection(&self.mg, &obs.connection_details),
        )
        .await;
        if let Err(e) = published {
            tracing::debug!(error = %e, "Cannot publish connection details");
            return ControlFlow::Break(
                self.failed(
                    &e,
                    Some(Event::warning(reason::CANNOT_PUBLISH, &e)),
                    vec![Condition::reconcile_error(&e)],
                )
                .await,
            );
        }
        // B32, B33: no event. The default finalizer PUTs the object, and the response replaces
        // `self.mg`, status included, so conditions Observe set are gone from here on.
        if let Err(e) = bounded(k8s, self.r.finalizer.add_finalizer(&mut self.mg)).await {
            tracing::debug!(error = %e, "Cannot add finalizer");
            return ControlFlow::Break(self.failed(&e, None, vec![Condition::reconcile_error(&e)]).await);
        }
        ControlFlow::Continue(())
    }

    /// B34-B41 (`R:1355-1476`).
    async fn create_external(&mut self, ext: &dyn DynExternal<M>, pre_op: &M) -> Result<Outcome> {
        let k8s = self.deadlines.kubernetes;

        // B34, B35. A plain update, not the retrying updater: a 409 on a stale object must
        // stop the create.
        meta::set_external_create_pending(&mut self.mg, self.r.clock.now());
        if let Err(e) = bounded(k8s, self.r.client.update(&mut self.mg)).await {
            tracing::debug!(error = %e, "{ERR_UPDATE_MANAGED}");
            let wrapped = e.clone().wrap(ERR_UPDATE_MANAGED);
            return self
                .failed(
                    &e,
                    Some(Event::warning(reason::CANNOT_UPDATE_MANAGED, &wrapped)),
                    vec![Condition::creating(), Condition::reconcile_error(&wrapped)],
                )
                .await;
        }

        let creation = match bounded(self.deadlines.external, ext.create(&mut self.mg)).await {
            Ok(creation) => creation,
            // B36: no conflict short-circuit, but a conflict emits no event.
            Err(e) => {
                tracing::debug!(error = %e, "Cannot create external resource");
                if !e.is_conflict() {
                    self.record.event(&self.mg, Event::warning(reason::CANNOT_CREATE, &e));
                }
                meta::set_external_create_failed(&mut self.mg, self.r.clock.now());
                // S4: the condition below reports why the create failed, not this.
                let annotated = bounded(
                    k8s,
                    self.r.critical_annotations.update_critical_annotations(&mut self.mg),
                )
                .await;
                if let Err(update_error) = annotated {
                    tracing::debug!(error = %update_error, "{ERR_UPDATE_MANAGED_ANNOTATIONS}");
                    self.record.event(
                        &self.mg,
                        Event::warning(
                            reason::CANNOT_UPDATE_MANAGED,
                            &update_error.wrap(ERR_UPDATE_MANAGED_ANNOTATIONS),
                        ),
                    );
                }
                self.log_change(pre_op, OperationType::Create, Some(&e), AdditionalDetails::new())
                    .await;
                return self
                    .report(
                        None,
                        vec![
                            Condition::creating(),
                            Condition::reconcile_error(&e.wrap(ERR_RECONCILE_CREATE)),
                        ],
                        Outcome::Requeue,
                    )
                    .await;
            }
        };

        // X7: Create may have set the external name.
        tracing::Span::current().record("external-name", meta::external_name(&self.mg));
        self.record = self
            .r
            .recorder
            .with_annotations(&[("external-name", meta::external_name(&self.mg))]);
        self.log_change(pre_op, OperationType::Create, None, creation.additional_details)
            .await;

        // B37, B38. The updater's response replaces everything Create changed except
        // annotations.
        meta::set_external_create_succeeded(&mut self.mg, self.r.clock.now());
        let annotated = bounded(
            k8s,
            self.r.critical_annotations.update_critical_annotations(&mut self.mg),
        )
        .await;
        if let Err(e) = annotated {
            tracing::debug!(error = %e, "{ERR_UPDATE_MANAGED_ANNOTATIONS}");
            let wrapped = e.clone().wrap(ERR_UPDATE_MANAGED_ANNOTATIONS);
            return self
                .failed(
                    &e,
                    Some(Event::warning(reason::CANNOT_UPDATE_MANAGED, &wrapped)),
                    vec![Condition::creating(), Condition::reconcile_error(&wrapped)],
                )
                .await;
        }

        // B39, B40
        let published = bounded(
            k8s,
            self.r
                .publisher
                .publish_connection(&self.mg, &creation.connection_details),
        )
        .await;
        if let Err(e) = published {
            tracing::debug!(error = %e, "Cannot publish connection details");
            return self
                .failed(
                    &e,
                    Some(Event::warning(reason::CANNOT_PUBLISH, &e)),
                    vec![Condition::creating(), Condition::reconcile_error(&e)],
                )
                .await;
        }

        // B41: requeue to observe whether the new resource is ready.
        tracing::debug!("{MSG_CREATED}");
        self.report(
            Some(Event::normal(reason::CREATED, MSG_CREATED)),
            vec![Condition::creating(), Condition::reconcile_success()],
            Outcome::Requeue,
        )
        .await
    }

    /// S5, B42 (`R:1479-1494`). The update's response replaces `self.mg`, so status set by
    /// Observe is reset to the server's. No conflict short-circuit; the event carries the raw
    /// error.
    async fn late_initialize(&mut self, obs: &ExternalObservation) -> Step {
        if !(obs.resource_late_initialized && self.policy.should_late_initialize()) {
            return ControlFlow::Continue(());
        }
        let updated = bounded(self.deadlines.kubernetes, self.r.client.update(&mut self.mg)).await;
        let Err(e) = updated else {
            return ControlFlow::Continue(());
        };
        tracing::debug!(error = %e, "{ERR_UPDATE_MANAGED}");
        ControlFlow::Break(
            self.report(
                Some(Event::warning(reason::CANNOT_UPDATE_MANAGED, &e)),
                vec![Condition::reconcile_error(&e.clone().wrap(ERR_UPDATE_MANAGED))],
                Outcome::Requeue,
            )
            .await,
        )
    }

    /// B43 (`R:1496-1514`).
    async fn up_to_date(&mut self) -> Result<Outcome> {
        let poll = self.r.poll(&self.mg);
        tracing::debug!("requeue-after" = %self.r.requeue_after(poll), "External resource is up to date");
        mark_conditions(&mut self.mg, [Condition::reconcile_success()]);
        self.r.metrics.record_first_time_ready(&self.r.subject(&self.mg));
        self.r
            .metrics
            .record_unchanged(self.mg.meta().name.as_deref().unwrap_or_default());
        self.finish(Outcome::RequeueAfter(poll)).await
    }

    /// B46-B48 (`R:1539-1586`). Neither failure has a conflict short-circuit.
    async fn update_external(&mut self, ext: &dyn DynExternal<M>, pre_op: &M) -> Result<Outcome> {
        let update = match bounded(self.deadlines.external, ext.update(&mut self.mg)).await {
            Ok(update) => update,
            // B46
            Err(e) => {
                tracing::debug!("Cannot update external resource");
                self.log_change(pre_op, OperationType::Update, Some(&e), AdditionalDetails::new())
                    .await;
                return self
                    .report(
                        Some(Event::warning(reason::CANNOT_UPDATE, &e)),
                        vec![Condition::reconcile_error(&e.clone().wrap(ERR_RECONCILE_UPDATE))],
                        Outcome::Requeue,
                    )
                    .await;
            }
        };
        self.r.metrics.record_drift(&self.r.subject(&self.mg));
        self.log_change(pre_op, OperationType::Update, None, update.additional_details)
            .await;

        // B47
        let published = bounded(
            self.deadlines.kubernetes,
            self.r
                .publisher
                .publish_connection(&self.mg, &update.connection_details),
        )
        .await;
        if let Err(e) = published {
            tracing::debug!(error = %e, "Cannot publish connection details");
            return self
                .report(
                    Some(Event::warning(reason::CANNOT_PUBLISH, &e)),
                    vec![Condition::reconcile_error(&e)],
                    Outcome::Requeue,
                )
                .await;
        }

        // B48
        let poll = self.r.poll(&self.mg);
        tracing::debug!("requeue-after" = %self.r.requeue_after(poll), "{MSG_UPDATED}");
        self.report(
            Some(Event::normal(reason::UPDATED, MSG_UPDATED)),
            vec![Condition::reconcile_success()],
            Outcome::RequeueAfter(poll),
        )
        .await
    }
}
