//! Runs one reconciler over a plain stream of keys. It knows nothing of kube, so a test feeds it
//! keys directly; `Controller::keys` builds the real stream from informers.

use std::any::Any;
use std::panic::{AssertUnwindSafe, resume_unwind};
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::errors::{Error, Result};
use futures::{FutureExt, Stream, StreamExt};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use super::ControllerOptions;
use super::workqueue::WorkQueue;
use crate::reconciler::{Outcome, Reconcile, Request};

/// Feeds `keys` into a work queue and runs `max_concurrent_reconciles` workers until `shutdown`,
/// as controller-runtime's `Controller.Start` does. On shutdown the queue stops handing out
/// items and in-flight reconciles finish. A reconciler panic with `recover_panic` off is resumed
/// here, the way an unrecovered panic in a Go worker ends the process.
pub(super) async fn run(
    name: String,
    keys: impl Stream<Item = Request> + Send + 'static,
    reconciler: Arc<dyn Reconcile>,
    options: ControllerOptions,
    shutdown: CancellationToken,
) -> Result<()> {
    let queue = Arc::new(WorkQueue::new(options.rate_limiter.clone()));
    let span = tracing::info_span!("controller", controller = %name);

    let feeder = tokio::spawn({
        let queue = queue.clone();
        async move {
            let mut keys = std::pin::pin!(keys);
            while let Some(key) = keys.next().await {
                queue.add(key);
            }
        }
    });

    let mut workers = JoinSet::new();
    for _ in 0..options.max_concurrent_reconciles.max(1) {
        let worker = work(queue.clone(), reconciler.clone(), options.recover_panic);
        workers.spawn(worker.instrument(span.clone()));
    }

    let panicked = tokio::select! {
        () = shutdown.cancelled() => None,
        Some(Err(err)) = workers.join_next() => err.try_into_panic().ok(),
    };
    queue.shut_down();
    feeder.abort();
    if let Some(panic) = panicked {
        resume_unwind(panic);
    }
    while let Some(joined) = workers.join_next().await {
        if let Err(err) = joined
            && let Ok(panic) = err.try_into_panic()
        {
            resume_unwind(panic);
        }
    }
    Ok(())
}

/// One worker: take a request, reconcile it, tell the queue what happened.
async fn work(queue: Arc<WorkQueue>, reconciler: Arc<dyn Reconcile>, recover_panic: bool) {
    while let Some(request) = queue.get().await {
        let _held = Held {
            queue: &queue,
            request: &request,
        };
        let result = reconcile(&*reconciler, &request, recover_panic).await;
        if let Err(err) = &result {
            tracing::error!(error = %err, "Reconciler error");
        }
        match queue_op(&result) {
            QueueOp::Forget => queue.forget(&request),
            QueueOp::ForgetAndAddAfter(delay) => {
                queue.forget(&request);
                queue.add_after(request.clone(), delay);
            }
            QueueOp::AddRateLimited => queue.add_rate_limited(request.clone()),
        }
    }
}

/// Releases the item when the worker is done with it, also when the reconcile panics or the
/// worker's future is dropped mid-reconcile, so a key is never left held. Go defers `Done` for
/// the same reason (`pkg/internal/controller/controller.go:432`).
struct Held<'a> {
    queue: &'a WorkQueue,
    request: &'a Request,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        self.queue.done(self.request);
    }
}

/// controller-runtime `Controller.Reconcile`: a panic becomes an error when `recover_panic` is
/// set (`pkg/internal/controller/controller.go:194-208`).
async fn reconcile(reconciler: &dyn Reconcile, request: &Request, recover_panic: bool) -> Result<Outcome> {
    let run = AssertUnwindSafe(async { reconciler.reconcile(request).await }).catch_unwind();
    match run.await {
        Ok(result) => result,
        Err(panic) if recover_panic => Err(Error::new(format!("panic: {} [recovered]", panic_message(&*panic)))),
        Err(panic) => resume_unwind(panic),
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    if let Some(s) = panic.downcast_ref::<&str>() {
        s
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s
    } else {
        "Box<dyn Any>"
    }
}

#[derive(Debug, PartialEq, Eq)]
enum QueueOp {
    Forget,
    ForgetAndAddAfter(Duration),
    AddRateLimited,
}

/// The one place Go's four outcomes meet the queue: controller-runtime `reconcileHandler`
/// (`pkg/internal/controller/controller.go:477-513`). Go tests `RequeueAfter > 0`, so a zero
/// delay is plain success.
fn queue_op(result: &Result<Outcome>) -> QueueOp {
    match result {
        Err(_) | Ok(Outcome::Requeue) => QueueOp::AddRateLimited,
        Ok(Outcome::RequeueAfter(delay)) if *delay > Duration::ZERO => QueueOp::ForgetAndAddAfter(*delay),
        Ok(Outcome::RequeueAfter(_) | Outcome::Done) => QueueOp::Forget,
    }
}

#[cfg(test)]
#[path = "driver_test.rs"]
mod tests;
