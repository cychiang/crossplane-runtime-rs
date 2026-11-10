use std::fmt;

/// Go `resource.ReferenceStatusType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferenceStatusType {
    Unknown,
    NotFound,
    NotReady,
    Ready,
}

impl fmt::Display for ReferenceStatusType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unknown => "Unknown",
            Self::NotFound => "NotFound",
            Self::NotReady => "NotReady",
            Self::Ready => "Ready",
        })
    }
}

/// Go `resource.ReferenceStatus`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceStatus {
    pub name: String,
    pub status: ReferenceStatusType,
}

impl fmt::Display for ReferenceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{reference:{} status:{}}}", self.name, self.status)
    }
}
