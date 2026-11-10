use std::collections::BTreeMap;

use crossplane_core::errors::Error;
use crossplane_core::resource::{Managed, ObjectType};
use crossplane_core::v2::{Condition, ConditionStatus};

pub const EXTERNAL_RESOURCE_TAG_KEY_KIND: &str = "crossplane-kind";
pub const EXTERNAL_RESOURCE_TAG_KEY_NAME: &str = "crossplane-name";
pub const EXTERNAL_RESOURCE_TAG_KEY_NAMESPACE: &str = "crossplane-namespace";
pub const EXTERNAL_RESOURCE_TAG_KEY_PROVIDER: &str = "crossplane-providerconfig";
pub const EXTERNAL_RESOURCE_TAG_KEY_PROVIDER_CONFIG_KIND: &str = "crossplane-providerconfig-kind";

/// Go `resource.Ignore`: an error that satisfies `is` becomes success.
pub fn ignore(
    is: impl Fn(&Error) -> bool,
    result: crossplane_core::errors::Result<()>,
) -> crossplane_core::errors::Result<()> {
    match result {
        Err(err) if is(&err) => Ok(()),
        other => other,
    }
}

/// Go `resource.IgnoreAny`.
pub fn ignore_any(
    result: crossplane_core::errors::Result<()>,
    is: &[&dyn Fn(&Error) -> bool],
) -> crossplane_core::errors::Result<()> {
    match result {
        Err(err) if is.iter().any(|f| f(&err)) => Ok(()),
        other => other,
    }
}

/// Go `resource.IsAPIErrorWrapped`: the error, or the cause under its wraps, is an API server
/// refusal.
pub fn is_api_error_wrapped(err: &Error) -> bool {
    err.cause().status_reason().is_some()
}

/// Go `resource.IsConditionTrue`.
pub fn is_condition_true(c: &Condition) -> bool {
    c.status == ConditionStatus::True
}

/// Go `resource.GetExternalTags`: the tags that identify the external resource in a provider API.
pub fn get_external_tags<M: Managed>(mg: &M) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::from([
        (
            EXTERNAL_RESOURCE_TAG_KEY_KIND.to_owned(),
            ObjectType::of::<M>().group_kind().to_lowercase(),
        ),
        (
            EXTERNAL_RESOURCE_TAG_KEY_NAME.to_owned(),
            mg.meta().name.clone().unwrap_or_default(),
        ),
    ]);
    if let Some(ns) = mg.meta().namespace.as_deref().filter(|ns| !ns.is_empty()) {
        tags.insert(EXTERNAL_RESOURCE_TAG_KEY_NAMESPACE.to_owned(), ns.to_owned());
    }
    if let Some(r) = mg.provider_config_reference() {
        if !r.name.is_empty() {
            tags.insert(EXTERNAL_RESOURCE_TAG_KEY_PROVIDER.to_owned(), r.name.clone());
        }
        if !r.kind.is_empty() {
            tags.insert(
                EXTERNAL_RESOURCE_TAG_KEY_PROVIDER_CONFIG_KIND.to_owned(),
                r.kind.clone(),
            );
        }
    }
    tags
}

/// Go `resource.DefaultFirstN`.
pub const DEFAULT_FIRST_N: i32 = 3;

/// Go `resource.FirstNAndSomeMore`.
pub fn first_n_and_some_more(n: i32, names: &[String]) -> String {
    if n <= 0 {
        return names.len().to_string();
    }
    let n = n as usize;
    if names.len() > n {
        format!("{}, and {} more", names[..n].join(", "), names.len() - n)
    } else if names.len() == n {
        format!("{}, and {}", names[..n - 1].join(", "), names[n - 1])
    } else {
        names.join(", ")
    }
}

/// Go `resource.StableNAndSomeMore`: sorts a copy first.
pub fn stable_n_and_some_more(n: i32, names: &[String]) -> String {
    let mut sorted = names.to_vec();
    sorted.sort();
    first_n_and_some_more(n, &sorted)
}

#[cfg(test)]
#[path = "helpers_test.rs"]
mod tests;
