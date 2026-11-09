//! `pkg/ratelimiter`, plus the client-go workqueue rate limiters it is built from. Three layers,
//! as in Go:
//!
//! 1. [`limit_rest_config`]: the client-side QPS throttle on every API request.
//! 2. [`new_global`] and [`Reconciler`]: one token bucket across all controllers, checked before
//!    the inner reconcile runs.
//! 3. [`new_controller`]: per-item exponential backoff that the work queue applies on
//!    `Outcome::Requeue` and on errors.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::errors::Result;
use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::time::Instant;

use crate::manager::RestConfig;
use crate::reconciler::{Outcome, Reconcile, Request};

/// client-go `workqueue.TypedRateLimiter[reconcile.Request]`.
pub trait RateLimiter: Send + Sync + 'static {
    /// How long the item should wait. Each call counts as one more failure.
    fn when(&self, item: &Request) -> Duration;
    /// Stops tracking the item, resetting its backoff.
    fn forget(&self, item: &Request);
}

/// Go `NewController`: per-item exponential backoff from 1s to 60s
/// (`pkg/ratelimiter/default.go:42`).
pub fn new_controller() -> Arc<dyn RateLimiter> {
    Arc::new(ItemExponentialBackoff::new(
        Duration::from_secs(1),
        Duration::from_secs(60),
    ))
}

/// Go `NewGlobal`: a token bucket of `rps` per second with burst `rps * 10`, shared by every
/// controller (`pkg/ratelimiter/default.go:32`).
pub fn new_global(rps: u32) -> Arc<dyn RateLimiter> {
    Arc::new(TokenBucket::new(rps, rps * 10))
}

/// Go `LimitRESTConfig`: QPS `rps * 5`, burst `rps * 10` (`pkg/ratelimiter/default.go:48`).
pub fn limit_rest_config(config: RestConfig, rps: u32) -> RestConfig {
    config.throttled((rps * 5) as f32, rps * 10)
}

/// client-go `ItemExponentialFailureRateLimiter`: `base * 2^failures`, capped at `max`
/// (`util/workqueue/default_rate_limiters.go:116-135`).
pub(crate) struct ItemExponentialBackoff {
    base: Duration,
    max: Duration,
    failures: Mutex<HashMap<Request, i32>>,
}

impl ItemExponentialBackoff {
    pub(crate) fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            failures: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn num_requeues(&self, item: &Request) -> i32 {
        self.failures.lock().unwrap().get(item).copied().unwrap_or(0)
    }
}

impl RateLimiter for ItemExponentialBackoff {
    fn when(&self, item: &Request) -> Duration {
        let exp = {
            let mut failures = self.failures.lock().unwrap();
            let exp = failures.entry(item.clone()).or_insert(0);
            *exp += 1;
            *exp - 1
        };
        // Go computes in float64 so that a large failure count saturates instead of overflowing.
        let backoff = self.base.as_nanos() as f64 * 2f64.powi(exp);
        if backoff > i64::MAX as f64 {
            return self.max;
        }
        Duration::from_nanos(backoff as u64).min(self.max)
    }

    fn forget(&self, item: &Request) {
        self.failures.lock().unwrap().remove(item);
    }
}

/// What `golang.org/x/time/rate` returns for a reservation it cannot grant.
const INF_DURATION: Duration = Duration::from_nanos(i64::MAX as u64);

/// client-go `BucketRateLimiter` over `golang.org/x/time/rate.Limiter`. `when` is
/// `Limiter.Reserve().Delay()`: it always takes a token, letting the bucket go negative, and never
/// gives it back (`util/workqueue/default_rate_limiters.go:68-70`, `rate/rate.go:337-381`).
struct TokenBucket {
    rps: f64,
    burst: u32,
    state: Mutex<Bucket>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(rps: u32, burst: u32) -> Self {
        Self {
            rps: f64::from(rps),
            burst,
            state: Mutex::new(Bucket {
                tokens: f64::from(burst),
                last: Instant::now(),
            }),
        }
    }

    /// Go `Limit.durationFromTokens`.
    fn duration_from_tokens(&self, tokens: f64) -> Duration {
        if self.rps <= 0.0 {
            return INF_DURATION;
        }
        let nanos = (tokens / self.rps) * 1e9;
        if nanos > i64::MAX as f64 {
            return INF_DURATION;
        }
        Duration::from_nanos(nanos as u64)
    }
}

impl RateLimiter for TokenBucket {
    fn when(&self, _: &Request) -> Duration {
        if self.burst < 1 {
            return INF_DURATION;
        }
        let now = Instant::now();
        let mut b = self.state.lock().unwrap();
        let elapsed = now.saturating_duration_since(b.last);
        let tokens = (b.tokens + elapsed.as_secs_f64() * self.rps).min(f64::from(self.burst)) - 1.0;
        let wait = if tokens < 0.0 {
            self.duration_from_tokens(-tokens)
        } else {
            Duration::ZERO
        };
        b.tokens = tokens;
        b.last = now;
        wait
    }

    fn forget(&self, _: &Request) {}
}

/// Go `ratelimiter.Reconciler`: when the global bucket says wait, the inner reconciler is not
/// called and the result is `Ok(Outcome::RequeueAfter(wait))`. A request it held back once passes
/// without asking the bucket again when it returns, so it pays one token per pass, not two.
pub struct Reconciler<R> {
    inner: R,
    limiter: Arc<dyn RateLimiter>,
    limited: Mutex<HashSet<Request>>,
}

impl<R: Reconcile> Reconciler<R> {
    /// Go keys the limiter by `name + request`. [`RateLimiter`] is keyed by request alone, which
    /// is the same for the global bucket because a bucket ignores the item.
    pub fn new(_name: &str, inner: R, limiter: Arc<dyn RateLimiter>) -> Self {
        Self {
            inner,
            limiter,
            limited: Mutex::new(HashSet::new()),
        }
    }

    /// Go `Reconciler.when` (`pkg/ratelimiter/reconciler.go:69-98`).
    fn when(&self, req: &Request) -> Duration {
        if self.limited.lock().unwrap().remove(req) {
            return Duration::ZERO;
        }
        let d = self.limiter.when(req);
        if d != Duration::ZERO {
            self.limited.lock().unwrap().insert(req.clone());
        }
        d
    }
}

impl<R: Reconcile> Reconcile for Reconciler<R> {
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        async move {
            let d = self.when(req);
            if d > Duration::ZERO {
                return Ok(Outcome::RequeueAfter(d));
            }
            self.limiter.forget(req);
            self.inner.reconcile(req).await
        }
        .boxed()
    }
}

#[cfg(test)]
#[path = "ratelimiter_test.rs"]
mod tests;
