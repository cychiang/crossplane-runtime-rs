//! controller-runtime's `reconcileHandler` cases (`pkg/internal/controller/controller_test.go`)
//! run through [`run`] over a key channel, on tokio's paused clock. Go counts the queue calls
//! through a delegating queue; here a recording rate limiter counts `when` (an `AddRateLimited`)
//! and `forget`, and the time each reconcile starts shows the delay the queue applied.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use futures::channel::mpsc as keys;
use futures::future::BoxFuture;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, advance};

use super::*;
use crate::ratelimiter::{ItemExponentialBackoff, RateLimiter};

fn request() -> Request {
    Request::namespaced("foo", "bar")
}

enum Step {
    Return(Result<Outcome>),
    /// Hold the reconcile until the test sends its result.
    Hold(oneshot::Receiver<Result<Outcome>>),
    Panic,
}

/// Go's `fakeReconciler`: reports each call and returns the next scripted result, or success
/// once the script runs out.
struct Scripted {
    steps: Mutex<VecDeque<Step>>,
    calls: mpsc::UnboundedSender<(Request, Instant)>,
}

impl Reconcile for Scripted {
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        self.calls.send((req.clone(), Instant::now())).unwrap();
        let step = self.steps.lock().unwrap().pop_front();
        Box::pin(async move {
            match step {
                None => Ok(Outcome::Done),
                Some(Step::Return(result)) => result,
                Some(Step::Hold(result)) => result.await.unwrap(),
                Some(Step::Panic) => panic!("boom"),
            }
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Call {
    When,
    Forget,
}

/// Crossplane's per-item limiter, 1s to 60s, with every call recorded.
#[derive(Default)]
struct Recording {
    inner: Option<ItemExponentialBackoff>,
    calls: Mutex<Vec<Call>>,
}

impl RateLimiter for Recording {
    fn when(&self, item: &Request) -> Duration {
        self.calls.lock().unwrap().push(Call::When);
        self.inner.as_ref().unwrap().when(item)
    }
    fn forget(&self, item: &Request) {
        self.calls.lock().unwrap().push(Call::Forget);
        self.inner.as_ref().unwrap().forget(item);
    }
}

struct Harness {
    keys: keys::UnboundedSender<Request>,
    calls: mpsc::UnboundedReceiver<(Request, Instant)>,
    limiter: Arc<Recording>,
    shutdown: CancellationToken,
    driver: tokio::task::JoinHandle<Result<()>>,
}

impl Harness {
    fn start(steps: impl IntoIterator<Item = Step>, workers: usize, recover_panic: bool) -> Self {
        let (keys, key_stream) = keys::unbounded();
        let (calls_tx, calls) = mpsc::unbounded_channel();
        let limiter = Arc::new(Recording {
            inner: Some(ItemExponentialBackoff::new(
                Duration::from_secs(1),
                Duration::from_secs(60),
            )),
            ..Default::default()
        });
        let reconciler = Arc::new(Scripted {
            steps: Mutex::new(steps.into_iter().collect()),
            calls: calls_tx,
        });
        let options = ControllerOptions {
            max_concurrent_reconciles: workers,
            rate_limiter: limiter.clone(),
            recover_panic,
        };
        let shutdown = CancellationToken::new();
        let driver = tokio::spawn(run("test".into(), key_stream, reconciler, options, shutdown.clone()));
        Self {
            keys,
            calls,
            limiter,
            shutdown,
            driver,
        }
    }

    fn add(&self, key: Request) {
        self.keys.unbounded_send(key).unwrap();
    }

    /// The next reconcile and how long after `since` it started.
    async fn reconciled(&mut self, since: Instant) -> (Request, Duration) {
        let next = tokio::time::timeout(Duration::from_secs(24 * 3600), self.calls.recv());
        let (req, at) = next
            .await
            .expect("the reconciler is called within a day: is the key stuck?")
            .expect("the reconciler is still there");
        (req, at - since)
    }

    /// No reconcile starts within an hour.
    async fn quiet(&mut self) {
        let next = tokio::time::timeout(Duration::from_secs(3600), self.calls.recv()).await;
        assert!(next.is_err(), "unexpected reconcile {next:?}");
    }

    fn limiter_calls(&self) -> Vec<Call> {
        std::mem::take(&mut *self.limiter.calls.lock().unwrap())
    }

    async fn stop(self) -> Result<()> {
        self.shutdown.cancel();
        self.driver.await.unwrap()
    }
}

/// Runs other tasks to their next await without moving the paused clock.
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

#[test]
fn queue_ops_match_controller_runtime() {
    let minute = Duration::from_secs(60);
    assert_eq!(queue_op(&Ok(Outcome::Done)), QueueOp::Forget);
    assert_eq!(queue_op(&Ok(Outcome::Requeue)), QueueOp::AddRateLimited);
    assert_eq!(
        queue_op(&Ok(Outcome::RequeueAfter(minute))),
        QueueOp::ForgetAndAddAfter(minute)
    );
    assert_eq!(
        queue_op(&Ok(Outcome::RequeueAfter(Duration::ZERO))),
        QueueOp::Forget,
        "Go tests RequeueAfter > 0"
    );
    assert_eq!(queue_op(&Err(Error::new("boom"))), QueueOp::AddRateLimited);
}

// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Reconciler should call the Reconciler function
#[tokio::test]
async fn calls_the_reconciler() {
    let (calls, _rx) = mpsc::unbounded_channel();
    let r = Scripted {
        steps: Mutex::new([Step::Return(Ok(Outcome::Requeue))].into()),
        calls,
    };
    assert_eq!(reconcile(&r, &request(), true).await, Ok(Outcome::Requeue));
}

// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Reconciler should not recover panic if RecoverPanic is false
#[tokio::test(start_paused = true)]
async fn does_not_recover_panic_when_recover_panic_is_false() {
    let h = Harness::start([Step::Panic], 1, false);
    h.add(request());
    let err = h.driver.await.expect_err("the driver ends with the reconciler's panic");
    assert!(err.is_panic());
    assert_eq!(err.into_panic().downcast_ref::<&str>(), Some(&"boom"));
}

// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Reconciler should recover panic if RecoverPanic is true by default
#[tokio::test]
async fn recovers_panic_by_default() {
    assert!(ControllerOptions::default().recover_panic);
    let (calls, _rx) = mpsc::unbounded_channel();
    let r = Scripted {
        steps: Mutex::new([Step::Panic].into()),
        calls,
    };
    let err = reconcile(&r, &request(), ControllerOptions::default().recover_panic)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("[recovered]"), "{err}");
}

/// A recovered panic is an error to the queue: the key is rate limited, released and reconciled
/// again, not left held by the worker that panicked.
// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Reconciler should recover panic if RecoverPanic is true
#[tokio::test(start_paused = true)]
async fn recovers_panic_when_recover_panic_is_true() {
    let mut h = Harness::start([Step::Panic], 1, true);
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await, (request(), Duration::ZERO));
    assert_eq!(h.reconciled(t0).await, (request(), Duration::from_secs(1)));
    assert_eq!(h.limiter_calls(), [Call::When, Call::Forget]);
    h.stop().await.unwrap();
}

#[tokio::test]
async fn a_recovered_panic_carries_go_s_message() {
    let (calls, _rx) = mpsc::unbounded_channel();
    let r = Scripted {
        steps: Mutex::new([Step::Panic].into()),
        calls,
    };
    assert_eq!(
        reconcile(&r, &request(), true).await.unwrap_err().to_string(),
        "panic: boom [recovered]"
    );
}

// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Processing queue items from a Controller should call Reconciler if an item is enqueued
#[tokio::test(start_paused = true)]
async fn calls_the_reconciler_for_an_enqueued_item() {
    let mut h = Harness::start([Step::Return(Ok(Outcome::Done))], 1, true);
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await, (request(), Duration::ZERO));
    h.quiet().await;
    assert_eq!(h.limiter_calls(), [Call::Forget]);
    h.stop().await.unwrap();
}

// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Processing queue items from a Controller should requeue a Request if there is an error and continue processing items
#[tokio::test(start_paused = true)]
async fn requeues_rate_limited_on_error() {
    let mut h = Harness::start([Step::Return(Err(Error::new("expected error: reconcile")))], 1, true);
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await, (request(), Duration::ZERO));
    assert_eq!(h.reconciled(t0).await, (request(), Duration::from_secs(1)));
    h.quiet().await;
    assert_eq!(h.limiter_calls(), [Call::When, Call::Forget]);
    h.stop().await.unwrap();
}

// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Processing queue items from a Controller should not reset backoff until there's a non-error result
#[tokio::test(start_paused = true)]
async fn does_not_reset_backoff_until_a_non_error_result() {
    let mut h = Harness::start(
        [
            Step::Return(Err(Error::new("something's wrong"))),
            Step::Return(Err(Error::new("another thing's wrong"))),
        ],
        1,
        true,
    );
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::ZERO);
    assert_eq!(h.reconciled(t0).await.1, Duration::from_secs(1));
    assert_eq!(
        h.reconciled(t0).await.1,
        Duration::from_secs(3),
        "the second error backs off 2s"
    );
    h.quiet().await;
    assert_eq!(h.limiter_calls(), [Call::When, Call::When, Call::Forget]);

    let t1 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t1).await.1, Duration::ZERO);
    h.stop().await.unwrap();
}

// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Processing queue items from a Controller should requeue a Request with rate limiting if the Result sets Requeue:true and continue processing items
#[tokio::test(start_paused = true)]
async fn requeues_rate_limited_on_requeue() {
    let mut h = Harness::start([Step::Return(Ok(Outcome::Requeue))], 1, true);
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::ZERO);
    assert_eq!(h.reconciled(t0).await.1, Duration::from_secs(1));
    h.quiet().await;
    assert_eq!(h.limiter_calls(), [Call::When, Call::Forget]);
    h.stop().await.unwrap();
}

/// Go's first result sets both `Requeue` and `RequeueAfter`; `Outcome` cannot hold both, and Go
/// treats it as `RequeueAfter` alone, which is what is run here.
// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Processing queue items from a Controller should requeue a Request after a duration (but not rate-limited) if the Result sets RequeueAfter (regardless of Requeue)
#[tokio::test(start_paused = true)]
async fn requeues_after_a_duration_without_rate_limiting() {
    let ms = Duration::from_millis;
    let mut h = Harness::start(
        [
            Step::Return(Ok(Outcome::RequeueAfter(ms(100)))),
            Step::Return(Ok(Outcome::RequeueAfter(ms(100)))),
        ],
        1,
        true,
    );
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::ZERO);
    assert_eq!(h.reconciled(t0).await.1, ms(100));
    assert_eq!(h.reconciled(t0).await.1, ms(200));
    h.quiet().await;
    assert_eq!(h.limiter_calls(), [Call::Forget, Call::Forget, Call::Forget]);
    h.stop().await.unwrap();
}

/// Go's first result is `RequeueAfter` with an error; `Result<Outcome>` cannot hold both, and Go
/// ignores the result when there is an error, so the error alone is run here.
// go: sigs.k8s.io/controller-runtime/pkg/internal/controller TestSource/Processing queue items from a Controller should perform error behavior if error is not nil, regardless of RequeueAfter
#[tokio::test(start_paused = true)]
async fn error_behaviour_wins_over_requeue_after() {
    let ms = Duration::from_millis;
    let mut h = Harness::start(
        [
            Step::Return(Err(Error::new("expected error: reconcile"))),
            Step::Return(Ok(Outcome::RequeueAfter(ms(100)))),
        ],
        1,
        true,
    );
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::ZERO);
    assert_eq!(h.reconciled(t0).await.1, Duration::from_secs(1));
    assert_eq!(h.reconciled(t0).await.1, Duration::from_secs(1) + ms(100));
    h.quiet().await;
    assert_eq!(h.limiter_calls(), [Call::When, Call::Forget, Call::Forget]);
    h.stop().await.unwrap();
}

/// The priority queue holds one entry per key, so an event that arrives during a reconcile
/// leaves the key ready, and the `RequeueAfter` the reconcile then returns is absorbed by it
/// (`priorityqueue.go:239-240`). client-go's queue would reconcile again after the delay too.
#[tokio::test(start_paused = true)]
async fn an_event_during_a_reconcile_absorbs_its_requeue_after() {
    let (release, held) = oneshot::channel();
    let mut h = Harness::start([Step::Hold(held)], 1, true);
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::ZERO);
    h.add(request());
    settle().await;
    release
        .send(Ok(Outcome::RequeueAfter(Duration::from_secs(60))))
        .unwrap();
    assert_eq!(
        h.reconciled(t0).await.1,
        Duration::ZERO,
        "the event is reconciled at once"
    );
    h.quiet().await;
    h.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_event_while_waiting_cancels_the_requeue_after() {
    let mut h = Harness::start(
        [Step::Return(Ok(Outcome::RequeueAfter(Duration::from_secs(60))))],
        1,
        true,
    );
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::ZERO);
    advance(Duration::from_secs(10)).await;
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::from_secs(10));
    h.quiet().await;
    h.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn shutdown_lets_the_in_flight_reconcile_finish_and_drops_queued_keys() {
    let (release, held) = oneshot::channel();
    let mut h = Harness::start([Step::Hold(held)], 1, true);
    let t0 = Instant::now();
    h.add(request());
    assert_eq!(h.reconciled(t0).await.1, Duration::ZERO);
    h.add(Request::namespaced("foo", "queued"));
    settle().await;
    h.shutdown.cancel();
    settle().await;
    assert!(!h.driver.is_finished(), "the driver waits for the in-flight reconcile");
    release.send(Ok(Outcome::Done)).unwrap();
    h.driver.await.unwrap().unwrap();
    assert!(
        h.calls.try_recv().is_err(),
        "the queued key is not reconciled after shutdown"
    );
}

/// The managed reconciler times out its own work and returns; the driver adds no timeout
/// (controller-runtime's `ReconciliationTimeout` is unset in the baseline). What the driver must
/// guarantee is that a reconcile that never returns, because its future is dropped, does not
/// leave its key held: the next add of the key is handed out.
#[tokio::test(start_paused = true)]
async fn a_dropped_reconcile_releases_its_key() {
    let queue = Arc::new(WorkQueue::new(Arc::new(ItemExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(60),
    ))));
    let (_never, held) = oneshot::channel();
    let (calls, mut called) = mpsc::unbounded_channel();
    let reconciler = Arc::new(Scripted {
        steps: Mutex::new([Step::Hold(held)].into()),
        calls,
    });
    queue.add(request());
    let worker = tokio::spawn(work(queue.clone(), reconciler, true));
    called.recv().await.unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());

    queue.add(request());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(24 * 3600), queue.get())
            .await
            .ok()
            .flatten(),
        Some(request()),
        "the key is not stuck in the held set"
    );
}

/// Many tasks add the same few keys while eight workers reconcile them. The reconciler is never
/// entered twice at once for one key, and no add is lost: each key's last reconcile sees the
/// last version its producers wrote before their last add.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn stress_one_reconcile_per_key_and_no_lost_adds() {
    const KEYS: usize = 4;
    const PRODUCERS: usize = 6;
    const ADDS: u64 = 500;

    struct Exclusive {
        versions: Vec<AtomicU64>,
        in_flight: Mutex<HashSet<Request>>,
        seen: Mutex<HashMap<Request, u64>>,
    }
    impl Reconcile for Exclusive {
        fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
            Box::pin(async move {
                assert!(
                    self.in_flight.lock().unwrap().insert(req.clone()),
                    "{req:?} is reconciled twice at once"
                );
                let i: usize = req.name.parse().unwrap();
                let version = self.versions[i].load(Ordering::SeqCst);
                self.seen.lock().unwrap().insert(req.clone(), version);
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_micros(50)).await;
                self.in_flight.lock().unwrap().remove(req);
                Ok(Outcome::Done)
            })
        }
    }

    let key = |i: usize| Request {
        namespace: None,
        name: i.to_string(),
    };
    let reconciler = Arc::new(Exclusive {
        versions: (0..KEYS).map(|_| AtomicU64::new(0)).collect(),
        in_flight: Mutex::new(HashSet::new()),
        seen: Mutex::new(HashMap::new()),
    });
    let (tx, key_stream) = keys::unbounded();
    let options = ControllerOptions {
        max_concurrent_reconciles: 8,
        rate_limiter: Arc::new(ItemExponentialBackoff::new(
            Duration::from_micros(10),
            Duration::from_millis(1),
        )),
        recover_panic: false,
    };
    let shutdown = CancellationToken::new();
    let driver = tokio::spawn(run(
        "stress".into(),
        key_stream,
        reconciler.clone(),
        options,
        shutdown.clone(),
    ));

    let producers: Vec<_> = (0..PRODUCERS)
        .map(|p| {
            let (tx, reconciler) = (tx.clone(), reconciler.clone());
            tokio::spawn(async move {
                for n in 0..ADDS {
                    let i = (p + n as usize) % KEYS;
                    reconciler.versions[i].fetch_add(1, Ordering::SeqCst);
                    tx.unbounded_send(key(i)).unwrap();
                    if n % 5 == 0 {
                        tokio::time::sleep(Duration::from_micros(200)).await;
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
            })
        })
        .collect();
    for p in producers {
        p.await.unwrap();
    }

    let caught_up = || {
        let seen = reconciler.seen.lock().unwrap();
        (0..KEYS).all(|i| seen.get(&key(i)) == Some(&reconciler.versions[i].load(Ordering::SeqCst)))
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        while !caught_up() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("every key's last reconcile sees its last add");

    shutdown.cancel();
    driver
        .await
        .expect("no reconcile overlapped another for its key")
        .unwrap();
}
