//! `pkg/event`, and under it the core/v1 event writer that client-go provides in Go.
//!
//! Events are core/v1, not `events.k8s.io`: stock Crossplane RBAC grants providers events only in
//! the core group.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;

use crossplane_core::errors::Error;
use crossplane_core::resource::Object;
use k8s_openapi::api::core::v1::ObjectReference;
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventType {
    Normal,
    Warning,
}

/// Open set. The managed reconciler's reasons are constants in `reconciler::managed::reason`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason(pub Cow<'static, str>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub type_: EventType,
    pub reason: Reason,
    pub message: String,
    pub annotations: BTreeMap<String, String>,
}

impl Event {
    pub fn normal(reason: Reason, message: impl Into<String>) -> Self {
        Self {
            type_: EventType::Normal,
            reason,
            message: message.into(),
            annotations: BTreeMap::new(),
        }
    }

    /// The message is `err.Error()`.
    pub fn warning(reason: Reason, err: &Error) -> Self {
        Self {
            type_: EventType::Warning,
            reason,
            message: err.to_string(),
            annotations: BTreeMap::new(),
        }
    }

    pub fn with_annotation(mut self, key: &str, value: &str) -> Self {
        self.annotations.insert(key.to_owned(), value.to_owned());
        self
    }
}

/// Go `event.Recorder`. Production recorders come from `Manager::event_recorder_for`.
pub trait Recorder: Send + Sync + 'static {
    fn event(&self, obj: &dyn Object, event: Event);
    fn with_annotations(&self, annotations: &[(&str, &str)]) -> Arc<dyn Recorder>;
}

/// Go `event.NewNopRecorder`, the reconcilers' default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NopRecorder;

impl Recorder for NopRecorder {
    fn event(&self, _: &dyn Object, _: Event) {}
    fn with_annotations(&self, _: &[(&str, &str)]) -> Arc<dyn Recorder> {
        Arc::new(NopRecorder)
    }
}

/// Go `event.APIRecorder`. Go's event filters are dropped: `WithAnnotations` builds a recorder
/// without them and the managed reconciler always goes through it (parity spec section 8 item
/// 6), so they never applied.
#[derive(Clone)]
pub(crate) struct ApiRecorder {
    component: Arc<str>,
    sink: mpsc::UnboundedSender<PendingEvent>,
    annotations: BTreeMap<String, String>,
}

impl ApiRecorder {
    pub(crate) fn new(component: &str, sink: mpsc::UnboundedSender<PendingEvent>) -> Self {
        Self {
            component: component.into(),
            sink,
            annotations: BTreeMap::new(),
        }
    }
}

impl Recorder for ApiRecorder {
    fn event(&self, obj: &dyn Object, event: Event) {
        let _ = (
            &self.component,
            &self.sink,
            &self.annotations,
            object_reference(obj),
            event,
        );
        todo!("merge annotations, send a PendingEvent; never blocks")
    }

    fn with_annotations(&self, annotations: &[(&str, &str)]) -> Arc<dyn Recorder> {
        let mut next = self.clone();
        next.annotations
            .extend(annotations.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
        Arc::new(next)
    }
}

fn object_reference(obj: &dyn Object) -> ObjectReference {
    let object_type = obj.object_type();
    let resource = object_type.api_resource();
    let meta = obj.meta();
    ObjectReference {
        api_version: Some(resource.api_version.clone()),
        kind: Some(resource.kind.clone()),
        name: meta.name.clone(),
        namespace: meta.namespace.clone(),
        uid: meta.uid.clone(),
        resource_version: meta.resource_version.clone(),
        ..Default::default()
    }
}

pub(crate) struct PendingEvent {
    pub involved: ObjectReference,
    pub annotations: BTreeMap<String, String>,
    pub type_: EventType,
    pub reason: Reason,
    pub message: String,
    pub source_component: String,
}

/// The manager's broadcaster task: client-go's `EventCorrelator` and `recordToSink`.
pub(crate) async fn run_broadcaster(kube: kube::Client, events: mpsc::UnboundedReceiver<PendingEvent>) {
    let _ = (kube, events);
    // TODO: OPEN (docs/design/core.md): the correlator is not designed beyond this outline.
    // client-go: aggregate 10 similar events in 600s, per-object spam filter of burst 25 then 1
    // per 300s, LRU 4096; new -> POST core/v1 Event named <involved>.<hex unix nanos>;
    // seen -> PATCH count, lastTimestamp, message; 12 tries, 10s apart.
    todo!()
}
