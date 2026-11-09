//! `pkg/resource/interfaces.go`: what the runtime needs to know about an object.
//!
//! Go discovers these capabilities by type assertion at run time. Here each is a trait, so a
//! kind that lacks one fails to compile at the controller setup that needs it. Providers never
//! implement them by hand: the derives at the crate root do.

mod kinds;
mod object;

pub use kinds::{Managed, ProviderConfig, ProviderConfigUsage, mark_conditions};
pub use object::{Kind, Object, ObjectKey, ObjectType};
