//! client-go's rate-limiting, delaying work queue, for one controller's requests.
//!
//! The semantics kube's scheduler lacks and Go's controllers rely on:
//!
//! - An item added while it is being processed is not handed to a second worker. It is marked
//!   dirty and re-queued when the first worker calls `done`.
//! - `add_rate_limited` delays by the item's own backoff; `forget` resets it.
//! - `add_after` delays by a fixed duration and does not touch the backoff.
//!
//! One queue has many producers and many consumers. That one shared structure is what gives
//! "never two reconciles for one request", so it is one mutex around plain data.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

use crate::ratelimiter::RateLimiter;
use crate::reconciler::Request;

pub(super) struct WorkQueue {
    state: Mutex<State>,
    ready: Notify,
    rate_limiter: Arc<dyn RateLimiter>,
}

#[derive(Default)]
struct State {
    /// Items ready to hand out, in arrival order.
    queue: VecDeque<Request>,
    /// Items that need processing. Deduplicates `queue`.
    dirty: HashSet<Request>,
    /// Items a worker holds right now.
    processing: HashSet<Request>,
    /// Items waiting for a time, earliest first.
    waiting: BTreeMap<(Instant, u64), Request>,
    shutting_down: bool,
}

impl WorkQueue {
    pub(super) fn new(rate_limiter: Arc<dyn RateLimiter>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            ready: Notify::new(),
            rate_limiter,
        }
    }

    pub(super) fn add(&self, item: Request) {
        let _ = (&self.state, &self.ready);
        todo!("client-go Type.Add for {item:?}: no-op when dirty; queue unless processing")
    }

    pub(super) fn add_after(&self, item: Request, delay: Duration) {
        // TODO: OPEN (docs/design/core.md): client-go's delaying queue keeps the earlier time
        // when the item is already waiting. Confirm against delaying_queue.go before porting.
        todo!("add {item:?} after {delay:?}")
    }

    pub(super) fn add_rate_limited(&self, item: Request) {
        let delay = self.rate_limiter.when(&item);
        self.add_after(item, delay);
    }

    pub(super) fn forget(&self, item: &Request) {
        self.rate_limiter.forget(item);
    }

    /// The next item, or `None` once the queue is shut down and drained. The caller must call
    /// `done` with the item when finished.
    pub(super) async fn get(&self) -> Option<Request> {
        todo!("move due items from waiting to queue, then pop into processing or wait on `ready`")
    }

    pub(super) fn done(&self, item: &Request) {
        todo!("{item:?} leaves processing; re-queue when dirty")
    }

    pub(super) fn shut_down(&self) {
        todo!()
    }
}
