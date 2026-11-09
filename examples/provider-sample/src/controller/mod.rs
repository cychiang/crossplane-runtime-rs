//! Go `internal/controller/register.go`.

pub mod config;
pub mod mytype;

use crossplane_core::errors::Result;
use crossplane_runtime::controller::Options;
use crossplane_runtime::manager::Manager;

/// The ProviderConfig controller is not gated. MyType waits for its CRD.
pub fn setup(mgr: &Manager, o: &Options) -> Result<()> {
    for setup in [config::setup, mytype::setup_gated] {
        setup(mgr, o)?;
    }
    Ok(())
}
