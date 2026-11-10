use std::sync::Arc;
use std::time::Duration;

use crossplane_core::errors::Result;
use crossplane_test::assert_result;
use futures::FutureExt;
use futures::future::BoxFuture;

use super::*;
use crate::reconciler::{Outcome, Reconcile, Request};

struct Predictable(Duration);

impl RateLimiter for Predictable {
    fn when(&self, _: &Request) -> Duration {
        self.0
    }
    fn forget(&self, _: &Request) {}
}

struct Returns(Outcome);

impl Reconcile for Returns {
    fn reconcile<'a>(&'a self, _: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        let outcome = self.0;
        async move { Ok(outcome) }.boxed()
    }
}

/// Go passes a nil inner reconciler, which would panic if called.
struct Unreachable;

impl Reconcile for Unreachable {
    fn reconcile<'a>(&'a self, _: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        panic!("the inner reconciler of a rate limited request must not be called")
    }
}

struct Case {
    name: &'static str,
    reason: &'static str,
    r: Box<dyn Fn() -> Box<dyn Reconcile>>,
    req: Request,
    want: Result<Outcome>,
}

#[tokio::test]
async fn reconcile() {
    let limited = || Request {
        namespace: None,
        name: "limited".into(),
    };
    let cases = vec![
        // go: pkg/ratelimiter TestReconcile/NotRateLimited
        Case {
            name: "NotRateLimited",
            reason: "Requests that are not rate limited should be passed to the inner Reconciler.",
            r: Box::new(|| {
                Box::new(Reconciler::new(
                    "test",
                    Returns(Outcome::Requeue),
                    Arc::new(Predictable(Duration::ZERO)),
                ))
            }),
            req: Request::default(),
            want: Ok(Outcome::Requeue),
        },
        // go: pkg/ratelimiter TestReconcile/RateLimited
        Case {
            name: "RateLimited",
            reason: "Requests that are rate limited should be requeued after the duration specified by the RateLimiter.",
            r: Box::new(|| {
                Box::new(Reconciler::new(
                    "test",
                    Unreachable,
                    Arc::new(Predictable(Duration::from_secs(8))),
                ))
            }),
            req: Request::default(),
            want: Ok(Outcome::RequeueAfter(Duration::from_secs(8))),
        },
        // go: pkg/ratelimiter TestReconcile/Returning
        Case {
            name: "Returning",
            reason: "Returning requests that were previously rate limited should be allowed through without further rate limiting.",
            r: Box::new(move || {
                let r = Reconciler::new(
                    "test",
                    Returns(Outcome::Requeue),
                    Arc::new(Predictable(Duration::from_secs(8))),
                );
                let first = r.reconcile(&limited()).now_or_never().unwrap();
                assert_eq!(first, Ok(Outcome::RequeueAfter(Duration::from_secs(8))));
                Box::new(r)
            }),
            req: limited(),
            want: Ok(Outcome::Requeue),
        },
    ];

    for tc in cases {
        let r = (tc.r)();
        let got = r.reconcile(&tc.req).await;
        assert_result(tc.name, tc.reason, &tc.want, &got);
    }
}

fn item(name: &str) -> Request {
    Request {
        namespace: None,
        name: name.into(),
    }
}

// go: k8s.io/client-go/util/workqueue TestItemExponentialFailureRateLimiter
#[test]
fn item_exponential_failure_rate_limiter() {
    let ms = Duration::from_millis;
    let limiter = ItemExponentialBackoff::new(ms(1), Duration::from_secs(1));
    let one = item("one");
    for want in [1, 2, 4, 8, 16] {
        assert_eq!(limiter.when(&one), ms(want));
    }
    assert_eq!(limiter.num_requeues(&one), 5);

    let two = item("two");
    assert_eq!(limiter.when(&two), ms(1));
    assert_eq!(limiter.when(&two), ms(2));
    assert_eq!(limiter.num_requeues(&two), 2);

    limiter.forget(&one);
    assert_eq!(limiter.num_requeues(&one), 0);
    assert_eq!(limiter.when(&one), ms(1));
}

// go: k8s.io/client-go/util/workqueue TestItemExponentialFailureRateLimiterOverFlow
#[test]
fn item_exponential_failure_rate_limiter_overflow() {
    let limiter = ItemExponentialBackoff::new(Duration::from_millis(1), Duration::from_secs(1000));
    let one = item("one");
    for _ in 0..5 {
        limiter.when(&one);
    }
    assert_eq!(limiter.when(&one), Duration::from_millis(32));

    let overflow1 = item("overflow1");
    for _ in 0..1000 {
        limiter.when(&overflow1);
    }
    assert_eq!(limiter.when(&overflow1), Duration::from_secs(1000));

    let minute = Duration::from_secs(60);
    let limiter = ItemExponentialBackoff::new(minute, 1000 * 60 * minute);
    let two = item("two");
    for _ in 0..2 {
        limiter.when(&two);
    }
    assert_eq!(limiter.when(&two), 4 * minute);

    let overflow2 = item("overflow2");
    for _ in 0..1000 {
        limiter.when(&overflow2);
    }
    assert_eq!(limiter.when(&overflow2), 1000 * 60 * minute);
}

#[test]
fn crossplane_per_item_backoff_is_1s_to_60s() {
    let limiter = new_controller();
    let mr = item("mr");
    let got: Vec<u64> = (0..8).map(|_| limiter.when(&mr).as_secs()).collect();
    assert_eq!(got, [1, 2, 4, 8, 16, 32, 60, 60], "pkg/ratelimiter/default.go:42-44");
    limiter.forget(&mr);
    assert_eq!(limiter.when(&mr), Duration::from_secs(1), "forget resets the backoff");
}

#[tokio::test(start_paused = true)]
async fn bucket_allows_burst_then_spaces_reservations() {
    let bucket = TokenBucket::new(10.0, 100);
    let any = item("any");
    for n in 0..100 {
        assert_eq!(bucket.when(&any), Duration::ZERO, "token {n} of the burst");
    }
    assert_eq!(bucket.when(&any), Duration::from_millis(100));
    assert_eq!(
        bucket.when(&any),
        Duration::from_millis(200),
        "each reservation takes a token even though the caller waits, so the debt grows"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    for n in 0..8 {
        assert_eq!(
            bucket.when(&any),
            Duration::ZERO,
            "a second earns 10 tokens, 2 repay the debt, so token {n} of 8 is free"
        );
    }
    assert_eq!(bucket.when(&any), Duration::from_millis(100));
}

#[tokio::test(start_paused = true)]
async fn bucket_refill_is_capped_at_burst() {
    let bucket = TokenBucket::new(1.0, 10);
    let any = item("any");
    for _ in 0..10 {
        bucket.when(&any);
    }
    tokio::time::advance(Duration::from_secs(3600)).await;
    for n in 0..10 {
        assert_eq!(bucket.when(&any), Duration::ZERO, "token {n} after a long idle");
    }
    assert_eq!(bucket.when(&any), Duration::from_secs(1));
}

#[test]
fn bucket_with_zero_rate_never_grants() {
    assert_eq!(TokenBucket::new(0.0, 0).when(&item("any")), INF_DURATION);
}

/// Go's wrapper asks the bucket once per pass: a request held back once is let through without a
/// second reservation when it returns, so it pays exactly one token.
#[tokio::test(start_paused = true)]
async fn global_limiter_charges_a_held_back_request_once() {
    let global = Arc::new(TokenBucket::new(1.0, 1));
    let r = Reconciler::new("test", Returns(Outcome::Done), global.clone());
    let (a, b) = (item("a"), item("b"));

    assert_eq!(r.reconcile(&a).await, Ok(Outcome::Done));
    assert_eq!(r.reconcile(&b).await, Ok(Outcome::RequeueAfter(Duration::from_secs(1))));
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(r.reconcile(&b).await, Ok(Outcome::Done), "b returns and passes");
    assert_eq!(
        global.when(&a),
        Duration::from_secs(1),
        "b's first pass took the token of the second just passed and its return took none; two would mean a 2s wait"
    );
}
