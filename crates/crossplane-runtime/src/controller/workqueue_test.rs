//! client-go's queue tests and controller-runtime's per-object priority queue tests, run against
//! [`WorkQueue`]. Timing tests run on tokio's paused clock, where `advance` plays the part of Go's
//! fake clock `Step`, so every delay is exact.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use tokio::time::advance;

use super::*;
use crate::ratelimiter::ItemExponentialBackoff;

fn key(name: &str) -> Request {
    Request {
        namespace: None,
        name: name.into(),
    }
}

fn queue() -> Arc<WorkQueue> {
    Arc::new(WorkQueue::new(Arc::new(ItemExponentialBackoff::new(
        Duration::from_millis(5),
        Duration::from_secs(1000),
    ))))
}

/// The item `get` hands out now, or `None` when it would wait.
fn get_now(q: &WorkQueue) -> Option<Request> {
    q.get().now_or_never().flatten()
}

/// Fails, instead of hanging, when a wakeup is missed.
async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), f)
        .await
        .expect("the queue wakes its waiters within a minute")
}

/// Runs other tasks to their next await without moving the paused clock.
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

/// Go also asserts `Len() == 0` after shutdown. The priority queue does not drain on shutdown,
/// so that part is left out; consumers stopping and nothing added after shutdown are kept.
// go: k8s.io/client-go/util/workqueue TestBasic
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn basic() {
    let q = queue();
    let producers: Vec<_> = (0..50)
        .map(|i| {
            let q = q.clone();
            tokio::spawn(async move {
                for _ in 0..50 {
                    q.add(key(&i.to_string()));
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
        })
        .collect();
    let consumers: Vec<_> = (0..10)
        .map(|_| {
            let q = q.clone();
            tokio::spawn(async move {
                while let Some(item) = q.get().await {
                    assert_ne!(item, key("added after shutdown!"), "Got an item added after shutdown.");
                    tokio::time::sleep(Duration::from_millis(3)).await;
                    q.done(&item);
                }
            })
        })
        .collect();
    for p in producers {
        p.await.unwrap();
    }
    q.shut_down();
    q.add(key("added after shutdown!"));
    for c in consumers {
        bounded(c).await.unwrap();
    }
}

/// Every worker re-adds an item the first time it sees it, so every item must be processed at
/// least twice: a re-add while processing is never lost. Go's `Len() == 0` after shutdown relies
/// on the classic queue draining and is left out, as in `basic`.
// go: k8s.io/client-go/util/workqueue TestAddWhileProcessing
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn add_while_processing() {
    let q = queue();
    let producers: Vec<_> = (0..50)
        .map(|i| {
            let q = q.clone();
            tokio::spawn(async move { q.add(key(&i.to_string())) })
        })
        .collect();
    let processed = Arc::new(Mutex::new(HashMap::<Request, u32>::new()));
    let consumers: Vec<_> = (0..10)
        .map(|_| {
            let (q, processed) = (q.clone(), processed.clone());
            tokio::spawn(async move {
                let mut counters = HashMap::<Request, u32>::new();
                while let Some(item) = q.get().await {
                    let n = counters.entry(item.clone()).or_default();
                    *n += 1;
                    if *n < 2 {
                        q.add(item.clone());
                    }
                    *processed.lock().unwrap().entry(item.clone()).or_default() += 1;
                    q.done(&item);
                }
            })
        })
        .collect();
    for p in producers {
        p.await.unwrap();
    }
    let twice = || {
        let processed = processed.lock().unwrap();
        processed.len() == 50 && processed.values().all(|n| *n >= 2)
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        while !twice() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("every item is processed at least twice");
    q.shut_down();
    for c in consumers {
        bounded(c).await.unwrap();
    }
}

// go: k8s.io/client-go/util/workqueue TestLen
#[test]
fn len() {
    let q = queue();
    q.add(key("foo"));
    assert_eq!(q.len(), 1);
    q.add(key("bar"));
    assert_eq!(q.len(), 2);
    q.add(key("foo"));
    assert_eq!(q.len(), 2, "should not increase the queue length");
}

// go: k8s.io/client-go/util/workqueue TestReinsert
#[test]
fn reinsert() {
    let q = queue();
    q.add(key("foo"));
    let i = get_now(&q).unwrap();
    assert_eq!(i, key("foo"));
    q.add(i.clone());
    q.done(&i);
    let i = get_now(&q).expect("It should be back on the queue");
    assert_eq!(i, key("foo"));
    q.done(&i);
    assert_eq!(q.len(), 0, "Expected queue to be empty");
}

/// Go also checks that the classic FIFO was `Touch`ed; the priority queue has no such hook.
// go: k8s.io/client-go/util/workqueue TestCollapse
#[test]
fn collapse() {
    let q = queue();
    q.add(key("bar"));
    q.add(key("bar"));
    let i = get_now(&q).unwrap();
    assert_eq!(i, key("bar"));
    q.done(&i);
    assert_eq!(q.len(), 0, "There should be no more objects in the queue");
}

// go: k8s.io/client-go/util/workqueue TestCollapseWhileProcessing
#[tokio::test(start_paused = true)]
async fn collapse_while_processing() {
    let q = queue();
    q.add(key("foo"));
    let i = get_now(&q).unwrap();
    assert_eq!(i, key("foo"));
    q.add(key("foo"));
    q.add(key("foo"));

    let worker = tokio::spawn({
        let q = q.clone();
        async move {
            let i = q.get().await.unwrap();
            assert_eq!(i, key("foo"));
            q.done(&i);
        }
    });
    settle().await;
    assert!(!worker.is_finished(), "worker should be blocked until we are done");
    q.done(&key("foo"));
    bounded(worker).await.unwrap();
    assert_eq!(q.len(), 0, "There should be no more objects in the queue");
}

// go: k8s.io/client-go/util/workqueue TestNoQueueDrainageUsingShutDown
#[test]
fn no_queue_drainage_using_shut_down() {
    let q = queue();
    q.add(key("foo"));
    q.add(key("bar"));
    get_now(&q).unwrap();
    get_now(&q).unwrap();
    q.shut_down();
}

// go: k8s.io/client-go/util/workqueue TestSimpleQueue
#[tokio::test(start_paused = true)]
async fn simple_queue() {
    let q = queue();
    q.add_after(key("foo"), Duration::from_millis(50));
    assert_eq!(get_now(&q), None, "should not have added");

    advance(Duration::from_millis(60)).await;
    let item = get_now(&q).expect("should have added");
    q.done(&item);

    advance(Duration::from_secs(10)).await;
    assert_eq!(get_now(&q), None, "should not have added");
}

// go: k8s.io/client-go/util/workqueue TestDeduping
#[tokio::test(start_paused = true)]
async fn deduping() {
    let q = queue();
    q.add_after(key("foo"), Duration::from_millis(50));
    q.add_after(key("foo"), Duration::from_millis(70));
    assert_eq!(get_now(&q), None, "should not have added");

    advance(Duration::from_millis(60)).await;
    let item = get_now(&q).expect("should have added");
    q.done(&item);

    advance(Duration::from_millis(20)).await;
    assert_eq!(get_now(&q), None, "should not have added");

    q.add_after(key("foo"), Duration::from_millis(50));
    q.add_after(key("foo"), Duration::from_millis(30));
    assert_eq!(get_now(&q), None, "should not have added");

    advance(Duration::from_millis(40)).await;
    let item = get_now(&q).expect("should have added");
    q.done(&item);

    advance(Duration::from_millis(20)).await;
    assert_eq!(get_now(&q), None, "should not have added");
}

// go: k8s.io/client-go/util/workqueue TestAddTwoFireEarly
#[tokio::test(start_paused = true)]
async fn add_two_fire_early() {
    let q = queue();
    q.add_after(key("foo"), Duration::from_secs(1));
    q.add_after(key("bar"), Duration::from_millis(50));
    assert_eq!(get_now(&q), None, "should not have added");

    advance(Duration::from_millis(60)).await;
    assert_eq!(get_now(&q), Some(key("bar")));

    q.add_after(key("baz"), Duration::from_secs(2));
    advance(Duration::from_secs(1)).await;
    assert_eq!(get_now(&q), Some(key("foo")));

    advance(Duration::from_secs(2)).await;
    assert_eq!(get_now(&q), Some(key("baz")));
}

// go: k8s.io/client-go/util/workqueue TestCopyShifting
#[tokio::test(start_paused = true)]
async fn copy_shifting() {
    let q = queue();
    q.add_after(key("foo"), Duration::from_secs(1));
    q.add_after(key("bar"), Duration::from_millis(500));
    q.add_after(key("baz"), Duration::from_millis(250));
    assert_eq!(get_now(&q), None, "should not have added");

    advance(Duration::from_secs(2)).await;
    assert_eq!(get_now(&q), Some(key("baz")));
    assert_eq!(get_now(&q), Some(key("bar")));
    assert_eq!(get_now(&q), Some(key("foo")));
}

/// Go reads each delayed entry's ready time off the delaying queue's channel. Here the item is
/// checked to be not ready one nanosecond before that time and ready at it.
// go: k8s.io/client-go/util/workqueue TestRateLimitingQueue
#[tokio::test(start_paused = true)]
async fn rate_limiting_queue() {
    let q = WorkQueue::new(Arc::new(ItemExponentialBackoff::new(
        Duration::from_millis(1),
        Duration::from_secs(1),
    )));
    let ready_after = async |item: &Request, delay: Duration| {
        advance(delay - Duration::from_nanos(1)).await;
        assert_eq!(get_now(&q), None, "{item:?} is not ready before {delay:?}");
        advance(Duration::from_nanos(1)).await;
        assert_eq!(get_now(&q).as_ref(), Some(item), "{item:?} is ready after {delay:?}");
        q.done(item);
    };
    let ms = Duration::from_millis;
    let (one, two) = (key("one"), key("two"));

    q.add_rate_limited(one.clone());
    ready_after(&one, ms(1)).await;
    q.add_rate_limited(one.clone());
    ready_after(&one, ms(2)).await;

    q.add_rate_limited(two.clone());
    ready_after(&two, ms(1)).await;
    q.add_rate_limited(two.clone());
    ready_after(&two, ms(2)).await;

    q.forget(&one);
    q.add_rate_limited(one.clone());
    ready_after(&one, ms(1)).await;
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/returns an item
#[test]
fn returns_an_item() {
    let q = queue();
    q.add(key("foo"));
    assert_eq!(get_now(&q), Some(key("foo")));
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/returns items in order
#[test]
fn returns_items_in_order() {
    let q = queue();
    q.add(key("foo"));
    q.add(key("bar"));
    assert_eq!(get_now(&q), Some(key("foo")));
    assert_eq!(get_now(&q), Some(key("bar")));
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/doesn't return an item that is currently locked
#[test]
fn does_not_return_a_locked_item() {
    let q = queue();
    q.add(key("foo"));
    assert_eq!(get_now(&q), Some(key("foo")));
    q.add(key("foo"));
    q.add(key("bar"));
    assert_eq!(get_now(&q), Some(key("bar")));
    assert_eq!(q.len(), 1, "foo waits, ready, for its worker to finish");
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/returns an item as soon as its unlocked
#[test]
fn returns_an_item_as_soon_as_it_is_unlocked() {
    let q = queue();
    q.add(key("foo"));
    assert_eq!(get_now(&q), Some(key("foo")));
    q.add(key("foo"));
    q.add(key("bar"));
    assert_eq!(get_now(&q), Some(key("bar")));
    q.add(key("baz"));
    q.done(&key("foo"));
    assert_eq!(get_now(&q), Some(key("foo")), "foo was added before baz");
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/de-duplicates items
#[test]
fn de_duplicates_items() {
    let q = queue();
    q.add(key("foo"));
    q.add(key("foo"));
    assert_eq!(q.len(), 1);
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/enqueues a locked item
#[test]
fn enqueues_a_locked_item() {
    let q = queue();
    q.add(key("foo"));
    assert_eq!(q.len(), 1);
    assert_eq!(get_now(&q), Some(key("foo")));
    q.add(key("foo"));
    assert_eq!(q.len(), 1);
    q.done(&key("foo"));
    assert_eq!(get_now(&q), Some(key("foo")));
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/retains the lowest after duration
#[test]
fn retains_the_lowest_after_duration() {
    let q = queue();
    q.add_after(key("foo"), Duration::ZERO);
    q.add_after(key("foo"), Duration::from_secs(3600));
    assert_eq!(get_now(&q), Some(key("foo")));
    assert_eq!(q.len(), 0);
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/doesn't include non-ready items in Len()
#[test]
fn does_not_count_waiting_items_in_len() {
    let q = queue();
    q.add_after(key("foo"), Duration::from_secs(60));
    q.add(key("baz"));
    q.add_after(key("bar"), Duration::from_secs(60));
    q.add(key("bal"));
    assert_eq!(q.len(), 2);
}

// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/Get from priority queue might get stuck when the priority queue is shut down
#[tokio::test]
async fn get_after_shut_down_returns_none() {
    let q = queue();
    q.add(key("baz"));
    q.shut_down();
    q.add_after(key("foo"), Duration::from_secs(1));
    assert_eq!(bounded(q.get()).await, None);
    assert_eq!(bounded(q.get()).await, None);
}

/// Go checks the depth metric; `len` is the same number. Added here: the cancelled delay does
/// not fire a second time, which is where client-go's delaying queue differs.
// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/updates metrics correctly for an item that gets initially added with after and then without
#[tokio::test(start_paused = true)]
async fn add_without_delay_makes_a_waiting_item_ready() {
    let q = queue();
    q.add_after(key("foo"), Duration::from_secs(3600));
    assert_eq!(q.len(), 0);
    q.add(key("foo"));
    assert_eq!(q.len(), 1);
    assert_eq!(get_now(&q), Some(key("foo")));
    assert_eq!(q.len(), 0);
    q.done(&key("foo"));

    advance(Duration::from_secs(7200)).await;
    assert_eq!(
        get_now(&q),
        None,
        "the hour's delay was replaced, not kept beside the add"
    );
}

#[tokio::test(start_paused = true)]
async fn a_shorter_delay_replaces_a_longer_one() {
    let q = queue();
    q.add_after(key("foo"), Duration::from_secs(60));
    q.add_rate_limited(key("foo"));
    advance(Duration::from_millis(5)).await;
    assert_eq!(get_now(&q), Some(key("foo")), "the 5ms backoff wins over the minute");
    q.done(&key("foo"));
    advance(Duration::from_secs(120)).await;
    assert_eq!(get_now(&q), None, "and the minute is gone");
}

#[tokio::test(start_paused = true)]
async fn add_rate_limited_after_shut_down_counts_no_failure() {
    let limiter = Arc::new(ItemExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(60),
    ));
    let q = WorkQueue::new(limiter.clone());
    q.shut_down();
    q.add_rate_limited(key("foo"));
    assert_eq!(limiter.when(&key("foo")), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_blocked_get_wakes_when_a_delay_expires() {
    let q = queue();
    let got = tokio::spawn({
        let q = q.clone();
        async move { (q.get().await, Instant::now()) }
    });
    settle().await;
    let start = Instant::now();
    q.add_after(key("foo"), Duration::from_secs(30));
    let (item, at) = bounded(got).await.unwrap();
    assert_eq!(item, Some(key("foo")));
    assert_eq!(at - start, Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn a_blocked_get_wakes_on_shut_down() {
    let q = queue();
    let got = tokio::spawn({
        let q = q.clone();
        async move { q.get().await }
    });
    settle().await;
    q.shut_down();
    assert_eq!(bounded(got).await.unwrap(), None);
}

/// Go draws a random number per add to choose a short delay or a priority; priorities are not
/// ported, so a fixed spread chooses a delay for one add in ten.
// go: sigs.k8s.io/controller-runtime/pkg/controller/priorityqueue TestControllerWorkqueue/returns many items
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn returns_many_items() {
    let q = queue();
    for round in 0..20u64 {
        for i in 0..1000u64 {
            let rn = (i * 7919 + round * 104_729) % 100;
            if rn < 10 {
                q.add_after(key(&format!("foo{i}")), Duration::from_millis(rn));
            } else {
                q.add(key(&format!("foo{i}")));
            }
        }
        let getters: Vec<_> = (0..100)
            .map(|_| {
                let q = q.clone();
                tokio::spawn(async move {
                    for _ in 0..10 {
                        let item = q.get().await.unwrap();
                        q.done(&item);
                    }
                })
            })
            .collect();
        for g in getters {
            bounded(g).await.unwrap();
        }
    }
}

#[test]
#[ignore = "timing probe: run with --release --ignored --nocapture"]
fn probe_add_after_overshoot() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let q = queue();
        let mut sleep_over = Vec::new();
        let mut queue_over = Vec::new();
        for i in 0..60u64 {
            tokio::time::sleep(Duration::from_micros(137 * (i % 7) + 50)).await;
            let delay = Duration::from_millis(50) + Duration::from_micros((i * 9973) % 1000);
            let at = tokio::time::Instant::now() + delay;
            if i % 2 == 0 {
                tokio::time::sleep_until(at).await;
                sleep_over.push((tokio::time::Instant::now() - at).as_secs_f64() * 1000.0);
            } else {
                q.add_after(key("a"), delay);
                let got = q.get().await.unwrap();
                queue_over.push((tokio::time::Instant::now() - at).as_secs_f64() * 1000.0);
                q.done(&got);
            }
        }
        for (name, mut v) in [("sleep_until", sleep_over), ("add_after+get", queue_over)] {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            println!("{name}: n={} mean={mean:.3}ms median={:.3}ms", v.len(), v[v.len() / 2]);
        }
    });
}
