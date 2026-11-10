//! `pkg/event`, and under it the core/v1 event writer that client-go provides in Go.
//!
//! Events are core/v1, not `events.k8s.io`: stock Crossplane RBAC grants providers events only in
//! the core group.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::BuildHasher;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::errors::Error;
use crossplane_core::resource::Object;
use futures::FutureExt;
use futures::future::BoxFuture;
use k8s_openapi::api::core::v1::{Event as CoreEvent, EventSource, ObjectReference};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use k8s_openapi::jiff::Timestamp;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::clock::{Clock, SystemClock};

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

/// Go `event.APIRecorder` over controller-runtime's `GetEventRecorderFor(name)`, which is
/// client-go's `recorderImpl` with `EventSource{Component: name}`.
///
/// Go's `APIRecorder.Event` passes the recorder's annotations to `AnnotatedEventf` and ignores
/// the event's own (`pkg/event/event.go:93-106`), so this does too. Go's event filters are dropped:
/// `WithAnnotations` builds a recorder without them and the managed reconciler always goes
/// through it (parity spec section 8 item 6), so they never applied.
#[derive(Clone)]
pub(crate) struct ApiRecorder {
    component: Arc<str>,
    sink: mpsc::Sender<CoreEvent>,
    clock: Arc<dyn Clock>,
    annotations: BTreeMap<String, String>,
}

impl ApiRecorder {
    pub(crate) fn new(component: &str, sink: mpsc::Sender<CoreEvent>) -> Self {
        Self::with_clock(component, sink, Arc::new(SystemClock))
    }

    pub(crate) fn with_clock(component: &str, sink: mpsc::Sender<CoreEvent>, clock: Arc<dyn Clock>) -> Self {
        Self {
            component: component.into(),
            sink,
            clock,
            annotations: BTreeMap::new(),
        }
    }

    /// client-go `recorderImpl.generateEvent`: never blocks. Go's broadcaster drops an event
    /// when its 1000-event queue is full (`tools/record/event.go:46,186`); so does this.
    pub(crate) fn record(&self, involved: ObjectReference, type_: EventType, reason: &str, message: &str) {
        let source = EventSource {
            component: Some(self.component.to_string()),
            host: None,
        };
        // controller-runtime logs each recorded event at V(1) through a logger named `events`.
        tracing::debug!(
            target: "events",
            r#type = %type_str(type_),
            object = %serde_json::to_string(&involved).unwrap_or_default(),
            reason,
            "{message}"
        );
        let event = make_event(
            involved,
            &self.annotations,
            type_,
            reason,
            message,
            source,
            self.clock.now(),
        );
        if let Err(mpsc::error::TrySendError::Full(event)) = self.sink.try_send(event) {
            tracing::error!(reason = ?event.reason, "Unable to record event: too many queued events, dropped event");
        }
    }
}

impl Recorder for ApiRecorder {
    fn event(&self, obj: &dyn Object, event: Event) {
        self.record(object_reference(obj), event.type_, &event.reason.0, &event.message);
    }

    fn with_annotations(&self, annotations: &[(&str, &str)]) -> Arc<dyn Recorder> {
        let mut next = self.clone();
        next.annotations
            .extend(annotations.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
        Arc::new(next)
    }
}

/// client-go `reference.GetReference` for an object with metadata.
pub(crate) fn object_reference(obj: &dyn Object) -> ObjectReference {
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

fn type_str(type_: EventType) -> &'static str {
    match type_ {
        EventType::Normal => "Normal",
        EventType::Warning => "Warning",
    }
}

/// client-go `recorderImpl.makeEvent` plus the source fields `generateEvent` sets
/// (`tools/record/event.go:420-470`).
pub(crate) fn make_event(
    involved: ObjectReference,
    annotations: &BTreeMap<String, String>,
    type_: EventType,
    reason: &str,
    message: &str,
    source: EventSource,
    now: Timestamp,
) -> CoreEvent {
    let namespace = involved
        .namespace
        .clone()
        .filter(|ns| !ns.is_empty())
        .unwrap_or_else(|| "default".to_owned());
    CoreEvent {
        metadata: ObjectMeta {
            name: Some(event_name(involved.name.as_deref().unwrap_or_default(), now)),
            namespace: Some(namespace),
            annotations: (!annotations.is_empty()).then(|| annotations.clone()),
            ..Default::default()
        },
        reporting_component: source.component.clone(),
        reporting_instance: Some(source.host.clone().unwrap_or_default()),
        source: Some(source),
        involved_object: involved,
        reason: Some(reason.to_owned()),
        message: Some(message.to_owned()),
        first_timestamp: Some(Time(now)),
        last_timestamp: Some(Time(now)),
        count: Some(1),
        type_: Some(type_str(type_).to_owned()),
        ..Default::default()
    }
}

/// client-go `util.GenerateEventName`: `<name>.<unix nanos in hex>`.
fn event_name(name: &str, now: Timestamp) -> String {
    format!("{name}.{:x}", now.as_nanosecond())
}

fn s(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or_default()
}

fn source_parts(e: &CoreEvent) -> (&str, &str) {
    e.source
        .as_ref()
        .map(|src| (s(&src.component), s(&src.host)))
        .unwrap_or_default()
}

/// client-go `getEventKey` (`tools/record/events_cache.go:51-66`).
pub(crate) fn event_key(e: &CoreEvent) -> String {
    let (component, host) = source_parts(e);
    let o = &e.involved_object;
    [
        component,
        host,
        s(&o.kind),
        s(&o.namespace),
        s(&o.name),
        s(&o.field_path),
        s(&o.uid),
        s(&o.api_version),
        s(&e.type_),
        s(&e.reason),
        s(&e.message),
    ]
    .concat()
}

/// client-go `getSpamKey`.
pub(crate) fn spam_key(e: &CoreEvent) -> String {
    let (component, host) = source_parts(e);
    let o = &e.involved_object;
    [
        component,
        host,
        s(&o.kind),
        s(&o.namespace),
        s(&o.name),
        s(&o.uid),
        s(&o.api_version),
        s(&e.type_),
    ]
    .concat()
}

/// client-go `EventAggregatorByReasonFunc`: the aggregate key and the local key (the message).
pub(crate) fn aggregator_by_reason(e: &CoreEvent) -> (String, String) {
    let (component, host) = source_parts(e);
    let o = &e.involved_object;
    let key = [
        component,
        host,
        s(&o.kind),
        s(&o.namespace),
        s(&o.name),
        s(&o.uid),
        s(&o.api_version),
        s(&e.type_),
        s(&e.reason),
        s(&e.reporting_component),
        s(&e.reporting_instance),
    ]
    .concat();
    (key, s(&e.message).to_owned())
}

/// client-go `EventAggregatorByReasonMessageFunc`.
pub(crate) fn aggregator_message(e: &CoreEvent) -> String {
    format!("(combined from similar events): {}", s(&e.message))
}

/// `k8s.io/utils/lru`: `get` and `add` both make the key the most recent; `add` evicts the least
/// recent past capacity.
struct Lru<V> {
    capacity: usize,
    tick: u64,
    entries: HashMap<String, (V, u64)>,
}

impl<V: Clone> Lru<V> {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            tick: 0,
            entries: HashMap::new(),
        }
    }

    fn get(&mut self, key: &str) -> Option<V> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(v, t)| {
            *t = tick;
            v.clone()
        })
    }

    fn add(&mut self, key: String, value: V) {
        self.tick += 1;
        self.entries.insert(key, (value, self.tick));
        if self.entries.len() > self.capacity
            && let Some(oldest) = self.entries.iter().min_by_key(|(_, (_, t))| *t).map(|(k, _)| k.clone())
        {
            self.entries.remove(&oldest);
        }
    }
}

/// `golang.org/x/time/rate.Limiter.AllowN(now, 1)` on a passive clock, as client-go's
/// `tokenBucketPassiveRateLimiter.TryAccept` uses it: a refused call changes nothing
/// (`rate/rate.go` `reserveN` with `maxFutureReserve` 0). A new limiter starts full.
#[derive(Clone)]
struct PassiveBucket {
    qps: f64,
    burst: u32,
    tokens: f64,
    last: Option<Timestamp>,
}

impl PassiveBucket {
    fn new(qps: f32, burst: u32) -> Self {
        Self {
            qps: f64::from(qps),
            burst,
            tokens: 0.0,
            last: None,
        }
    }

    fn try_accept(&mut self, now: Timestamp) -> bool {
        let burst = f64::from(self.burst);
        let tokens = match self.last {
            None => burst,
            Some(last) => {
                let elapsed = if now < last {
                    0.0
                } else {
                    now.duration_since(last).as_secs_f64()
                };
                (self.tokens + elapsed * self.qps).min(burst)
            }
        } - 1.0;
        if tokens < 0.0 || self.burst < 1 {
            return false;
        }
        self.tokens = tokens;
        self.last = Some(now);
        true
    }
}

#[derive(Clone)]
struct AggregateRecord {
    /// Shared with the cached record, as Go's set is a map held by reference, so `PopAny` below
    /// shrinks the cached set too.
    local_keys: Arc<Mutex<HashSet<String>>>,
    last_timestamp: Option<Timestamp>,
}

#[derive(Clone, Default)]
struct EventLog {
    count: i32,
    first_timestamp: Option<Time>,
    name: Option<String>,
    resource_version: Option<String>,
}

/// client-go `CorrelatorOptions`; zero values take the defaults, as `populateDefaults` does.
#[derive(Clone)]
pub(crate) struct CorrelatorOptions {
    pub lru_cache_size: usize,
    pub burst_size: u32,
    pub qps: f32,
    pub max_events: usize,
    pub max_interval_seconds: i64,
    pub spam_key: fn(&CoreEvent) -> String,
}

impl Default for CorrelatorOptions {
    /// `events_cache.go:35-48`: LRU 4096, aggregate after 10 similar events in 600s, a burst of
    /// 25 per source and object refilled at one event per 300s.
    fn default() -> Self {
        Self {
            lru_cache_size: 4096,
            burst_size: 25,
            qps: 1.0 / 300.0,
            max_events: 10,
            max_interval_seconds: 600,
            spam_key,
        }
    }
}

/// What a correlated event turns into.
#[derive(Debug, PartialEq)]
pub(crate) enum Correlated {
    /// The spam filter dropped it.
    Skip,
    /// `patch` is set when the event repeats one already written: PATCH it, else POST.
    Write {
        event: Box<CoreEvent>,
        patch: Option<Value>,
    },
}

/// client-go `EventCorrelator` (`tools/record/events_cache.go`): filters, aggregates, counts and
/// de-duplicates events before they reach the API server.
pub(crate) struct Correlator {
    clock: Arc<dyn Clock>,
    options: CorrelatorOptions,
    spam: Mutex<Lru<PassiveBucket>>,
    aggregates: Mutex<Lru<AggregateRecord>>,
    logs: Mutex<Lru<EventLog>>,
}

impl Correlator {
    pub(crate) fn new(clock: Arc<dyn Clock>, options: CorrelatorOptions) -> Self {
        let size = options.lru_cache_size;
        Self {
            clock,
            options,
            spam: Mutex::new(Lru::new(size)),
            aggregates: Mutex::new(Lru::new(size)),
            logs: Mutex::new(Lru::new(size)),
        }
    }

    /// Go `EventCorrelate`. The filter runs last, after the observation has already counted the
    /// event, as in Go.
    pub(crate) fn correlate(&self, event: &CoreEvent) -> Correlated {
        let (aggregated, key) = self.aggregate(event);
        let (observed, patch) = self.observe(aggregated, key);
        if self.filter(&observed) {
            return Correlated::Skip;
        }
        Correlated::Write {
            event: Box::new(observed),
            patch,
        }
    }

    /// Go `UpdateState`: keyed by the full event key, whatever key `correlate` used.
    pub(crate) fn update_state(&self, event: &CoreEvent) {
        self.logs.lock().unwrap().add(event_key(event), log_of(event));
    }

    /// Go `EventSourceObjectSpamFilter.Filter`.
    fn filter(&self, event: &CoreEvent) -> bool {
        let key = (self.options.spam_key)(event);
        let mut spam = self.spam.lock().unwrap();
        let mut bucket = spam
            .get(&key)
            .unwrap_or_else(|| PassiveBucket::new(self.options.qps, self.options.burst_size));
        let filtered = !bucket.try_accept(self.clock.now());
        spam.add(key, bucket);
        filtered
    }

    /// Go `EventAggregator.EventAggregate`.
    fn aggregate(&self, event: &CoreEvent) -> (CoreEvent, String) {
        let now = self.clock.now();
        let key = event_key(event);
        let (aggregate_key, local_key) = aggregator_by_reason(event);
        let mut aggregates = self.aggregates.lock().unwrap();
        let fresh = || AggregateRecord {
            local_keys: Arc::new(Mutex::new(HashSet::new())),
            last_timestamp: None,
        };
        let mut record = aggregates.get(&aggregate_key).unwrap_or_else(fresh);
        let max_interval = self.options.max_interval_seconds as f64;
        let stale = record
            .last_timestamp
            .is_none_or(|last| now.duration_since(last).as_secs_f64() > max_interval);
        if stale {
            record = fresh();
        }
        record.local_keys.lock().unwrap().insert(local_key);
        record.last_timestamp = Some(now);
        aggregates.add(aggregate_key.clone(), record.clone());

        let mut local_keys = record.local_keys.lock().unwrap();
        if local_keys.len() < self.options.max_events {
            return (event.clone(), key);
        }
        if let Some(any) = local_keys.iter().next().cloned() {
            local_keys.remove(&any);
        }
        let aggregated = CoreEvent {
            metadata: ObjectMeta {
                name: Some(event_name(s(&event.involved_object.name), now)),
                namespace: event.metadata.namespace.clone(),
                ..Default::default()
            },
            count: Some(1),
            first_timestamp: Some(Time(now)),
            involved_object: event.involved_object.clone(),
            last_timestamp: Some(Time(now)),
            message: Some(aggregator_message(event)),
            type_: event.type_.clone(),
            reason: event.reason.clone(),
            source: event.source.clone(),
            ..Default::default()
        };
        (aggregated, aggregate_key)
    }

    /// Go `eventLogger.eventObserve`. The patch is `CreateTwoWayMergePatch` between the event with
    /// count, lastTimestamp and message blanked and the event itself, so it holds exactly those
    /// three fields when they are set.
    fn observe(&self, mut event: CoreEvent, key: String) -> (CoreEvent, Option<Value>) {
        let mut logs = self.logs.lock().unwrap();
        let last = logs.get(&key).unwrap_or_default();
        let mut patch = None;
        if last.count > 0 {
            event.metadata.name = last.name.clone();
            event.metadata.resource_version = last.resource_version.clone();
            event.first_timestamp = last.first_timestamp.clone();
            event.count = Some(last.count + 1);
            let mut fields = serde_json::Map::new();
            fields.insert("count".into(), json!(last.count + 1));
            let epoch = Time(Timestamp::UNIX_EPOCH);
            if event.last_timestamp.as_ref() != Some(&epoch) {
                fields.insert("lastTimestamp".into(), json!(event.last_timestamp));
            }
            if !s(&event.message).is_empty() {
                fields.insert("message".into(), json!(event.message));
            }
            patch = Some(Value::Object(fields));
        }
        logs.add(key, log_of(&event));
        (event, patch)
    }
}

fn log_of(event: &CoreEvent) -> EventLog {
    EventLog {
        count: event.count.unwrap_or_default(),
        first_timestamp: event.first_timestamp.clone(),
        name: event.metadata.name.clone(),
        resource_version: event.metadata.resource_version.clone(),
    }
}

/// How a write to the API server failed, in the classes client-go's `recordEvent` tells apart.
#[derive(Debug)]
pub(crate) enum SinkError {
    /// `restclient.RequestConstructionError`: the same request would fail again.
    Construction(String),
    /// `errors.StatusError`: the server answered with this code.
    Status(u16, String),
    /// Anything else, transport errors included: retried.
    Other(String),
}

/// client-go `EventSink`, less `Update`, which `recordToSink` never calls.
pub(crate) trait EventSink: Send + Sync + 'static {
    fn create<'a>(&'a self, event: &'a CoreEvent) -> BoxFuture<'a, Result<CoreEvent, SinkError>>;
    fn patch<'a>(&'a self, event: &'a CoreEvent, patch: &'a Value) -> BoxFuture<'a, Result<CoreEvent, SinkError>>;
}

/// client-go `maxTriesPerEvent` and `defaultSleepDuration` (`tools/record/event.go:39-41`).
const MAX_TRIES_PER_EVENT: u32 = 12;
pub(crate) const DEFAULT_SLEEP_DURATION: Duration = Duration::from_secs(10);

/// A float in [0, 1) without a random-number dependency: std seeds each `RandomState` randomly.
pub(crate) fn random_fraction() -> f64 {
    let bits = std::collections::hash_map::RandomState::new().hash_one(0u8);
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

/// client-go `eventBroadcasterImpl.recordToSink`: correlate, then write with up to 12 tries, the
/// first wait randomized, giving up at once when `cancel` fires during a wait.
pub(crate) async fn record_to_sink(
    sink: &dyn EventSink,
    event: &CoreEvent,
    correlator: &Correlator,
    sleep: Duration,
    cancel: &CancellationToken,
) {
    let Correlated::Write { mut event, patch } = correlator.correlate(event) else {
        return;
    };
    let mut tries = 0;
    loop {
        if record_event(sink, &mut event, patch.as_ref(), correlator).await {
            break;
        }
        tries += 1;
        if tries >= MAX_TRIES_PER_EVENT {
            tracing::error!(event = ?event.metadata.name, "Unable to write event (retry limit exceeded!)");
            break;
        }
        let delay = if tries == 1 {
            sleep.mul_f64(random_fraction())
        } else {
            sleep
        };
        tokio::select! {
            biased;
            () = cancel.cancelled() => {
                tracing::error!(event = ?event.metadata.name, "Unable to write event (broadcaster is shut down)");
                return;
            }
            () = tokio::time::sleep(delay) => {}
        }
    }
}

/// client-go `recordEvent`: true when the event was written or abandoned, false to retry. A patch
/// that finds no event falls back to a create.
async fn record_event(
    sink: &dyn EventSink,
    event: &mut CoreEvent,
    patch: Option<&Value>,
    correlator: &Correlator,
) -> bool {
    let update = event.count.unwrap_or_default() > 1;
    let mut result = None;
    if update && let Some(patch) = patch {
        result = Some(sink.patch(event, patch).await);
    }
    let not_found = matches!(result, Some(Err(SinkError::Status(404, _))));
    if !update || patch.is_none() || not_found {
        event.metadata.resource_version = None;
        result = Some(sink.create(event).await);
    }
    match result.expect("a write was attempted") {
        Ok(written) => {
            correlator.update_state(&written);
            true
        }
        Err(SinkError::Construction(err)) => {
            tracing::error!(error = %err, "Unable to construct event (will not retry!)");
            true
        }
        Err(SinkError::Status(code, err)) => {
            tracing::error!(code, error = %err, "Server rejected event (will not retry!)");
            true
        }
        Err(SinkError::Other(err)) => {
            tracing::error!(error = %err, "Unable to write event (may retry after sleeping)");
            false
        }
    }
}

/// client-go's `maxQueuedEvents`: the broadcaster's queue length.
pub(crate) const MAX_QUEUED_EVENTS: usize = 1000;

/// The manager's broadcaster task: client-go's `StartRecordingToSink` with a fresh correlator,
/// writing one event at a time until `cancel`.
pub(crate) async fn run_broadcaster(
    sink: Arc<dyn EventSink>,
    mut events: mpsc::Receiver<CoreEvent>,
    clock: Arc<dyn Clock>,
    sleep: Duration,
    cancel: CancellationToken,
) {
    let correlator = Correlator::new(clock, CorrelatorOptions::default());
    loop {
        let event = tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            event = events.recv() => match event {
                Some(event) => event,
                None => return,
            },
        };
        record_to_sink(&*sink, &event, &correlator, sleep, &cancel).await;
    }
}

/// The production sink: core/v1 Events through the API server, as client-go's `EventSinkImpl`.
pub(crate) struct KubeEventSink {
    kube: kube::Client,
}

impl KubeEventSink {
    pub(crate) fn new(kube: kube::Client) -> Self {
        Self { kube }
    }

    fn api(&self, event: &CoreEvent) -> kube::Api<CoreEvent> {
        kube::Api::namespaced(self.kube.clone(), s(&event.metadata.namespace))
    }
}

fn sink_error(err: kube::Error) -> SinkError {
    match err {
        kube::Error::Api(status) => SinkError::Status(status.code, status.message.clone()),
        kube::Error::BuildRequest(err) => SinkError::Construction(err.to_string()),
        other => SinkError::Other(other.to_string()),
    }
}

impl EventSink for KubeEventSink {
    fn create<'a>(&'a self, event: &'a CoreEvent) -> BoxFuture<'a, Result<CoreEvent, SinkError>> {
        async move {
            self.api(event)
                .create(&kube::api::PostParams::default(), event)
                .await
                .map_err(sink_error)
        }
        .boxed()
    }

    fn patch<'a>(&'a self, event: &'a CoreEvent, patch: &'a Value) -> BoxFuture<'a, Result<CoreEvent, SinkError>> {
        async move {
            self.api(event)
                .patch(
                    s(&event.metadata.name),
                    &kube::api::PatchParams::default(),
                    &kube::api::Patch::Strategic(patch),
                )
                .await
                .map_err(sink_error)
        }
        .boxed()
    }
}

#[cfg(test)]
#[path = "event_test.rs"]
mod tests;
