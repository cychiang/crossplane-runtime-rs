//! `pkg/errors`, plus the Kubernetes status reason Go reads with `kerrors.IsNotFound` and friends.
//!
//! Condition and event messages are built from this type's display, so `"<message>: <cause>"` is
//! part of the parity contract.

use std::fmt;
use std::sync::Arc;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Only the reasons in-scope code branches on are named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusReason {
    NotFound,
    AlreadyExists,
    Conflict,
    Forbidden,
    Invalid,
    Other(String),
}

/// `Clone` because Go errors are values: a table case hands the same `boom` to a mock and to the
/// expected result.
#[derive(Clone)]
pub struct Error(Arc<Repr>);

enum Repr {
    Message(String),
    Wrapped {
        message: String,
        cause: Error,
    },
    /// The only variant that stores a [`StatusReason`], so wrapping cannot lose it.
    Status {
        reason: StatusReason,
        message: String,
    },
    DeadlineExceeded,
    Foreign(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Go `errors.New`.
    pub fn new(message: impl Into<String>) -> Self {
        Self(Arc::new(Repr::Message(message.into())))
    }

    /// Go `errors.Wrap(self, message)`.
    pub fn wrap(self, message: impl Into<String>) -> Self {
        Self(Arc::new(Repr::Wrapped {
            message: message.into(),
            cause: self,
        }))
    }

    /// An API server refusal. Built by `Client` implementations and by tests.
    pub fn status(reason: StatusReason, message: impl Into<String>) -> Self {
        Self(Arc::new(Repr::Status {
            reason,
            message: message.into(),
        }))
    }

    pub fn not_found() -> Self {
        Self::status(StatusReason::NotFound, "not found")
    }

    pub fn conflict() -> Self {
        Self::status(
            StatusReason::Conflict,
            "the object has been modified; please apply your changes to the latest version and try again",
        )
    }

    /// Displays Go's `context deadline exceeded`.
    pub fn deadline_exceeded() -> Self {
        Self(Arc::new(Repr::DeadlineExceeded))
    }

    /// An error from outside the runtime, such as an external API SDK.
    pub fn foreign(err: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Arc::new(Repr::Foreign(Box::new(err))))
    }

    /// The status reason anywhere in the wrap chain: Go's `errors.As` walk.
    pub fn status_reason(&self) -> Option<&StatusReason> {
        match &*self.0 {
            Repr::Wrapped { cause, .. } => cause.status_reason(),
            Repr::Status { reason, .. } => Some(reason),
            Repr::Message(_) | Repr::DeadlineExceeded | Repr::Foreign(_) => None,
        }
    }

    pub fn is_not_found(&self) -> bool {
        self.status_reason() == Some(&StatusReason::NotFound)
    }

    pub fn is_conflict(&self) -> bool {
        self.status_reason() == Some(&StatusReason::Conflict)
    }

    pub fn is_already_exists(&self) -> bool {
        self.status_reason() == Some(&StatusReason::AlreadyExists)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.0 {
            Repr::Message(message) | Repr::Status { message, .. } => f.write_str(message),
            Repr::Wrapped { message, cause } => write!(f, "{message}: {cause}"),
            Repr::DeadlineExceeded => f.write_str("context deadline exceeded"),
            Repr::Foreign(err) => fmt::Display::fmt(err, f),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error({:?}, reason={:?})", self.to_string(), self.status_reason())
    }
}

impl std::error::Error for Error {}

/// Go `test.EquateErrors`, extended with the status reason: equal display text and equal reason.
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string() && self.status_reason() == other.status_reason()
    }
}

/// `Ok` passes through, as `errors.Wrap(nil, ..)` returns nil.
pub trait ResultExt<T> {
    fn wrap_err(self, message: &str) -> Result<T>;
}

impl<T> ResultExt<T> for Result<T> {
    fn wrap_err(self, message: &str) -> Result<T> {
        self.map_err(|err| err.wrap(message))
    }
}

/// Go `resource.IgnoreNotFound`.
pub fn ignore_not_found(result: Result<()>) -> Result<()> {
    match result {
        Err(err) if err.is_not_found() => Ok(()),
        other => other,
    }
}
