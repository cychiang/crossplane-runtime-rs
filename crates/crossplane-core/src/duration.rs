//! Go `time.ParseDuration`, so flags and the `crossplane.io/poll-interval` annotation parse the
//! same strings as the Go provider.

use std::time::Duration;

/// A negative value is an error here. The annotation reader treats any error as "ignore".
pub fn parse(s: &str) -> Result<Duration, String> {
    // TODO: port time.ParseDuration: sign, decimal fractions, units ns us µs ms s m h.
    todo!("parse Go duration {s:?}")
}
