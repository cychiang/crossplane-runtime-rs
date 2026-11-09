//! Runs one reconciler over a plain stream of keys. It knows nothing of kube, so a test feeds it
//! keys directly; `Controller::keys` builds the real stream from informers.

use std::sync::Arc;
use std::time::Duration;

use crossplane_core::errors::Result;
use futures::Stream;
use tokio_util::sync::CancellationToken;

use super::ControllerOptions;
use super::workqueue::WorkQueue;
use crate::reconciler::{Outcome, Reconcile, Request};

/// Feeds `keys` into a work queue and runs `max_concurrent_reconciles` workers until `shutdown`.
/// In-flight reconciles finish first.
pub(super) async fn run(
    name: String,
    keys: impl Stream<Item = Request> + Send + 'static,
    reconciler: Arc<dyn Reconcile>,
    options: ControllerOptions,
    shutdown: CancellationToken,
) -> Result<()> {
    let queue = WorkQueue::new(options.rate_limiter.clone());
    let _ = (
        std::pin::pin!(keys),
        work(&queue, &*reconciler, options.recover_panic),
        shutdown,
    );
    // TODO: spawn: keys -> queue.add; spawn max_concurrent_reconciles x `work` inside a span
    // `controller = name`; on shutdown: queue.shut_down(), join the workers.
    todo!("run controller {name}")
}

/// One worker: take a request, reconcile it, tell the queue what happened.
async fn work(queue: &WorkQueue, reconciler: &dyn Reconcile, recover_panic: bool) {
    while let Some(request) = queue.get().await {
        // TODO: with recover_panic, a panic in the reconciler becomes an error, as
        // controller-runtime's RecoverPanic does.
        let _ = recover_panic;
        let result = reconciler.reconcile(&request).await;
        match queue_op(&result) {
            QueueOp::Forget => queue.forget(&request),
            QueueOp::ForgetAndAddAfter(delay) => {
                queue.forget(&request);
                queue.add_after(request.clone(), delay);
            }
            QueueOp::AddRateLimited => queue.add_rate_limited(request.clone()),
        }
        queue.done(&request);
    }
}

#[derive(Debug, PartialEq, Eq)]
enum QueueOp {
    Forget,
    ForgetAndAddAfter(Duration),
    AddRateLimited,
}

/// The one place Go's four outcomes meet the queue: controller-runtime `reconcileHandler`
/// (`pkg/internal/controller/controller.go:482-513`).
fn queue_op(result: &Result<Outcome>) -> QueueOp {
    match result {
        Err(_) | Ok(Outcome::Requeue) => QueueOp::AddRateLimited,
        Ok(Outcome::RequeueAfter(delay)) => QueueOp::ForgetAndAddAfter(*delay),
        Ok(Outcome::Done) => QueueOp::Forget,
    }
}

#[cfg(test)]
mod tests {
    use crossplane_core::errors::Error;

    use super::*;

    #[test]
    fn queue_ops_match_controller_runtime() {
        let minute = Duration::from_secs(60);
        assert_eq!(queue_op(&Ok(Outcome::Done)), QueueOp::Forget);
        assert_eq!(queue_op(&Ok(Outcome::Requeue)), QueueOp::AddRateLimited);
        assert_eq!(
            queue_op(&Ok(Outcome::RequeueAfter(minute))),
            QueueOp::ForgetAndAddAfter(minute)
        );
        assert_eq!(queue_op(&Err(Error::new("boom"))), QueueOp::AddRateLimited);
    }
}
