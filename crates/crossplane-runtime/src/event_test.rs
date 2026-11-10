//! client-go `tools/record/events_cache_test.go` and `event_test.go`, the spec for the correlator,
//! `recordToSink` and the recorder (listed in `docs/port-extra.tsv`).

use std::sync::atomic::{AtomicU32, Ordering};

use k8s_openapi::jiff::SignedDuration;

use super::*;

/// `k8s.io/utils/clock/testing.SimpleIntervalClock`: every `now` advances by `step`.
struct IntervalClock {
    now: Mutex<Timestamp>,
    step: SignedDuration,
}

impl IntervalClock {
    fn new(step: Duration) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(Timestamp::now()),
            step: SignedDuration::try_from(step).unwrap(),
        })
    }
}

impl Clock for IntervalClock {
    fn now(&self) -> Timestamp {
        let mut now = self.now.lock().unwrap();
        *now += self.step;
        *now
    }
}

/// `FakeClock`: moves only when stepped.
struct FakeClock(Mutex<Timestamp>);

impl FakeClock {
    fn step(&self, d: Duration) {
        *self.0.lock().unwrap() += SignedDuration::try_from(d).unwrap();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().unwrap()
    }
}

fn object_ref(kind: &str, name: &str, namespace: &str) -> ObjectReference {
    ObjectReference {
        kind: Some(kind.into()),
        name: Some(name.into()),
        namespace: Some(namespace.into()),
        uid: Some("C934D34AFB20242".into()),
        api_version: Some("version".into()),
        field_path: Some("spec.containers{mycontainer}".into()),
        ..Default::default()
    }
}

/// Go `makeEvent` in `events_cache_test.go`.
fn kubelet_event(reason: &str, message: &str, involved: ObjectReference) -> CoreEvent {
    let now = Some(Time(Timestamp::now()));
    CoreEvent {
        reason: Some(reason.into()),
        message: Some(message.into()),
        involved_object: involved,
        source: Some(EventSource {
            component: Some("kubelet".into()),
            host: Some("kublet.node1".into()),
        }),
        count: Some(1),
        first_timestamp: now.clone(),
        last_timestamp: now,
        type_: Some("Normal".into()),
        ..Default::default()
    }
}

fn copies(n: usize, template: &CoreEvent) -> Vec<CoreEvent> {
    vec![template.clone(); n]
}

fn unique_events(n: usize) -> Vec<CoreEvent> {
    (0..n)
        .map(|i| {
            kubelet_event(
                &format!("reason-{i}"),
                &format!("message-{i}"),
                object_ref("Pod", &format!("pod-{i}"), &format!("ns-{i}")),
            )
        })
        .collect()
}

fn similar_events(n: usize, template: &CoreEvent, prefix: &str) -> Vec<CoreEvent> {
    (0..n)
        .map(|i| {
            let mut e = template.clone();
            e.message = Some(format!("{prefix}-{i}-{}", s(&template.message)));
            e
        })
        .collect()
}

fn with_count(mut e: CoreEvent, count: i32) -> CoreEvent {
    e.count = Some(count);
    e
}

/// Go `validateEvent`: timestamps are set, equal exactly when the count is one, the name has the
/// expected prefix, and everything else matches.
fn validate_event(context: &str, actual: &CoreEvent, expected: &CoreEvent) {
    let (first, last) = (actual.first_timestamp.clone(), actual.last_timestamp.clone());
    assert!(
        first.is_some() && last.is_some(),
        "{context}: timestamp wasn't set: {actual:?}"
    );
    if expected.count.unwrap_or_default() > 1 {
        assert_ne!(
            first, last,
            "{context}: FirstTimestamp and LastTimestamp must differ after compression"
        );
    } else {
        assert_eq!(
            first, last,
            "{context}: FirstTimestamp and LastTimestamp must be equal for one occurrence"
        );
    }
    let mut got = actual.clone();
    got.first_timestamp = expected.first_timestamp.clone();
    got.last_timestamp = expected.last_timestamp.clone();
    got.reporting_component = expected.reporting_component.clone();
    let (name, prefix) = (s(&got.metadata.name), s(&expected.metadata.name));
    assert!(
        name.starts_with(prefix),
        "{context}: name {name:?} lacks prefix {prefix:?}"
    );
    got.metadata.name = expected.metadata.name.clone();
    assert_eq!(&got, expected, "{context}");
}

// go: k8s.io/client-go/tools/record TestEventAggregatorByReasonFunc
#[test]
fn event_aggregator_by_reason_func() {
    let e1 = kubelet_event("end-of-world", "it was fun", object_ref("Pod", "pod1", "other"));
    let e2 = kubelet_event("end-of-world", "it was awful", object_ref("Pod", "pod1", "other"));
    let e3 = kubelet_event("nevermind", "it was a bug", object_ref("Pod", "pod1", "other"));
    let (agg1, local1) = aggregator_by_reason(&e1);
    let (agg2, local2) = aggregator_by_reason(&e2);
    let (agg3, _) = aggregator_by_reason(&e3);
    assert_eq!(agg1, agg2);
    assert_ne!(local1, local2);
    assert_ne!(agg1, agg3);
}

// go: k8s.io/client-go/tools/record TestEventAggregatorByReasonMessageFunc
#[test]
fn event_aggregator_by_reason_message_func() {
    let e = kubelet_event("end-of-world", "it was fun", object_ref("Pod", "pod1", "other"));
    assert!(aggregator_message(&e).starts_with("(combined from similar events): "));
}

#[test]
fn event_correlator() {
    struct Case {
        name: &'static str,
        previous: Vec<CoreEvent>,
        new: CoreEvent,
        expected: Option<CoreEvent>,
        interval_seconds: u64,
    }

    let pod = || object_ref("Pod", "my-pod", "my-ns");
    let first = kubelet_event("first", "i am first", pod());
    let duplicate = kubelet_event("duplicate", "me again", pod());
    let unique = kubelet_event("unique", "snowflake", pod());
    let mut similar = kubelet_event("similar", "similar message", pod());
    similar.involved_object.field_path = Some("spec.containers{container1}".into());
    let aggregate = kubelet_event(
        "similar",
        &aggregator_message(&similar),
        similar.involved_object.clone(),
    );
    let mut other_container = similar.clone();
    other_container.involved_object.field_path = Some("spec.containers{container2}".into());
    let similar_msg = s(&similar.message).to_owned();

    let cases = vec![
        // go: k8s.io/client-go/tools/record TestEventCorrelator/create-a-single-event
        Case {
            name: "create-a-single-event",
            previous: vec![],
            new: first.clone(),
            expected: Some(with_count(first.clone(), 1)),
            interval_seconds: 5,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/the-same-event-should-just-count
        Case {
            name: "the-same-event-should-just-count",
            previous: copies(1, &duplicate),
            new: duplicate.clone(),
            expected: Some(with_count(duplicate.clone(), 2)),
            interval_seconds: 5,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/the-same-event-should-just-count-even-if-more-than-aggregate
        Case {
            name: "the-same-event-should-just-count-even-if-more-than-aggregate",
            previous: copies(10, &duplicate),
            new: duplicate.clone(),
            expected: Some(with_count(duplicate.clone(), 11)),
            interval_seconds: 30,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/the-same-event-is-spam-if-happens-too-frequently
        Case {
            name: "the-same-event-is-spam-if-happens-too-frequently",
            previous: copies(26, &duplicate),
            new: duplicate.clone(),
            expected: None,
            interval_seconds: 1,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/create-many-unique-events
        Case {
            name: "create-many-unique-events",
            previous: unique_events(30),
            new: unique.clone(),
            expected: Some(with_count(unique.clone(), 1)),
            interval_seconds: 5,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/similar-events-should-aggregate-event
        Case {
            name: "similar-events-should-aggregate-event",
            previous: similar_events(9, &similar, &similar_msg),
            new: similar.clone(),
            expected: Some(with_count(aggregate.clone(), 1)),
            interval_seconds: 5,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/similar-events-many-times-should-count-the-aggregate
        Case {
            name: "similar-events-many-times-should-count-the-aggregate",
            previous: similar_events(10, &similar, &similar_msg),
            new: similar.clone(),
            expected: Some(with_count(aggregate.clone(), 2)),
            interval_seconds: 5,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/events-from-different-containers-do-not-aggregate
        Case {
            name: "events-from-different-containers-do-not-aggregate",
            previous: copies(1, &other_container),
            new: similar.clone(),
            expected: Some(with_count(similar.clone(), 1)),
            interval_seconds: 5,
        },
        // go: k8s.io/client-go/tools/record TestEventCorrelator/similar-events-whose-interval-is-greater-than-aggregate-interval-do-not-aggregate
        Case {
            name: "similar-events-whose-interval-is-greater-than-aggregate-interval-do-not-aggregate",
            previous: similar_events(9, &similar, &similar_msg),
            new: similar.clone(),
            expected: Some(with_count(similar.clone(), 1)),
            interval_seconds: 600,
        },
    ];

    for tc in cases {
        let clock = IntervalClock::new(Duration::from_secs(tc.interval_seconds));
        let correlator = Correlator::new(clock.clone(), CorrelatorOptions::default());
        for mut event in tc.previous {
            let now = Some(Time(clock.now()));
            event.first_timestamp = now.clone();
            event.last_timestamp = now;
            if let Correlated::Write { event, .. } = correlator.correlate(&event) {
                correlator.update_state(&event);
            }
        }
        let mut new = tc.new;
        let now = Some(Time(clock.now()));
        new.first_timestamp = now.clone();
        new.last_timestamp = now;
        match (correlator.correlate(&new), tc.expected) {
            (Correlated::Skip, None) => {}
            (Correlated::Write { event, .. }, Some(expected)) => validate_event(tc.name, &event, &expected),
            (got, want) => panic!("{}: got {got:?}, want skip {}", tc.name, want.is_none()),
        }
    }
}

fn spam_key_with_reason(e: &CoreEvent) -> String {
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
        s(&e.reason),
    ]
    .concat()
}

#[test]
fn event_spam_filter() {
    struct Case {
        name: &'static str,
        new: CoreEvent,
        expected: Option<CoreEvent>,
        spam_key: Option<fn(&CoreEvent) -> String>,
    }

    let pod = || object_ref("Pod", "my-pod", "my-ns");
    let original = kubelet_event("original", "i am first", pod());
    let different_reason = kubelet_event("duplicate", "me again", pod());
    let spam = kubelet_event("original", "me again", pod());
    let cases = vec![
        // go: k8s.io/client-go/tools/record TestEventSpamFilter/event should be reported as spam if object reference is the same for default spam filter
        Case {
            name: "event should be reported as spam if object reference is the same for default spam filter",
            new: different_reason.clone(),
            expected: None,
            spam_key: None,
        },
        // go: k8s.io/client-go/tools/record TestEventSpamFilter/event should not be reported as spam if object reference is the same, but reason is different for custom spam filter
        Case {
            name: "event should not be reported as spam if object reference is the same, but reason is different for custom spam filter",
            new: different_reason.clone(),
            expected: Some(different_reason.clone()),
            spam_key: Some(spam_key_with_reason),
        },
        // go: k8s.io/client-go/tools/record TestEventSpamFilter/event should  be reported as spam if object reference and reason is the same, but message is different for custom spam filter
        Case {
            name: "event should  be reported as spam if object reference and reason is the same, but message is different for custom spam filter",
            new: spam.clone(),
            expected: None,
            spam_key: Some(spam_key_with_reason),
        },
    ];
    for tc in cases {
        let options = CorrelatorOptions {
            burst_size: 1,
            spam_key: tc.spam_key.unwrap_or(spam_key),
            ..Default::default()
        };
        let correlator = Correlator::new(IntervalClock::new(Duration::from_secs(1)), options);
        if let Correlated::Write { event, .. } = correlator.correlate(&original) {
            correlator.update_state(&event);
        }
        match (correlator.correlate(&tc.new), tc.expected) {
            (Correlated::Skip, None) => {}
            (Correlated::Write { event, .. }, Some(expected)) => validate_event(tc.name, &event, &expected),
            (got, want) => panic!("{}: got {got:?}, want skip {}", tc.name, want.is_none()),
        }
    }
}

type SinkFn = Box<dyn Fn(&CoreEvent, Option<&Value>) -> Result<CoreEvent, SinkError> + Send + Sync>;

/// Go `testEventSink`: one closure for both verbs; the patch is `None` on create.
struct TestSink(SinkFn);

impl EventSink for TestSink {
    fn create<'a>(&'a self, event: &'a CoreEvent) -> BoxFuture<'a, Result<CoreEvent, SinkError>> {
        let result = (self.0)(event, None);
        async move { result }.boxed()
    }
    fn patch<'a>(&'a self, event: &'a CoreEvent, patch: &'a Value) -> BoxFuture<'a, Result<CoreEvent, SinkError>> {
        let result = (self.0)(event, Some(patch));
        async move { result }.boxed()
    }
}

#[tokio::test]
async fn write_event_error() {
    struct Case {
        name: &'static str,
        times_to_send_error: u32,
        attempts_wanted: u32,
        err: fn() -> SinkError,
    }

    let cases = vec![
        // go: k8s.io/client-go/tools/record TestWriteEventError/giveUp1
        Case {
            name: "giveUp1",
            times_to_send_error: 1000,
            attempts_wanted: 1,
            err: || SinkError::Construction(String::new()),
        },
        // go: k8s.io/client-go/tools/record TestWriteEventError/giveUp2
        Case {
            name: "giveUp2",
            times_to_send_error: 1000,
            attempts_wanted: 1,
            err: || SinkError::Status(0, String::new()),
        },
        // go: k8s.io/client-go/tools/record TestWriteEventError/retry1
        Case {
            name: "retry1",
            times_to_send_error: 1000,
            attempts_wanted: 12,
            err: || SinkError::Other("unexpected object".into()),
        },
        // go: k8s.io/client-go/tools/record TestWriteEventError/retry2
        Case {
            name: "retry2",
            times_to_send_error: 1000,
            attempts_wanted: 12,
            err: || SinkError::Other("A weird error".into()),
        },
        // go: k8s.io/client-go/tools/record TestWriteEventError/succeedEventually
        Case {
            name: "succeedEventually",
            times_to_send_error: 2,
            attempts_wanted: 2,
            err: || SinkError::Other("A weird error".into()),
        },
    ];
    let correlator = Correlator::new(IntervalClock::new(Duration::from_secs(1)), CorrelatorOptions::default());
    for tc in cases {
        let attempts = Arc::new(AtomicU32::new(0));
        let counted = attempts.clone();
        let sink = TestSink(Box::new(move |event, _| {
            if counted.fetch_add(1, Ordering::SeqCst) + 1 < tc.times_to_send_error {
                return Err((tc.err)());
            }
            Ok(event.clone())
        }));
        record_to_sink(
            &sink,
            &CoreEvent::default(),
            &correlator,
            Duration::ZERO,
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(attempts.load(Ordering::SeqCst), tc.attempts_wanted, "{}", tc.name);
    }
}

// go: k8s.io/client-go/tools/record TestUpdateExpiredEvent
#[tokio::test]
async fn update_expired_event() {
    let correlator = Correlator::new(IntervalClock::new(Duration::from_secs(1)), CorrelatorOptions::default());
    let created = Arc::new(Mutex::new(None));
    let seen = created.clone();
    let sink = TestSink(Box::new(move |event, patch| match patch {
        Some(_) => Err(SinkError::Status(404, "NotFound".into())),
        None => {
            *seen.lock().unwrap() = Some(event.clone());
            Ok(event.clone())
        }
    }));
    let mut ev = CoreEvent::default();
    ev.metadata.resource_version = Some("updated-resource-version".into());
    ev.count = Some(2);
    record_to_sink(&sink, &ev, &correlator, Duration::ZERO, &CancellationToken::new()).await;
    let created = created
        .lock()
        .unwrap()
        .clone()
        .expect("Event did not get created after patch failed");
    assert_eq!(created.metadata.resource_version, None, "resource version cleared");
}

// go: k8s.io/client-go/tools/record TestCancelEvent
#[tokio::test]
async fn cancel_event() {
    let correlator = Correlator::new(IntervalClock::new(Duration::from_secs(1)), CorrelatorOptions::default());
    let attempts = Arc::new(AtomicU32::new(0));
    let counted = attempts.clone();
    let sink = TestSink(Box::new(move |_, _| {
        counted.fetch_add(1, Ordering::SeqCst);
        Err(SinkError::Other("unexpected object".into()))
    }));
    let cancel = CancellationToken::new();
    cancel.cancel();
    record_to_sink(
        &sink,
        &CoreEvent::default(),
        &correlator,
        Duration::from_secs(1),
        &cancel,
    )
    .await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1, "tried once, then gave up");
}

/// Go `OnCreateFactory` and `OnPatchFactory`: creates are cached by event key; a patch is merged
/// into the cached event. Each write is sent on `writes` with whether it was a patch.
fn caching_sink(writes: mpsc::UnboundedSender<(bool, CoreEvent)>) -> TestSink {
    let cache: Mutex<HashMap<String, CoreEvent>> = Mutex::new(HashMap::new());
    TestSink(Box::new(move |event, patch| {
        let mut cache = cache.lock().unwrap();
        let written = match patch {
            None => {
                cache.insert(event_key(event), event.clone());
                event.clone()
            }
            Some(patch) => {
                let cached = cache
                    .get(&event_key(event))
                    .ok_or_else(|| SinkError::Other("couldn't find Event in testCache".into()))?;
                let mut doc = serde_json::to_value(cached).unwrap();
                for (k, v) in patch.as_object().unwrap() {
                    doc[k] = v.clone();
                }
                serde_json::from_value(doc).unwrap()
            }
        };
        writes.send((patch.is_some(), written.clone())).unwrap();
        Ok(written)
    }))
}

fn pod_ref(uid: &str, field_path: Option<&str>) -> ObjectReference {
    ObjectReference {
        kind: Some("Pod".into()),
        name: Some("foo".into()),
        namespace: Some("baz".into()),
        uid: Some(uid.into()),
        api_version: Some("v1".into()),
        field_path: field_path.map(Into::into),
        ..Default::default()
    }
}

/// An expected event as the Go tables write it: the source and reporting controller are
/// `eventTest`, the name is only a prefix.
fn expected(involved: ObjectReference, reason: &str, message: &str, count: i32) -> CoreEvent {
    let namespace = involved
        .namespace
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or("default".into());
    CoreEvent {
        metadata: ObjectMeta {
            name: involved.name.clone(),
            namespace: Some(namespace),
            ..Default::default()
        },
        involved_object: involved,
        reason: Some(reason.into()),
        message: Some(message.into()),
        source: Some(EventSource {
            component: Some("eventTest".into()),
            host: None,
        }),
        reporting_component: Some("eventTest".into()),
        reporting_instance: Some(String::new()),
        count: Some(count),
        type_: Some("Normal".into()),
        ..Default::default()
    }
}

struct Emit {
    involved: ObjectReference,
    reason: &'static str,
    message: &'static str,
    expect: CoreEvent,
    expect_update: bool,
}

/// Runs the table through a recorder and a broadcaster writing to a caching sink, stepping the
/// fake clock a second before each event, as Go's `TestEventf` does.
async fn run_table(
    table: &[Emit],
    clock: &Arc<FakeClock>,
    sink: Arc<dyn EventSink>,
    mut writes: mpsc::UnboundedReceiver<(bool, CoreEvent)>,
) {
    let (tx, rx) = mpsc::channel(MAX_QUEUED_EVENTS);
    let cancel = CancellationToken::new();
    let broadcaster = tokio::spawn(run_broadcaster(sink, rx, clock.clone(), Duration::ZERO, cancel.clone()));
    let recorder = ApiRecorder::with_clock("eventTest", tx, clock.clone());
    for (index, item) in table.iter().enumerate() {
        clock.step(Duration::from_secs(1));
        recorder.record(item.involved.clone(), EventType::Normal, item.reason, item.message);
        let (patched, got) = writes.recv().await.unwrap();
        assert_eq!(patched, item.expect_update, "{index}: patched");
        validate_event(&index.to_string(), &got, &item.expect);
    }
    cancel.cancel();
    broadcaster.await.unwrap();
}

fn eventf_table() -> Vec<Emit> {
    let r1 = || pod_ref("bar", Some("spec.containers[2]"));
    let r2 = || pod_ref("differentUid", Some("spec.containers[3]"));
    let msg = "some verbose message: 1";
    vec![
        Emit {
            involved: r1(),
            reason: "Started",
            message: msg,
            expect: expected(r1(), "Started", msg, 1),
            expect_update: false,
        },
        Emit {
            involved: pod_ref("bar", None),
            reason: "Killed",
            message: "some other verbose message: 1",
            expect: expected(pod_ref("bar", None), "Killed", "some other verbose message: 1", 1),
            expect_update: false,
        },
        Emit {
            involved: r1(),
            reason: "Started",
            message: msg,
            expect: expected(r1(), "Started", msg, 2),
            expect_update: true,
        },
        Emit {
            involved: r2(),
            reason: "Started",
            message: msg,
            expect: expected(r2(), "Started", msg, 1),
            expect_update: false,
        },
        Emit {
            involved: r1(),
            reason: "Started",
            message: msg,
            expect: expected(r1(), "Started", msg, 3),
            expect_update: true,
        },
        Emit {
            involved: r2(),
            reason: "Stopped",
            message: msg,
            expect: expected(r2(), "Stopped", msg, 1),
            expect_update: false,
        },
        Emit {
            involved: r2(),
            reason: "Stopped",
            message: msg,
            expect: expected(r2(), "Stopped", msg, 2),
            expect_update: true,
        },
    ]
}

fn fake_clock() -> Arc<FakeClock> {
    Arc::new(FakeClock(Mutex::new(Timestamp::now())))
}

// go: k8s.io/client-go/tools/record TestEventf
#[tokio::test]
async fn eventf() {
    let (tx, rx) = mpsc::unbounded_channel();
    run_table(&eventf_table(), &fake_clock(), Arc::new(caching_sink(tx)), rx).await;
}

// go: k8s.io/client-go/tools/record TestEventfNoNamespace
#[tokio::test]
async fn eventf_no_namespace() {
    let mut r = pod_ref("bar", Some("spec.containers[2]"));
    r.namespace = Some(String::new());
    let msg = "some verbose message: 1";
    let mut want = expected(r.clone(), "Started", msg, 1);
    want.metadata.namespace = Some("default".into());
    let table = [Emit {
        involved: r,
        reason: "Started",
        message: msg,
        expect: want,
        expect_update: false,
    }];
    let (tx, rx) = mpsc::unbounded_channel();
    run_table(&table, &fake_clock(), Arc::new(caching_sink(tx)), rx).await;
}

// go: k8s.io/client-go/tools/record TestMultiSinkCache
#[tokio::test]
async fn multi_sink_cache() {
    let clock = fake_clock();
    let (tx, rx) = mpsc::unbounded_channel();
    run_table(&eventf_table(), &clock, Arc::new(caching_sink(tx)), rx).await;
    let (tx, rx) = mpsc::unbounded_channel();
    run_table(&eventf_table(), &clock, Arc::new(caching_sink(tx)), rx).await;
}

// go: k8s.io/client-go/tools/record TestLotsOfEvents
#[tokio::test]
async fn lots_of_events() {
    let counts: Arc<Vec<AtomicU32>> = Arc::new((0..MAX_QUEUED_EVENTS).map(|_| AtomicU32::new(0)).collect());
    let (done_tx, mut done) = mpsc::unbounded_channel();
    let seen = counts.clone();
    let sink = TestSink(Box::new(move |event, _| {
        let num: usize = s(&event.message).parse().unwrap();
        if seen[num].fetch_add(1, Ordering::SeqCst) + 1 < 5 {
            return Err(SinkError::Other("fake error".into()));
        }
        done_tx.send(num).unwrap();
        Ok(event.clone())
    }));
    let (tx, rx) = mpsc::channel(MAX_QUEUED_EVENTS);
    let cancel = CancellationToken::new();
    let broadcaster = tokio::spawn(run_broadcaster(
        Arc::new(sink),
        rx,
        Arc::new(SystemClock),
        Duration::ZERO,
        cancel.clone(),
    ));
    let recorder = ApiRecorder::new("eventTest", tx);
    let mut emitters = tokio::task::JoinSet::new();
    for i in 0..MAX_QUEUED_EVENTS {
        let recorder = recorder.clone();
        emitters.spawn(async move {
            let mut r = pod_ref("bar", None);
            r.name = Some(format!("foo-{i}"));
            r.api_version = Some("version".into());
            recorder.record(r, EventType::Normal, &format!("Reason-{i}"), &i.to_string());
        });
    }
    emitters.join_all().await;
    for _ in 0..MAX_QUEUED_EVENTS {
        done.recv().await.expect("no event was dropped");
    }
    for (i, c) in counts.iter().enumerate() {
        assert!(c.load(Ordering::SeqCst) >= 5, "only attempted event {i} {c:?} times");
    }
    cancel.cancel();
    broadcaster.await.unwrap();
}

// go: k8s.io/client-go/tools/record TestNonRacyShutdown
#[tokio::test]
async fn non_racy_shutdown() {
    let (tx, rx) = mpsc::channel(MAX_QUEUED_EVENTS);
    let cancel = CancellationToken::new();
    let sink = Arc::new(TestSink(Box::new(|e, _| Ok(e.clone()))));
    let broadcaster = tokio::spawn(run_broadcaster(
        sink,
        rx,
        Arc::new(SystemClock),
        Duration::ZERO,
        cancel.clone(),
    ));
    let recorder = ApiRecorder::new("eventTest", tx);
    let mut emitters = tokio::task::JoinSet::new();
    for _ in 0..100 {
        let recorder = recorder.clone();
        emitters
            .spawn(async move { recorder.record(ObjectReference::default(), EventType::Normal, "Started", "blah") });
    }
    emitters.join_all().await;
    cancel.cancel();
    broadcaster.await.unwrap();
}

// go: k8s.io/client-go/tools/record TestBroadcasterShutdown
#[tokio::test]
async fn broadcaster_shutdown() {
    let (_tx, rx) = mpsc::channel(MAX_QUEUED_EVENTS);
    let cancel = CancellationToken::new();
    let sink = Arc::new(TestSink(Box::new(|e, _| Ok(e.clone()))));
    let broadcaster = tokio::spawn(run_broadcaster(
        sink,
        rx,
        Arc::new(SystemClock),
        Duration::ZERO,
        cancel.clone(),
    ));
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(5), broadcaster)
        .await
        .expect("the broadcaster task ends when cancelled")
        .unwrap();
}

// go: k8s.io/client-go/tools/record TestStartEventWatcherExitsOnDirectShutdown
#[tokio::test]
async fn broadcaster_exits_when_every_recorder_is_gone() {
    let (tx, rx) = mpsc::channel(MAX_QUEUED_EVENTS);
    let sink = Arc::new(TestSink(Box::new(|e, _| Ok(e.clone()))));
    let broadcaster = tokio::spawn(run_broadcaster(
        sink,
        rx,
        Arc::new(SystemClock),
        Duration::ZERO,
        CancellationToken::new(),
    ));
    drop(tx);
    tokio::time::timeout(Duration::from_secs(5), broadcaster)
        .await
        .expect("the broadcaster task ends when its channel closes")
        .unwrap();
}

#[test]
fn a_full_queue_drops_the_event() {
    let (tx, mut rx) = mpsc::channel(1);
    let recorder = ApiRecorder::new("eventTest", tx);
    recorder.record(pod_ref("a", None), EventType::Normal, "One", "1");
    recorder.record(pod_ref("b", None), EventType::Warning, "Two", "2");
    assert_eq!(rx.try_recv().unwrap().reason.as_deref(), Some("One"));
    assert!(rx.try_recv().is_err(), "the second event was dropped, not queued");
}

#[test]
fn api_recorder_uses_its_own_annotations_not_the_events() {
    let (tx, mut rx) = mpsc::channel(4);
    let recorder = ApiRecorder::new("managed/mytype.sample.example.com", tx);
    let annotated = recorder.with_annotations(&[("external-name", "x")]);
    let obj = config_map();
    annotated.event(
        &obj,
        Event::normal(Reason(Cow::Borrowed("R")), "m").with_annotation("ignored", "y"),
    );
    recorder.event(&obj, Event::normal(Reason(Cow::Borrowed("R")), "m"));
    let first = rx.try_recv().unwrap();
    assert_eq!(
        first.metadata.annotations,
        Some([("external-name".to_owned(), "x".to_owned())].into())
    );
    assert_eq!(
        rx.try_recv().unwrap().metadata.annotations,
        None,
        "an empty map is omitted, as Go's omitempty"
    );
    assert_eq!(
        first.source.and_then(|s| s.component).as_deref(),
        Some("managed/mytype.sample.example.com")
    );
    assert_eq!(
        first.reporting_component.as_deref(),
        Some("managed/mytype.sample.example.com")
    );
    assert_eq!(first.reporting_instance.as_deref(), Some(""));
    let name = first.metadata.name.unwrap();
    assert!(name.starts_with("cm."), "{name}");
    assert!(
        u128::from_str_radix(&name[3..], 16).is_ok(),
        "<name>.<hex nanos>: {name}"
    );
    assert_eq!(first.involved_object.resource_version.as_deref(), Some("7"));
}

fn config_map() -> k8s_openapi::api::core::v1::ConfigMap {
    k8s_openapi::api::core::v1::ConfigMap {
        metadata: ObjectMeta {
            name: Some("cm".into()),
            namespace: Some("ns".into()),
            uid: Some("u".into()),
            resource_version: Some("7".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}
