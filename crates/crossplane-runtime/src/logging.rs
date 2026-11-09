//! Go threads a `logging.Logger` through options. Here the global `tracing` subscriber does the
//! job and each controller attaches its name as a span field, so no option carries a logger.

/// `--debug` selects the pretty format at `debug`; otherwise JSON at `info`.
pub fn init(debug: bool) {
    todo!("tracing_subscriber::fmt() with env filter, debug = {debug}")
}
