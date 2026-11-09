//! An injectable clock. Go reads `time.Now()` inline; injecting it makes the annotation-timestamp
//! and creation-grace cases deterministic (parity spec 6.3 item 8). A change in test mechanics,
//! not in behaviour.

use k8s_openapi::jiff::Timestamp;

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Timestamp;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}
