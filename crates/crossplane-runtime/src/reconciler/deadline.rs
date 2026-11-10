//! A Go context's deadline over one call, shared by the reconcilers that port a
//! `context.WithTimeout`.

use std::future::Future;

use crossplane_core::errors::{Error, Result};
use tokio::time::Instant;

/// A call under a Go context with a deadline: it fails with `context deadline exceeded` when the
/// deadline has passed or passes while it runs, and the reconcile carries on to the branch that
/// handles the error.
pub(crate) async fn bounded<T>(deadline: Instant, call: impl Future<Output = Result<T>>) -> Result<T> {
    if Instant::now() >= deadline {
        return Err(Error::deadline_exceeded());
    }
    match tokio::time::timeout_at(deadline, call).await {
        Ok(result) => result,
        Err(_) => Err(Error::deadline_exceeded()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::FutureExt;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_call_after_the_deadline_is_never_started() {
        let deadline = Instant::now();
        tokio::time::advance(Duration::from_millis(1)).await;
        let call = async { panic!("the call ran after its deadline") };
        assert_eq!(bounded::<()>(deadline, call).await, Err(Error::deadline_exceeded()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_still_running_at_the_deadline_fails_then() {
        let start = Instant::now();
        let got = bounded::<()>(start + Duration::from_secs(5), futures::future::pending().boxed()).await;
        assert_eq!(got, Err(Error::deadline_exceeded()));
        assert_eq!(Instant::now() - start, Duration::from_secs(5));
    }
}
