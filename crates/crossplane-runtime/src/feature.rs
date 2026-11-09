//! `pkg/feature`.

use std::collections::BTreeSet;
use std::fmt;

/// Open set: providers add their own flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Flag(pub &'static str);

impl fmt::Display for Flag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

pub const ENABLE_BETA_MANAGEMENT_POLICIES: Flag = Flag("EnableBetaManagementPolicies");
pub const ENABLE_ALPHA_CHANGE_LOGS: Flag = Flag("EnableAlphaChangeLogs");

/// Go `feature.Flags`. Set once in `main` before controllers start, then cloned into each setup
/// function; never mutated concurrently.
#[derive(Clone, Debug, Default)]
pub struct Features(BTreeSet<Flag>);

impl Features {
    pub fn enable(&mut self, flag: Flag) {
        self.0.insert(flag);
    }

    pub fn enabled(&self, flag: Flag) -> bool {
        self.0.contains(&flag)
    }
}
