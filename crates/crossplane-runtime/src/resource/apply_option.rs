use crossplane_core::errors::{Error, ErrorKind, Result};
use crossplane_core::resource::Object;
use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

use super::connection::CONNECTION_SECRET_TYPE;

type UpdateIf = Box<dyn Fn(&serde_json::Value, &serde_json::Value) -> bool + Send + Sync>;
type Custom = Box<dyn Fn(&mut dyn Object, &dyn Object) -> Result<()> + Send + Sync>;

/// Go `resource.ApplyOption`: called with the current and the desired object before an applicator
/// writes. Not called when no current object exists.
pub enum ApplyOption {
    /// Go `MustBeControllableBy(uid)`.
    MustBeControllableBy(String),
    /// Go `ConnectionSecretMustBeControllableBy(uid)`.
    ConnectionSecretMustBeControllableBy(String),
    /// Go `AllowUpdateIf(fn)`, over the JSON forms of current and desired.
    AllowUpdateIf(UpdateIf),
    /// Go's bare `ApplyOption` function, which may also change the current object.
    Custom(Custom),
}

impl ApplyOption {
    /// Go `UpdateFn`.
    pub fn update_fn(f: impl Fn(&mut dyn Object, &dyn Object) + Send + Sync + 'static) -> Self {
        Self::Custom(Box::new(move |current, desired| {
            f(current, desired);
            Ok(())
        }))
    }

    pub fn run(&self, current: &mut dyn Object, desired: &dyn Object) -> Result<()> {
        match self {
            Self::MustBeControllableBy(uid) => match controller_uid(current.meta()) {
                Some(c) if c != uid => Err(Error::with_kind(
                    ErrorKind::NotControllable,
                    format!("existing object is not controlled by UID {uid:?}"),
                )),
                _ => Ok(()),
            },
            Self::ConnectionSecretMustBeControllableBy(uid) => {
                let secret = current
                    .downcast_ref::<Secret>()
                    .ok_or_else(|| Error::new("current resource is not a Secret"))?;
                let secret_type = secret.type_.as_deref().unwrap_or("");
                match controller_uid(&secret.metadata) {
                    None if secret_type != CONNECTION_SECRET_TYPE => Err(Error::with_kind(
                        ErrorKind::NotControllable,
                        format!("refusing to modify uncontrolled secret of type {secret_type:?}"),
                    )),
                    Some(c) if c != uid => Err(Error::with_kind(
                        ErrorKind::NotControllable,
                        format!("existing secret is not controlled by UID {uid:?}"),
                    )),
                    _ => Ok(()),
                }
            }
            Self::AllowUpdateIf(f) => {
                if f(&current.to_json()?, &desired.to_json()?) {
                    Ok(())
                } else {
                    Err(new_not_allowed("update not allowed"))
                }
            }
            Self::Custom(f) => f(current, desired),
        }
    }
}

fn controller_uid(meta: &ObjectMeta) -> Option<&str> {
    meta.owner_references
        .as_ref()?
        .iter()
        .find(|r| r.controller == Some(true))
        .map(|r| r.uid.as_str())
}

/// Go `resource.IsNotControllable`.
pub fn is_not_controllable(err: &Error) -> bool {
    err.kind() == Some(ErrorKind::NotControllable)
}

/// Go `resource.NewNotAllowed`.
pub fn new_not_allowed(message: &str) -> Error {
    Error::with_kind(ErrorKind::NotAllowed, message)
}

/// Go `resource.IsNotAllowed`.
pub fn is_not_allowed(err: &Error) -> bool {
    err.kind() == Some(ErrorKind::NotAllowed)
}

/// Go `resource.IsMissingReference`.
pub fn is_missing_reference(err: &Error) -> bool {
    err.kind() == Some(ErrorKind::MissingReference)
}

#[cfg(test)]
#[path = "apply_option_test.rs"]
mod tests;
