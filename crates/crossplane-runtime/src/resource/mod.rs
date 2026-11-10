//! `pkg/resource` minus the interfaces, which are in `crossplane_core::resource`.

mod applicator;
mod apply_option;
mod connection;
mod credentials;
mod enqueue;
mod finalizer;
mod helpers;
mod late_init;
mod predicates;
mod reference;
mod tracker;

pub use self::applicator::{
    ApiPatchingApplicator, ApiUpdatingApplicator, Applicator, ApplicatorWithRetry, ApplyFn, Backoff, DEFAULT_RETRY,
};
pub use self::apply_option::{ApplyOption, is_missing_reference, is_not_allowed, is_not_controllable, new_not_allowed};
pub use self::connection::{CONNECTION_SECRET_TYPE, local_connection_secret_for};
pub use self::credentials::{common_credential_extractor, extract_env, extract_fs, extract_secret};
pub use self::enqueue::EnqueueRequestForProviderConfig;
pub use self::finalizer::{ApiFinalizer, Finalizer, FinalizerFns};
pub use self::helpers::{
    DEFAULT_FIRST_N, EXTERNAL_RESOURCE_TAG_KEY_KIND, EXTERNAL_RESOURCE_TAG_KEY_NAME,
    EXTERNAL_RESOURCE_TAG_KEY_NAMESPACE, EXTERNAL_RESOURCE_TAG_KEY_PROVIDER,
    EXTERNAL_RESOURCE_TAG_KEY_PROVIDER_CONFIG_KIND, first_n_and_some_more, get_external_tags, ignore, ignore_any,
    is_api_error_wrapped, is_condition_true, stable_n_and_some_more,
};
pub use self::late_init::LateInitializer;
pub use self::predicates::{DesiredStateChanged, desired_state_changed};
pub use self::reference::{ReferenceStatus, ReferenceStatusType};
pub use self::tracker::{ERR_APPLY_PCU, ERR_MISSING_PC_KIND, ERR_MISSING_PC_REF, ProviderConfigUsageTracker};
