//! controller-runtime's priority queue (`pkg/controller/priorityqueue/priorityqueue.go`), the queue
//! controller-runtime v0.24.1 uses by default, for one controller's requests. Priorities are not
//! ported; every item has Go's default priority 0, so ready items leave in the order Go hands them
//! out at one priority.
//!
//! Per object it behaves as client-go's rate-limiting, delaying queue does, with one difference
//! that comes from holding one entry per key instead of a FIFO plus a separate delay heap:
//!
//! - An item a worker holds is never handed to a second worker. Added again meanwhile, it waits
//!   and is handed out after `done`.
//! - Adds of a queued item collapse. Of two delays the earlier ready time wins, and an add with
//!   no delay makes a waiting item ready at once and cancels its delay (`priorityqueue.go:239-243`).
//!   client-go's delaying queue keeps the delayed copy and adds it a second time when it fires.
//! - `add_rate_limited` delays by the item's own backoff; `forget` resets it.
//! - `shut_down` stops handing out items at once. Queued items are dropped, not drained
//!   (`priorityqueue.go:424-428,475-484`).
//!
//! One queue has many producers and many consumers, so it is one mutex around plain data.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;

use crate::ratelimiter::RateLimiter;
use crate::reconciler::Request;

pub(super) struct WorkQueue {
    state: Mutex<State>,
    changed: Notify,
    rate_limiter: Arc<dyn RateLimiter>,
}

/// Go's `item`. `added` orders ready items, and breaks ties among waiting items with one time.
#[derive(Clone, Copy)]
struct Entry {
    added: u64,
    ready_at: Option<Instant>,
}

#[derive(Default)]
struct State {
    /// Every queued item, each in exactly one of `ready` (no `ready_at`) and `waiting`.
    items: HashMap<Request, Entry>,
    ready: BTreeMap<u64, Request>,
    waiting: BTreeMap<(Instant, u64), Request>,
    /// Items a worker holds. Such an item may also be queued; it stays put until `done`.
    locked: HashSet<Request>,
    added_counter: u64,
    shutting_down: bool,
}

impl State {
    fn next_added(&mut self) -> u64 {
        self.added_counter += 1;
        self.added_counter
    }

    /// Go `lockedAddWithOpts` at priority 0.
    fn add(&mut self, item: Request, ready_at: Option<Instant>) {
        let Some(old) = self.items.get(&item).copied() else {
            let entry = Entry {
                added: self.next_added(),
                ready_at,
            };
            self.insert(item, entry);
            return;
        };
        let ready_at = match (old.ready_at, ready_at) {
            (Some(old), Some(new)) => Some(old.min(new)),
            _ => None,
        };
        self.remove(&item, old);
        self.insert(item, Entry { ready_at, ..old });
    }

    fn insert(&mut self, item: Request, entry: Entry) {
        match entry.ready_at {
            None => self.ready.insert(entry.added, item.clone()),
            Some(at) => self.waiting.insert((at, entry.added), item.clone()),
        };
        self.items.insert(item, entry);
    }

    fn remove(&mut self, item: &Request, entry: Entry) {
        match entry.ready_at {
            None => self.ready.remove(&entry.added),
            Some(at) => self.waiting.remove(&(at, entry.added)),
        };
        self.items.remove(item);
    }

    /// Go `handleWaitingItems`: a due item joins the back of the ready items.
    fn promote_due(&mut self, now: Instant) {
        while let Some(entry) = self.waiting.first_entry() {
            if entry.key().0 > now {
                break;
            }
            let item = entry.remove();
            let entry = Entry {
                added: self.next_added(),
                ready_at: None,
            };
            self.insert(item, entry);
        }
    }

    /// Go `handleReadyItems` for one waiter: the first ready item no worker holds.
    fn take_ready(&mut self) -> Option<Request> {
        let (&added, item) = self.ready.iter().find(|(_, item)| !self.locked.contains(*item))?;
        let item = item.clone();
        self.remove(&item, Entry { added, ready_at: None });
        self.locked.insert(item.clone());
        Some(item)
    }
}

impl WorkQueue {
    pub(super) fn new(rate_limiter: Arc<dyn RateLimiter>) -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: Notify::new(),
            rate_limiter,
        }
    }

    pub(super) fn add(&self, item: Request) {
        self.add_after(item, Duration::ZERO);
    }

    pub(super) fn add_after(&self, item: Request, delay: Duration) {
        self.add_with(item, |_| delay);
    }

    pub(super) fn add_rate_limited(&self, item: Request) {
        self.add_with(item, |item| self.rate_limiter.when(item));
    }

    /// The delay is computed only when the queue is open: Go does not ask the rate limiter, and so
    /// does not count a failure, for an add after shutdown.
    fn add_with(&self, item: Request, delay: impl FnOnce(&Request) -> Duration) {
        {
            let mut state = self.state.lock().unwrap();
            if state.shutting_down {
                return;
            }
            let delay = delay(&item);
            let ready_at = (delay > Duration::ZERO).then(|| ready_at(Instant::now(), delay));
            state.add(item, ready_at);
        }
        self.changed.notify_waiters();
    }

    pub(super) fn forget(&self, item: &Request) {
        self.rate_limiter.forget(item);
    }

    /// The next item, or `None` once the queue is shut down. The caller must call `done` with the
    /// item when finished.
    pub(super) async fn get(&self) -> Option<Request> {
        loop {
            let changed = self.changed.notified();
            let mut changed = std::pin::pin!(changed);
            changed.as_mut().enable();
            let next_ready_at = {
                let mut state = self.state.lock().unwrap();
                if state.shutting_down {
                    return None;
                }
                state.promote_due(Instant::now());
                if let Some(item) = state.take_ready() {
                    return Some(item);
                }
                state.waiting.keys().next().map(|(at, _)| *at)
            };
            match next_ready_at {
                Some(at) => {
                    tokio::select! {
                        () = changed => {}
                        () = tokio::time::sleep_until(at) => {}
                    }
                }
                None => changed.await,
            }
        }
    }

    pub(super) fn done(&self, item: &Request) {
        self.state.lock().unwrap().locked.remove(item);
        self.changed.notify_waiters();
    }

    pub(super) fn shut_down(&self) {
        self.state.lock().unwrap().shutting_down = true;
        self.changed.notify_waiters();
    }

    /// Go `Len`: ready items, including ones a worker holds, but not waiting ones.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.state.lock().unwrap().ready.len()
    }
}

/// Go's limiter returns `math.MaxInt64` nanoseconds for a reservation it can never grant, which
/// overflows an `Instant`. Such an item waits for good, which is what Go's does.
fn ready_at(now: Instant, delay: Duration) -> Instant {
    now.checked_add(delay)
        .unwrap_or_else(|| now + Duration::from_secs(100 * 365 * 24 * 3600))
}

#[cfg(test)]
#[path = "workqueue_test.rs"]
mod tests;
