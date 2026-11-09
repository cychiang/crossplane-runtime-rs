//! Reconcilers and the outcome type the driver consumes.

pub mod customresourcesgate;
pub mod managed;
pub mod providerconfig;

use std::time::Duration;

use crossplane_core::errors::Result;
use crossplane_core::resource::ObjectKey;
use futures::future::BoxFuture;

/// Go `reconcile.Request`.
pub type Request = ObjectKey;

/// A successful reconcile's instruction to the queue. With `Err` this gives Go's four outcomes
/// and nothing else. Go's `Result{Requeue: true, RequeueAfter: d}` and "result plus error" are
/// not representable; controller-runtime already collapses both (`RequeueAfter` wins, an error
/// discards the result).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// `Result{}`: forget the item's backoff, do not requeue.
    Done,
    /// `Result{Requeue: true}`: requeue through the per-item rate limiter.
    Requeue,
    /// `Result{RequeueAfter: d}`: forget the backoff, requeue after `d`.
    RequeueAfter(Duration),
}

/// Go `reconcile.Reconciler`. Object safe, so wrappers such as `ratelimiter::Reconciler` compose
/// without generics leaking upward.
pub trait Reconcile: Send + Sync + 'static {
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>>;
}

/// Go `errors.SilentlyRequeueOnConflict` (parity spec X1). It lives here, not in
/// `crossplane_core::errors`, because it names [`Outcome`].
pub fn silently_requeue_on_conflict(result: Result<Outcome>) -> Result<Outcome> {
    match result {
        Err(err) if err.is_conflict() => Ok(Outcome::Requeue),
        other => other,
    }
}
