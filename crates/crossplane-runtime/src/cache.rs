//! The informer cache: controller-runtime's `cache.Cache`, on stable kube-runtime.
//!
//! One informer per kind, started on first use. It keeps the latest copy of every object, serves
//! cached reads to `KubeClient`, and hands each subscribed controller the kind's events with the
//! old object attached, so predicates compare old and new as Go's do. A kind costs one LIST and
//! one WATCH however many readers and controllers use it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::client::ListOptions;
use crossplane_core::errors::Result;
use crossplane_core::resource::{Object, ObjectKey, ObjectType};
use futures::Stream;
use kube::api::{Api, DynamicObject};
use kube::runtime::{WatchStreamExt, watcher};
use tokio::sync::mpsc;

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
    /// Go `Cache.SyncPeriod`, the `--sync` flag.
    sync_period: Duration,
    informers: Arc<Mutex<HashMap<ObjectType, Arc<Informer>>>>,
}

struct Informer {
    /// Written only by this informer's watch task.
    objects: Mutex<HashMap<ObjectKey, Arc<dyn Object>>>,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<InformerEvent>>>,
}

impl Cache {
    pub(crate) fn new(kube: kube::Client, sync_period: Duration) -> Self {
        Self {
            kube,
            sync_period,
            informers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Starts the kind's informer and waits for its first sync when this is the first use.
    /// NotFound when a synced informer does not hold the object, as controller-runtime's cache
    /// reader returns.
    pub(crate) async fn get(&self, object_type: &ObjectType, key: &ObjectKey) -> Result<Box<dyn Object>> {
        let _ = self.informer(object_type);
        todo!("cached get of {key:?}")
    }

    pub(crate) async fn list(&self, object_type: &ObjectType, options: &ListOptions) -> Result<Vec<Box<dyn Object>>> {
        let _ = self.informer(object_type);
        todo!("cached list with {options:?}")
    }

    /// A subscriber first receives the objects already held as `Create`s, as client-go replays
    /// to a late event handler, then live events.
    pub(crate) fn subscribe(&self, object_type: &ObjectType) -> mpsc::UnboundedReceiver<InformerEvent> {
        let _ = &self.informer(object_type).subscribers;
        todo!()
    }

    fn informer(&self, object_type: &ObjectType) -> Arc<Informer> {
        let _ = (&self.informers, self.sync_period, watch(self.kube.clone(), object_type));
        // TODO: get or insert under the map lock. On insert, spawn the single writer task:
        //   for each watcher event: decode with object_type.decode;
        //     Apply/InitApply: old = objects.insert(key, new); send Update{old,new} or Create(new)
        //     Delete: objects.remove(key); send Delete
        //     InitDone: OPEN (docs/design/core.md): objects the relist did not return are still
        //       in `objects` and no Delete was sent for them. client-go sends
        //       DeletedFinalStateUnknown here.
        //   every sync_period: send Update{old: o, new: o} for each stored object.
        todo!()
    }
}

/// The only place the runtime opens a watch. Stable kube APIs only.
fn watch(
    kube: kube::Client,
    object_type: &ObjectType,
) -> impl Stream<Item = Result<watcher::Event<DynamicObject>, watcher::Error>> + Send + use<> {
    let api: Api<DynamicObject> = Api::all_with(kube, object_type.api_resource());
    watcher(api, watcher::Config::default()).default_backoff()
}
