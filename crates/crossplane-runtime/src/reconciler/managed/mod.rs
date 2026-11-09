//! `pkg/reconciler/managed`: the managed resource reconciler.
//!
//! [`Reconciler<M>`] is generic over the provider's kind, so collaborators and the external
//! client receive `&mut MyType`, not an interface to assert. [`ReconcilerBuilder`] is Go's
//! `ReconcilerOption` list: every collaborator a Go table replaces is a builder method, and
//! `build()` with no options gives Go's defaults.
//!
//! The 48 return branches of `docs/phase0/02-parity-spec.md` section 1.3 live in [`Run`], one
//! method per phase, each named with the branch IDs it owns. `R:n` and `A:n` are that
//! document's aliases for `reconciler.go:n` and `api.go:n` in the Go package.

mod changelog;
mod collaborators;
mod external;
mod metrics;
pub mod policy;

#[cfg(test)]
mod reconciler_modern_test;

use std::borrow::Cow;
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::Client;
use crossplane_core::errors::{Error, Result, ResultExt};
use crossplane_core::meta;
use crossplane_core::resource::{Kind, Managed, ObjectType, mark_conditions};
use crossplane_core::v2::Condition;
use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::time::Instant;

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
pub const ERR_UPDATE_MANAGED_STATUS: &str = "cannot update managed resource status";
pub const ERR_RESOLVE_REFERENCES: &str = "cannot resolve references";
pub const ERR_UPDATE_CRITICAL_ANNOTATIONS: &str = "cannot update critical annotations";
pub const ERR_CREATE_OR_UPDATE_SECRET: &str = "cannot create or update connection secret";

pub const MSG_RECONCILIATION_PAUSED: &str =
    "Reconciliation is paused either through the `spec.managementPolicies` or the pause annotation";

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
        todo!("hook: d + (rand - 0.5) * 2 * {jitter:?}")
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
    /// Go keeps this unexported (`withLocalConnectionPublishers`); so does Rust.
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

impl<M: Managed> Reconcile for Reconciler<M> {
    /// Two deadlines, as Go's two contexts (X2): the external calls share one that ends at
    /// `timing.timeout`; the whole run ends `timing.grace` later.
    ///
    /// OPEN (docs/design/core.md): at the outer deadline this drops the whole run and returns a
    /// deadline error. Go lets each later Kubernetes call fail on the expired context and
    /// carries on through the remaining branches.
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        async move {
            let start = Instant::now();
            let run = self.run(req, start + self.timing.timeout);
            let result = match tokio::time::timeout_at(start + self.timing.timeout + self.timing.grace, run).await {
                Ok(result) => result,
                Err(_) => Err(Error::deadline_exceeded()),
            };
            silently_requeue_on_conflict(result)
        }
        .boxed()
    }
}

impl<M: Managed> Reconciler<M> {
    async fn run(&self, req: &Request, external_deadline: Instant) -> Result<Outcome> {
        let mut mg = match self.client.get_as::<M>(req).await {
            Ok(mg) => mg,
            Err(e) if e.is_not_found() => return Ok(Outcome::Done),
            Err(e) => return Err(e.wrap(ERR_GET_MANAGED)),
        };
        self.metrics.record_first_time_reconciled(&self.subject(&mg));
        let record = self
            .recorder
            .with_annotations(&[("external-name", meta::external_name(&mg))]);
        let policy = PolicyResolver::new(
            self.management_policies,
            mg.management_policies(),
            &self.supported_policies,
        );

        // B03
        if meta::is_paused(&mg) || policy.is_paused() {
            record.event(
                &mg,
                Event::normal(reason::RECONCILIATION_PAUSED, MSG_RECONCILIATION_PAUSED)
                    .with_annotation("annotation", meta::ANNOTATION_KEY_RECONCILIATION_PAUSED),
            );
            mark_conditions(&mut mg, [Condition::reconcile_paused()]);
            return self
                .client
                .update_status(&mut mg)
                .await
                .map(|()| Outcome::Done)
                .wrap_err(ERR_UPDATE_MANAGED_STATUS);
        }

        // S1
        let token = match meta::reconcile_request(&mg) {
            Some(t) if M::TRACKS_RECONCILE_REQUESTS && last_handled(&mg) != Some(t) => {
                record.event(
                    &mg,
                    Event::normal(reason::RECONCILE_REQUEST_HANDLED, "Handling reconcile request")
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
            external_deadline,
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

    /// Go `effectivePollInterval` then the hook (`R:867-875`, 1.5).
    fn poll(&self, mg: &M) -> Duration {
        let base = match meta::poll_interval(mg) {
            Some(d) => d.max(self.timing.min_poll_interval),
            None => self.timing.poll_interval,
        };
        (self.poll_interval_hook)(mg, base)
    }
}

fn last_handled<M: Managed>(mg: &M) -> Option<&str> {
    mg.managed_status()
        .map(|s| s.observed.last_handled_reconcile_at.as_str())
}

/// One reconcile after the pause check: the per-run state Go keeps in locals of `Reconcile`.
/// Each phase either continues or ends the run with the branch's result.
struct Run<'a, M: Managed> {
    r: &'a Reconciler<M>,
    mg: M,
    record: Arc<dyn Recorder>,
    policy: PolicyResolver<'a>,
    token: Option<String>,
    external_deadline: Instant,
}

type Step = ControlFlow<Result<Outcome>>;

impl<M: Managed> Run<'_, M> {
    async fn reconcile(mut self) -> Result<Outcome> {
        if let ControlFlow::Break(r) = self.validate_policy().await {
            return r;
        }
        if meta::was_deleted(&self.mg) && !self.policy.should_delete() {
            return self.delete_without_external().await;
        }
        if let ControlFlow::Break(r) = self.initialize().await {
            return r;
        }
        if let ControlFlow::Break(r) = self.resolve_references().await {
            return r;
        }
        let external = match self.connect().await {
            Ok(e) => e,
            Err(r) => return r,
        };
        let r = self.with_external(&*external).await;
        self.disconnect(&*external).await;
        r
    }

    /// B18-B48: everything after a successful connect.
    async fn with_external(&mut self, ext: &dyn DynExternal<M>) -> Result<Outcome> {
        let obs = match self.observe(ext).await {
            Ok(o) => o,
            Err(r) => return r,
        };
        if meta::was_deleted(&self.mg) {
            return self.delete_external(ext, &obs).await;
        }
        if let ControlFlow::Break(r) = self.publish_observed_and_finalize(&obs).await {
            return r;
        }
        if !obs.resource_exists && self.policy.should_create() {
            return self.create_external(ext).await;
        }
        if let ControlFlow::Break(r) = self.late_initialize(&obs).await {
            return r;
        }
        if obs.resource_up_to_date {
            return self.up_to_date().await;
        }
        if !obs.resource_exists || !self.policy.should_update() {
            // B44, B45
            mark_conditions(&mut self.mg, [Condition::reconcile_success()]);
            let poll = self.r.poll(&self.mg);
            return self.finish(Outcome::RequeueAfter(poll)).await;
        }
        self.update_external(ext).await
    }

    /// X3: stamp the pending token, PUT status, return `outcome` or the wrapped status error.
    /// Every branch that writes status ends here.
    async fn finish(&mut self, outcome: Outcome) -> Result<Outcome> {
        if let Some(t) = &self.token {
            self.mg.managed_status_mut().observed.last_handled_reconcile_at = t.clone();
        }
        self.r
            .client
            .update_status(&mut self.mg)
            .await
            .map(|()| outcome)
            .wrap_err(ERR_UPDATE_MANAGED_STATUS)
    }

    /// Mark `conds`, emit `ev` if any, then [`Run::finish`].
    async fn fail(&mut self, conds: Vec<Condition>, ev: Option<Event>, outcome: Outcome) -> Result<Outcome> {
        if let Some(ev) = ev {
            self.record.event(&self.mg, ev);
        }
        mark_conditions(&mut self.mg, conds);
        self.finish(outcome).await
    }

    /// Bound an external call by the shared external deadline.
    async fn external<T>(&self, f: impl Future<Output = Result<T>>) -> Result<T> {
        match tokio::time::timeout_at(self.external_deadline, f).await {
            Ok(result) => result,
            Err(_) => Err(Error::deadline_exceeded()),
        }
    }

    /// B04, B05.
    async fn validate_policy(&mut self) -> Step {
        match self.policy.validate() {
            Ok(()) => ControlFlow::Continue(()),
            Err(m) => {
                let e = Error::new(m);
                let conds = vec![Condition::reconcile_error(&e)];
                ControlFlow::Break(
                    self.fail(
                        conds,
                        Some(Event::warning(reason::CANNOT_USE_INVALID_POLICY, &e)),
                        Outcome::Done,
                    )
                    .await,
                )
            }
        }
    }

    /// B06-B10: deleted and the policy forbids Delete. Unpublish with no details, remove the
    /// finalizer, never connect.
    async fn delete_without_external(&mut self) -> Result<Outcome> {
        todo!("R:1032-1080")
    }

    /// B11-B13 and S2: the initializer chain, then the create-pending guard.
    async fn initialize(&mut self) -> Step {
        for i in &self.r.initializers {
            if let Err(e) = i.initialize(&mut self.mg).await {
                if e.is_conflict() {
                    return ControlFlow::Break(Ok(Outcome::Requeue));
                }
                let conds = vec![Condition::reconcile_error(&e)];
                return ControlFlow::Break(
                    self.fail(
                        conds,
                        Some(Event::warning(reason::CANNOT_INITIALIZE, &e)),
                        Outcome::Requeue,
                    )
                    .await,
                );
            }
        }
        if meta::external_create_incomplete(&self.mg) && !self.r.deterministic_external_name {
            let e = Error::new(ERR_CREATE_INCOMPLETE);
            let conds = vec![Condition::creating(), Condition::reconcile_error(&e)];
            return ControlFlow::Break(
                self.fail(
                    conds,
                    Some(Event::warning(reason::CANNOT_INITIALIZE, &e)),
                    Outcome::Done,
                )
                .await,
            );
        }
        ControlFlow::Continue(())
    }

    /// B14, B15 and S3 (skipped when deleted).
    async fn resolve_references(&mut self) -> Step {
        todo!("R:1125-1141, error wrapped in ERR_RESOLVE_REFERENCES by the resolver")
    }

    /// B16, B17. The event carries the raw error, the condition the wrapped one.
    async fn connect(&mut self) -> Result<Box<dyn DynExternal<M>>, Result<Outcome>> {
        let connected = self.external(self.r.external.connect(&self.mg)).await;
        match connected {
            Ok(c) => Ok(c),
            Err(e) => Err(self
                .fail(
                    vec![Condition::reconcile_error(&e.clone().wrap(ERR_RECONCILE_CONNECT))],
                    Some(Event::warning(reason::CANNOT_CONNECT, &e)),
                    Outcome::Requeue,
                )
                .await),
        }
    }

    /// X5: connector first, then client; failures only emit events.
    async fn disconnect(&mut self, ext: &dyn DynExternal<M>) {
        if let Err(e) = self.r.external.disconnect().await {
            self.record
                .event(&self.mg, Event::warning(reason::CANNOT_DISCONNECT, &e));
        }
        if let Err(e) = ext.disconnect().await {
            self.record
                .event(&self.mg, Event::warning(reason::CANNOT_DISCONNECT, &e));
        }
    }

    /// B18-B21.
    async fn observe(&mut self, ext: &dyn DynExternal<M>) -> Result<ExternalObservation, Result<Outcome>> {
        let _ = (ext, self.r.timing.creation_grace, &self.r.clock);
        todo!("R:1178-1222: observe under external deadline; observe-only absent -> B20; grace period -> B21")
    }

    /// B22-B29.
    async fn delete_external(&mut self, ext: &dyn DynExternal<M>, obs: &ExternalObservation) -> Result<Outcome> {
        let _ = (ext, obs, &self.r.finalizer, &self.r.change_logger);
        todo!("R:1229-1321")
    }

    /// B30-B33: publish observed details, add the finalizer.
    async fn publish_observed_and_finalize(&mut self, obs: &ExternalObservation) -> Step {
        let _ = (obs, &self.r.publisher);
        todo!("R:1324-1352")
    }

    /// B34-B41: pending annotation PUT, Create, critical annotations, publish. Rebuilds the
    /// recorder with the new external name after Create (X7).
    async fn create_external(&mut self, ext: &dyn DynExternal<M>) -> Result<Outcome> {
        let _ = (
            ext,
            &self.r.critical_annotations,
            Cow::<str>::Borrowed(ERR_UPDATE_MANAGED_ANNOTATIONS),
        );
        todo!("R:1355-1476")
    }

    /// S5, B42.
    async fn late_initialize(&mut self, obs: &ExternalObservation) -> Step {
        let _ = obs;
        // TODO (R:1479-1494): the Update response overwrites the in-memory object, status
        // included, as in Go. OPEN (docs/design/core.md): no ported case exercises that yet.
        todo!()
    }

    /// B43.
    async fn up_to_date(&mut self) -> Result<Outcome> {
        let _ = &self.r.metrics;
        todo!("R:1496-1514: record_first_time_ready, record_unchanged, success, RequeueAfter(poll)")
    }

    /// B46-B48.
    async fn update_external(&mut self, ext: &dyn DynExternal<M>) -> Result<Outcome> {
        let _ = ext;
        todo!("R:1539-1586")
    }
}
