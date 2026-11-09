//! Go `internal/controller/mytype`.

mod external;
mod wiring;

pub use external::{External, reconciler_options};
pub use wiring::{setup, setup_gated};
