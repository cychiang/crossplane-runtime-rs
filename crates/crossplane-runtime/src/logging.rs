//! Go threads a `logging.Logger` through options. Here the global `tracing` subscriber does the
//! job and each controller attaches its name as a span field, so no option carries a logger.

use tracing_subscriber::EnvFilter;

/// `--debug` selects the pretty format at `debug`; otherwise JSON at `info`. `RUST_LOG` overrides
/// the level. Panics when a global subscriber is already set.
pub fn init(debug: bool) {
    let default_level = if debug { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if debug {
        builder.pretty().init();
    } else {
        builder.json().init();
    }
}
