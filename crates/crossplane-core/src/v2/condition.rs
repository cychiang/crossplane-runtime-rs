use std::borrow::Cow;
use std::fmt;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Open set: providers define their own, so a string newtype with the core types as constants.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ConditionType(pub Cow<'static, str>);

impl ConditionType {
    pub const READY: Self = Self(Cow::Borrowed("Ready"));
    pub const SYNCED: Self = Self(Cow::Borrowed("Synced"));
    pub const HEALTHY: Self = Self(Cow::Borrowed("Healthy"));
    pub const UP_TO_DATE: Self = Self(Cow::Borrowed("UpToDate"));
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ConditionReason(pub Cow<'static, str>);

impl ConditionReason {
    pub const AVAILABLE: Self = Self(Cow::Borrowed("Available"));
    pub const UNAVAILABLE: Self = Self(Cow::Borrowed("Unavailable"));
    pub const CREATING: Self = Self(Cow::Borrowed("Creating"));
    pub const DELETING: Self = Self(Cow::Borrowed("Deleting"));
    pub const RECONCILE_SUCCESS: Self = Self(Cow::Borrowed("ReconcileSuccess"));
    pub const RECONCILE_ERROR: Self = Self(Cow::Borrowed("ReconcileError"));
    pub const RECONCILE_PAUSED: Self = Self(Cow::Borrowed("ReconcilePaused"));
}

/// `corev1.ConditionStatus`. A plain string in the Go schema, so [`Condition::status`] overrides
/// its schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum ConditionStatus {
    True,
    False,
    Unknown,
}

/// `xpv2.Condition`. `message` and `observedGeneration` are Go `omitempty` value fields: plain
/// values skipped when empty, not `Option`s, so the schema has no `nullable`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    #[serde(rename = "type")]
    pub type_: ConditionType,
    #[schemars(with = "String")]
    pub status: ConditionStatus,
    pub last_transition_time: Time,
    pub reason: ConditionReason,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub observed_generation: i64,
}

fn is_zero(v: &i64) -> bool {
    *v == 0
}

impl Condition {
    fn new(type_: ConditionType, status: ConditionStatus, reason: ConditionReason) -> Self {
        Self {
            type_,
            status,
            last_transition_time: Time(Timestamp::now()),
            reason,
            message: String::new(),
            observed_generation: 0,
        }
    }

    /// Go `Condition.Equal`: every field except `lastTransitionTime`. Not `PartialEq`, so `==`
    /// keeps its usual meaning.
    pub fn equal(&self, other: &Condition) -> bool {
        self.type_ == other.type_
            && self.status == other.status
            && self.reason == other.reason
            && self.message == other.message
            && self.observed_generation == other.observed_generation
    }

    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.message = msg.into();
        self
    }

    pub fn with_observed_generation(mut self, g: i64) -> Self {
        self.observed_generation = g;
        self
    }

    pub fn available() -> Self {
        Self::new(ConditionType::READY, ConditionStatus::True, ConditionReason::AVAILABLE)
    }
    pub fn unavailable() -> Self {
        Self::new(
            ConditionType::READY,
            ConditionStatus::False,
            ConditionReason::UNAVAILABLE,
        )
    }
    pub fn creating() -> Self {
        Self::new(ConditionType::READY, ConditionStatus::False, ConditionReason::CREATING)
    }
    pub fn deleting() -> Self {
        Self::new(ConditionType::READY, ConditionStatus::False, ConditionReason::DELETING)
    }
    pub fn reconcile_success() -> Self {
        Self::new(
            ConditionType::SYNCED,
            ConditionStatus::True,
            ConditionReason::RECONCILE_SUCCESS,
        )
    }
    /// The message is `err.Error()`.
    pub fn reconcile_error(err: &impl fmt::Display) -> Self {
        Self::new(
            ConditionType::SYNCED,
            ConditionStatus::False,
            ConditionReason::RECONCILE_ERROR,
        )
        .with_message(err.to_string())
    }
    pub fn reconcile_paused() -> Self {
        Self::new(
            ConditionType::SYNCED,
            ConditionStatus::False,
            ConditionReason::RECONCILE_PAUSED,
        )
    }
}

/// `xpv2.ConditionedStatus`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ConditionedStatus {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("x-kubernetes-list-type" = "map", "x-kubernetes-list-map-keys" = ["type"]))]
    pub conditions: Vec<Condition>,
}

impl ConditionedStatus {
    pub fn get_condition(&self, ct: &ConditionType) -> Condition {
        // TODO: the condition of type `ct`; when absent, an Unknown condition of that type with
        // every other field zero (condition.go GetCondition).
        todo!("get condition {ct:?} of {} conditions", self.conditions.len())
    }

    /// Replaces by type. An existing entry that is `equal` to the new one is kept, so its
    /// `lastTransitionTime` does not move.
    pub fn set_conditions(&mut self, cs: impl IntoIterator<Item = Condition>) {
        for new in cs {
            match self.conditions.iter_mut().find(|c| c.type_ == new.type_) {
                Some(existing) if existing.equal(&new) => {}
                Some(existing) => *existing = new,
                None => self.conditions.push(new),
            }
        }
    }
}
