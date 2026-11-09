//! `pkg/ratelimiter`, plus the client-go workqueue rate limiters it is built from. Three layers,
//! as in Go:
//!
//! 1. [`limit_rest_config`]: the client-side QPS throttle on every API request.
//! 2. [`new_global`] and [`Reconciler`]: one token bucket across all controllers, checked before
//!    the inner reconcile runs.
//! 3. [`new_controller`]: per-item exponential backoff that the work queue applies on
//!    `Outcome::Requeue` and on errors.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossplane_core::errors::Result;
use futures::FutureExt;
use futures::future::BoxFuture;

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
    Arc::new(TokenBucket { rps, burst: rps * 10 })
}

/// Go `LimitRESTConfig`: QPS `rps * 5`, burst `rps * 10` (`pkg/ratelimiter/default.go:48`).
pub fn limit_rest_config(config: RestConfig, rps: u32) -> RestConfig {
    config.throttled((rps * 5) as f32, rps * 10)
}

/// client-go `ItemExponentialFailureRateLimiter`: `base * 2^failures`, capped at `max`.
pub(crate) struct ItemExponentialBackoff {
    base: Duration,
    max: Duration,
    failures: Mutex<HashMap<Request, u32>>,
}

impl ItemExponentialBackoff {
    pub(crate) fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            failures: Mutex::new(HashMap::new()),
        }
    }
}

impl RateLimiter for ItemExponentialBackoff {
    fn when(&self, item: &Request) -> Duration {
        let _ = (self.base, self.max, &self.failures);
        todo!("delay for this failure of {item:?}, then count it")
    }

    fn forget(&self, item: &Request) {
        todo!("drop the failure count of {item:?}")
    }
}

/// client-go `BucketRateLimiter`.
struct TokenBucket {
    rps: u32,
    burst: u32,
}

impl RateLimiter for TokenBucket {
    fn when(&self, _: &Request) -> Duration {
        let _ = (self.rps, self.burst);
        // TODO: OPEN (docs/design/core.md): Go's `When` reserves the token even when the caller
        // then skips the reconcile. Zero when a token is available now, else the wait.
        todo!()
    }

    fn forget(&self, _: &Request) {}
}

/// Go `ratelimiter.Reconciler`: when the global bucket says wait, the inner reconciler is not
/// called and the result is `Ok(Outcome::RequeueAfter(wait))`.
pub struct Reconciler<R> {
    name: String,
    inner: R,
    limiter: Arc<dyn RateLimiter>,
}

impl<R: Reconcile> Reconciler<R> {
    pub fn new(name: &str, inner: R, limiter: Arc<dyn RateLimiter>) -> Self {
        Self {
            name: name.to_owned(),
            inner,
            limiter,
        }
    }
}

impl<R: Reconcile> Reconcile for Reconciler<R> {
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        async move {
            let _ = (&self.name, &self.inner, &self.limiter, req);
            todo!("pkg/ratelimiter/reconciler.go:54")
        }
        .boxed()
    }
}
