//! `pkg/controller` (the options every setup function receives) and what controller-runtime's
//! builder, controller and workqueue do, on stable kube-runtime.
//!
//! Flow: the manager's informer for each watched kind yields events with the old object
//! attached. [`Predicate`]s and [`EventHandler`]s turn them into a plain stream of [`Request`]s.
//! The private driver runs that stream through a work queue with client-go semantics and
//! `max_concurrent_reconciles` workers, and feeds each [`Outcome`](crate::reconciler::Outcome)
//! back to the queue. [`new_managed_by`] is the only way to register a controller.

mod driver;
mod workqueue;

use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::errors::Result;
use crossplane_core::resource::{Kind, Object, ObjectType};
use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use kube::core::GroupVersionKind;
use tokio_util::sync::CancellationToken;

use crate::cache::InformerEvent;
use crate::feature::Features;
use crate::gate::Gate;
use crate::manager::{CACHE_SYNC_TIMEOUT, Manager, Runnable};
use crate::ratelimiter::{ItemExponentialBackoff, RateLimiter};
use crate::reconciler::managed::{ChangeLogger, MetricRecorder};
use crate::reconciler::{Reconcile, Request};
use crate::statemetrics::MrStateMetrics;

/// Go `controller.Options`. Cloned into every setup function and into gated callbacks.
#[derive(Clone)]
pub struct Options {
    pub max_concurrent_reconciles: usize,
    pub poll_interval: Duration,
    pub global_rate_limiter: Arc<dyn RateLimiter>,
    pub features: Features,
    pub gate: Arc<Gate<GroupVersionKind>>,
    pub metric_options: Option<MetricOptions>,
    pub change_log_options: Option<ChangeLogOptions>,
}

#[derive(Clone)]
pub struct MetricOptions {
    pub poll_state_metric_interval: Duration,
    pub mr_metrics: Arc<dyn MetricRecorder>,
    pub mr_state_metrics: Option<MrStateMetrics>,
}

#[derive(Clone)]
pub struct ChangeLogOptions {
    pub change_logger: Arc<dyn ChangeLogger>,
}

/// controller-runtime `controller.Options`, the part Crossplane sets.
#[derive(Clone)]
pub struct ControllerOptions {
    pub max_concurrent_reconciles: usize,
    /// The per-item limiter the work queue applies on `Outcome::Requeue` and on errors.
    pub rate_limiter: Arc<dyn RateLimiter>,
    pub recover_panic: bool,
}

impl Options {
    /// Go `Options.ForControllerRuntime()` (`pkg/controller/options.go:83-91`).
    pub fn for_controller(&self) -> ControllerOptions {
        ControllerOptions {
            max_concurrent_reconciles: self.max_concurrent_reconciles,
            rate_limiter: crate::ratelimiter::new_controller(),
            recover_panic: true,
        }
    }
}

impl Default for ControllerOptions {
    /// controller-runtime's own defaults, used when `with_options` is not called: one worker, a
    /// 5ms to 1000s per-item backoff (`pkg/controller/controller.go:252-258`), and panics
    /// recovered, because an unset `RecoverPanic` counts as true
    /// (`pkg/internal/controller/controller.go:199`).
    fn default() -> Self {
        Self {
            max_concurrent_reconciles: 1,
            rate_limiter: Arc::new(ItemExponentialBackoff::new(
                Duration::from_millis(5),
                Duration::from_secs(1000),
            )),
            recover_panic: true,
        }
    }
}

/// controller-runtime `predicate.Predicate`. It receives whole objects, old and new on update,
/// as Go's does; kube's own predicates compare against a hash cache instead.
pub trait Predicate: Send + Sync + 'static {
    fn create(&self, _obj: &dyn Object) -> bool {
        true
    }
    fn update(&self, _old: &dyn Object, _new: &dyn Object) -> bool {
        true
    }
    fn delete(&self, _obj: &dyn Object) -> bool {
        true
    }
}

/// An informer event on a watched kind, as an [`EventHandler`] sees it.
#[derive(Debug)]
pub enum WatchEvent<'a, K> {
    Create(&'a K),
    Update { old: &'a K, new: &'a K },
    Delete(&'a K),
}

/// controller-runtime `handler.EventHandler`: map an event on a watched kind to requests for the
/// controlled kind. Typed by the watched kind, so a handler cannot be paired with a kind it does
/// not read.
pub trait EventHandler<U>: Send + Sync + 'static {
    fn requests(&self, event: &WatchEvent<'_, U>) -> Vec<Request>;
}

/// controller-runtime `handler.EnqueueRequestForObject`, which `for_kind` installs.
struct EnqueueRequestForObject;

impl<K: Kind> EventHandler<K> for EnqueueRequestForObject {
    fn requests(&self, event: &WatchEvent<'_, K>) -> Vec<Request> {
        let obj: &dyn Object = match event {
            WatchEvent::Create(o) | WatchEvent::Delete(o) => *o,
            WatchEvent::Update { new, .. } => *new,
        };
        vec![obj.key()]
    }
}

/// One watch of a controller: a kind and the erased form of its typed handler.
struct Source {
    object_type: ObjectType,
    requests: Arc<dyn Fn(&InformerEvent) -> Vec<Request> + Send + Sync>,
}

impl Source {
    /// The one place a watched object is downcast back to the handler's kind.
    fn new<U: Kind>(handler: impl EventHandler<U>) -> Self {
        Self {
            object_type: ObjectType::of::<U>(),
            requests: Arc::new(move |event| {
                fn typed<U: Kind>(o: &Arc<dyn Object>) -> &U {
                    (**o)
                        .downcast_ref::<U>()
                        .unwrap_or_else(|| panic!("the cache sent {o:?} on a watch of {:?}", ObjectType::of::<U>()))
                }
                let event = match event {
                    InformerEvent::Create(o) => WatchEvent::Create(typed::<U>(o)),
                    InformerEvent::Update { old, new } => WatchEvent::Update {
                        old: typed::<U>(old),
                        new: typed::<U>(new),
                    },
                    InformerEvent::Delete(o) => WatchEvent::Delete(typed::<U>(o)),
                };
                handler.requests(&event)
            }),
        }
    }
}

/// Go `ctrl.NewControllerManagedBy(mgr)`.
pub fn new_managed_by(mgr: &Manager) -> Builder<()> {
    Builder {
        mgr: mgr.clone(),
        name: None,
        options: ControllerOptions::default(),
        filters: Vec::new(),
        sources: Vec::new(),
        _kind: PhantomData,
    }
}

/// `K` is the `For` kind. It is `()` until [`Builder::for_kind`] is called, so `complete` cannot
/// be reached without one.
pub struct Builder<K> {
    mgr: Manager,
    name: Option<String>,
    options: ControllerOptions,
    filters: Vec<Arc<dyn Predicate>>,
    sources: Vec<Source>,
    _kind: PhantomData<fn() -> K>,
}

impl<K> Builder<K> {
    pub fn named(mut self, name: &str) -> Self {
        self.name = Some(name.to_owned());
        self
    }

    pub fn with_options(mut self, options: ControllerOptions) -> Self {
        self.options = options;
        self
    }

    /// Go `WithEventFilter`: applies to every watch of this controller.
    pub fn with_event_filter(mut self, predicate: impl Predicate) -> Self {
        self.filters.push(Arc::new(predicate));
        self
    }
}

impl Builder<()> {
    pub fn for_kind<K: Kind>(mut self) -> Builder<K> {
        self.sources.push(Source::new::<K>(EnqueueRequestForObject));
        Builder {
            mgr: self.mgr,
            name: self.name,
            options: self.options,
            filters: self.filters,
            sources: self.sources,
            _kind: PhantomData,
        }
    }
}

impl<K: Kind> Builder<K> {
    /// Go `Watches(&U{}, handler)`.
    pub fn watches<U: Kind>(mut self, handler: impl EventHandler<U>) -> Self {
        self.sources.push(Source::new::<U>(handler));
        self
    }

    /// Go `Complete(r)`. The controller starts with the manager, or at once if the manager is
    /// already running (the safe-start gate path).
    pub fn complete(self, reconciler: impl Reconcile) -> Result<()> {
        let name = self.name.unwrap_or_else(|| K::kind(&()).to_lowercase());
        self.mgr.add(Controller {
            mgr: self.mgr.clone(),
            name,
            options: self.options,
            filters: self.filters,
            sources: self.sources,
            reconciler: Arc::new(reconciler),
        })
    }
}

struct Controller {
    mgr: Manager,
    name: String,
    options: ControllerOptions,
    filters: Vec<Arc<dyn Predicate>>,
    sources: Vec<Source>,
    reconciler: Arc<dyn Reconcile>,
}

impl Controller {
    /// Every source's informer events, filtered by the predicates and mapped by its handler,
    /// merged into one stream of keys. This is the only kube-facing half of a controller.
    /// controller-runtime ANDs a watch's predicates, and `WithEventFilter`'s apply to every watch
    /// (`pkg/builder/controller.go`).
    fn keys(&self) -> BoxStream<'static, Request> {
        let cache = self.mgr.cache();
        let streams = self.sources.iter().map(|source| {
            let filters = self.filters.clone();
            let requests = source.requests.clone();
            let events = cache.subscribe(&source.object_type);
            futures::stream::unfold(
                events,
                |mut events| async move { events.recv().await.map(|e| (e, events)) },
            )
            .filter(move |event| std::future::ready(passes(&filters, event)))
            .flat_map(move |event| futures::stream::iter(requests(&event)))
            .boxed()
        });
        futures::stream::select_all(streams).boxed()
    }
}

fn passes(filters: &[Arc<dyn Predicate>], event: &InformerEvent) -> bool {
    filters.iter().all(|p| match event {
        InformerEvent::Create(o) => p.create(&**o),
        InformerEvent::Update { old, new } => p.update(&**old, &**new),
        InformerEvent::Delete(o) => p.delete(&**o),
    })
}

impl Runnable for Controller {
    /// controller-runtime `Controller.Start`: start the watches, wait for their caches to sync,
    /// then run the workers (`pkg/internal/controller/controller.go:340-400`).
    fn start(self: Box<Self>, shutdown: CancellationToken) -> BoxFuture<'static, Result<()>> {
        let keys = self.keys();
        let cache = self.mgr.cache();
        let kinds: Vec<_> = self.sources.iter().map(|s| s.object_type.clone()).collect();
        Box::pin(async move {
            for kind in &kinds {
                tokio::select! {
                    () = shutdown.cancelled() => return Ok(()),
                    synced = cache.wait_for_sync(kind, CACHE_SYNC_TIMEOUT) => synced?,
                }
            }
            driver::run(self.name, keys, self.reconciler, self.options, shutdown).await
        })
    }
}
