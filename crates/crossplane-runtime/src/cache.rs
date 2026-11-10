//! The informer cache: controller-runtime's `cache.Cache`, on stable kube-runtime.
//!
//! One informer per kind, started on the first read or subscription of that kind, as
//! controller-runtime's `Informers.Get` adds an informer on a miss (`pkg/cache/internal/informers.go:311-337`).
//! It keeps the latest copy of every object, serves cached reads to `KubeClient`, and hands each
//! subscribed controller the kind's events with the old object attached, so predicates compare old
//! and new as Go's do. A kind costs one LIST and one WATCH however many readers and controllers
//! use it.
//!
//! Three client-go behaviours are kept on purpose:
//!
//! - A relist (the watcher's `Init` .. `InitDone`) replaces the store atomically, then sends a
//!   `Delete` with the last known object for every object the relist no longer returns, then an
//!   `Update` or `Create` for each listed object in list order. That is client-go v0.36's
//!   `processReplacedAllInfo` (`tools/cache/controller.go:756-800`), whose deletions are
//!   `DeletedFinalStateUnknown` tombstones that controller-runtime unwraps to the last known
//!   object (`pkg/internal/source/event_handler.go`).
//! - A resync sends `Update` with old equal to new for every stored object
//!   (`tools/cache/controller.go:629-638`), every sync period with controller-runtime's 10%
//!   jitter (`informers.go:607-611`).
//! - Each subscriber has its own unbounded queue, as each client-go `processorListener` has an
//!   unbounded `pendingNotifications` buffer (`tools/cache/shared_informer.go:1223-1228`). A slow
//!   subscriber costs memory and never loses an event.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::client::ListOptions;
use crossplane_core::errors::{Error, Result, StatusReason};
use crossplane_core::resource::{Object, ObjectKey, ObjectType};
use futures::{Stream, StreamExt};
use kube::api::{Api, DynamicObject};
use kube::runtime::{WatchStreamExt, watcher};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

/// A resync is an `Update` whose `old` and `new` are the same object.
#[derive(Clone, Debug)]
pub(crate) enum InformerEvent {
    Create(Arc<dyn Object>),
    Update { old: Arc<dyn Object>, new: Arc<dyn Object> },
    Delete(Arc<dyn Object>),
}

/// A cheap handle. Clones share the informers.
#[derive(Clone)]
pub(crate) struct Cache {
    kube: kube::Client,
    /// Go `Cache.SyncPeriod`, the `--sync` flag. Zero turns resync off.
    sync_period: Duration,
    informers: Arc<Mutex<HashMap<ObjectType, Arc<Informer>>>>,
    stop: CancellationToken,
}

struct Informer {
    /// One lock for the store and the subscriber list, so a subscriber added while events flow
    /// sees each event exactly once: either in its replay or live.
    state: Mutex<Store>,
    synced: watch::Sender<bool>,
}

/// The store and its fan-out. Pure: the watch task feeds it events.
#[derive(Default)]
pub(crate) struct Store {
    /// Ordered by namespace and name, the order the API server lists in.
    objects: BTreeMap<ObjectKey, Arc<dyn Object>>,
    subscribers: Vec<mpsc::UnboundedSender<InformerEvent>>,
    /// Objects of a relist in progress, applied at `InitDone`.
    relist: Option<Vec<Arc<dyn Object>>>,
}

impl Store {
    /// Applies one watcher event. Returns true when it completed a list, so the store is synced.
    pub(crate) fn apply(&mut self, event: watcher::Event<Arc<dyn Object>>) -> bool {
        match event {
            watcher::Event::Init => {
                self.relist = Some(Vec::new());
                false
            }
            watcher::Event::InitApply(obj) => {
                self.relist.get_or_insert_with(Vec::new).push(obj);
                false
            }
            watcher::Event::InitDone => {
                let listed = self.relist.take().unwrap_or_default();
                self.replace(listed);
                true
            }
            watcher::Event::Apply(new) => {
                let event = match self.objects.insert(new.key(), new.clone()) {
                    Some(old) => InformerEvent::Update { old, new },
                    None => InformerEvent::Create(new),
                };
                self.send(event);
                false
            }
            watcher::Event::Delete(obj) => {
                let last = self.objects.remove(&obj.key()).unwrap_or(obj);
                self.send(InformerEvent::Delete(last));
                false
            }
        }
    }

    /// client-go `processReplacedAllInfo`.
    fn replace(&mut self, listed: Vec<Arc<dyn Object>>) {
        let old = std::mem::take(&mut self.objects);
        self.objects = listed.iter().map(|o| (o.key(), o.clone())).collect();
        let mut events: Vec<_> = old
            .iter()
            .filter(|(key, _)| !self.objects.contains_key(key))
            .map(|(_, last)| InformerEvent::Delete(last.clone()))
            .collect();
        events.extend(listed.into_iter().map(|new| match old.get(&new.key()) {
            Some(old) => InformerEvent::Update { old: old.clone(), new },
            None => InformerEvent::Create(new),
        }));
        for event in events {
            self.send(event);
        }
    }

    pub(crate) fn resync(&mut self) {
        let objects: Vec<_> = self.objects.values().cloned().collect();
        for obj in objects {
            self.send(InformerEvent::Update {
                old: obj.clone(),
                new: obj,
            });
        }
    }

    /// client-go `AddEventHandler` on a running informer: the objects already held first, as
    /// `Create`s, then live events.
    pub(crate) fn subscribe(&mut self) -> mpsc::UnboundedReceiver<InformerEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        for obj in self.objects.values() {
            let _ = tx.send(InformerEvent::Create(obj.clone()));
        }
        self.subscribers.push(tx);
        rx
    }

    fn send(&mut self, event: InformerEvent) {
        self.subscribers.retain(|s| s.send(event.clone()).is_ok());
    }

    fn get(&self, key: &ObjectKey) -> Option<Arc<dyn Object>> {
        self.objects.get(key).cloned()
    }

    fn list(&self, options: &ListOptions) -> Vec<Arc<dyn Object>> {
        self.objects
            .values()
            .filter(|o| options.namespace.is_none() || o.meta().namespace == options.namespace)
            .filter(|o| {
                let labels = o.meta().labels.as_ref();
                options
                    .labels
                    .iter()
                    .all(|(k, v)| labels.and_then(|l| l.get(k)) == Some(v))
            })
            .cloned()
            .collect()
    }
}

/// Cached reads hand out copies, as controller-runtime's reader deep-copies unless told not to.
fn copy(object_type: &ObjectType, obj: &dyn Object) -> Result<Box<dyn Object>> {
    object_type.decode(obj.to_json()?)
}

impl Cache {
    pub(crate) fn new(kube: kube::Client, sync_period: Duration) -> Self {
        Self {
            kube,
            sync_period,
            informers: Arc::new(Mutex::new(HashMap::new())),
            stop: CancellationToken::new(),
        }
    }

    /// Stops every informer's watch.
    pub(crate) fn stop(&self) {
        self.stop.cancel();
    }

    /// controller-runtime's cache reader: waits for the kind's first sync, then NotFound when
    /// the store does not hold the object. Go builds that error from the kind, not the plural
    /// (`pkg/cache/internal/cache_reader.go:72-76`), so its text is `<Kind>.<group> "<name>" not found`.
    pub(crate) async fn get(&self, object_type: &ObjectType, key: &ObjectKey) -> Result<Box<dyn Object>> {
        let informer = self.informer(object_type);
        wait_synced(&informer).await;
        let found = informer.state.lock().unwrap().get(key);
        match found {
            Some(obj) => copy(object_type, &*obj),
            None => Err(Error::status(
                StatusReason::NotFound,
                format!("{} \"{}\" not found", object_type.group_kind(), key.name),
            )),
        }
    }

    pub(crate) async fn list(&self, object_type: &ObjectType, options: &ListOptions) -> Result<Vec<Box<dyn Object>>> {
        let informer = self.informer(object_type);
        wait_synced(&informer).await;
        let listed = informer.state.lock().unwrap().list(options);
        listed.iter().map(|o| copy(object_type, &**o)).collect()
    }

    pub(crate) fn subscribe(&self, object_type: &ObjectType) -> mpsc::UnboundedReceiver<InformerEvent> {
        self.informer(object_type).state.lock().unwrap().subscribe()
    }

    /// controller-runtime `Kind.WaitForSync`: an error naming the kind when the first list does
    /// not finish in `timeout` (`pkg/internal/source/kind.go:137-147`).
    pub(crate) async fn wait_for_sync(&self, object_type: &ObjectType, timeout: Duration) -> Result<()> {
        let informer = self.informer(object_type);
        tokio::time::timeout(timeout, wait_synced(&informer))
            .await
            .map_err(|_| {
                Error::new(format!(
                    "timed out waiting for cache to be synced for Kind {object_type:?}"
                ))
            })
    }

    fn informer(&self, object_type: &ObjectType) -> Arc<Informer> {
        let mut informers = self.informers.lock().unwrap();
        if let Some(informer) = informers.get(object_type) {
            return informer.clone();
        }
        let informer = Arc::new(Informer {
            state: Mutex::new(Store::default()),
            synced: watch::channel(false).0,
        });
        informers.insert(object_type.clone(), informer.clone());
        tokio::spawn(run(
            informer.clone(),
            watch(self.kube.clone(), object_type),
            object_type.clone(),
            resync_period(self.sync_period),
            self.stop.clone(),
        ));
        informer
    }
}

async fn wait_synced(informer: &Informer) {
    let mut synced = informer.synced.subscribe();
    let _ = synced.wait_for(|s| *s).await;
}

/// controller-runtime `calculateResyncPeriod`: the period times a factor in [0.9, 1.1).
fn resync_period(period: Duration) -> Duration {
    period.mul_f64(crate::event::random_fraction() / 5.0 + 0.9)
}

/// The informer's single writer: watch events into the store, and resyncs.
async fn run(
    informer: Arc<Informer>,
    events: impl Stream<Item = std::result::Result<watcher::Event<DynamicObject>, watcher::Error>> + Send,
    object_type: ObjectType,
    resync: Duration,
    stop: CancellationToken,
) {
    let mut events = std::pin::pin!(events);
    let mut ticks = (!resync.is_zero()).then(|| tokio::time::interval_at(tokio::time::Instant::now() + resync, resync));
    loop {
        let tick = async {
            match &mut ticks {
                Some(t) => {
                    t.tick().await;
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            () = stop.cancelled() => return,
            () = tick => informer.state.lock().unwrap().resync(),
            event = events.next() => match event {
                Some(Ok(event)) => {
                    let Some(event) = decode_event(&object_type, event) else { continue };
                    if informer.state.lock().unwrap().apply(event) {
                        informer.synced.send_replace(true);
                    }
                }
                Some(Err(err)) => tracing::warn!(kind = ?object_type, error = %err, "watch error, retrying"),
                None => return,
            },
        }
    }
}

fn decode_event(
    object_type: &ObjectType,
    event: watcher::Event<DynamicObject>,
) -> Option<watcher::Event<Arc<dyn Object>>> {
    let decode = |obj: DynamicObject| -> Option<Arc<dyn Object>> {
        let decoded = serde_json::to_value(obj)
            .map_err(Error::foreign)
            .and_then(|v| object_type.decode(v));
        match decoded {
            Ok(obj) => Some(Arc::from(obj)),
            Err(err) => {
                tracing::error!(kind = ?object_type, error = %err, "cannot decode watched object");
                None
            }
        }
    };
    Some(match event {
        watcher::Event::Init => watcher::Event::Init,
        watcher::Event::InitDone => watcher::Event::InitDone,
        watcher::Event::InitApply(o) => watcher::Event::InitApply(decode(o)?),
        watcher::Event::Apply(o) => watcher::Event::Apply(decode(o)?),
        watcher::Event::Delete(o) => watcher::Event::Delete(decode(o)?),
    })
}

/// The only place the runtime opens a watch. Stable kube APIs only.
fn watch(
    kube: kube::Client,
    object_type: &ObjectType,
) -> impl Stream<Item = std::result::Result<watcher::Event<DynamicObject>, watcher::Error>> + Send + use<> {
    let api: Api<DynamicObject> = Api::all_with(kube, object_type.api_resource());
    watcher(api, watcher::Config::default()).default_backoff()
}

#[cfg(test)]
#[path = "cache_test.rs"]
mod tests;
