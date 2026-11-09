//! Go `internal/provider`.

mod client;
mod connector;
mod options;

pub use client::{Client, ClientConfig, Thing};
pub use connector::Connector;
pub use options::{Flags, configure};
