//! Go `internal/provider/options.go`. THIS FILE IS YOURS.

use std::sync::OnceLock;

use crossplane_core::errors::Result;
use crossplane_runtime::controller;

/// Provider-specific flags, flattened into `main`'s `Args`.
#[derive(clap::Args, Clone, Debug)]
pub struct Flags {
    /// Base URL of the external service.
    #[arg(
        long,
        env = "EXTERNAL_ENDPOINT",
        default_value = "http://fake-external.parity-external.svc.cluster.local:8080"
    )]
    pub external_endpoint: String,
}

/// Go keeps the endpoint in a package variable that `Flags` fills in. Set once by `configure`.
static ENDPOINT: OnceLock<String> = OnceLock::new();

pub(super) fn endpoint() -> String {
    ENDPOINT.get().cloned().unwrap_or_default()
}

/// Adjusts controller options after the standard flags are applied. An error aborts startup.
pub fn configure(_o: &mut controller::Options, flags: &Flags) -> Result<()> {
    ENDPOINT.get_or_init(|| flags.external_endpoint.clone());
    Ok(())
}
