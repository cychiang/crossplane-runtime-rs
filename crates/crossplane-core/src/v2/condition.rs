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
    pub const UPDATE_RESTRICTED: Self = Self(Cow::Borrowed("UpdateRestricted"));
    pub const OBSERVE_MATCHED: Self = Self(Cow::Borrowed("ObserveMatched"));
    pub const UPDATE_FAILED: Self = Self(Cow::Borrowed("UpdateFailed"));
    pub const UPDATE_REQUESTED: Self = Self(Cow::Borrowed("UpdateRequested"));
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

/// Go's zero `metav1.Time`, 0001-01-01T00:00:00Z.
fn zero_time() -> Time {
    Time("0001-01-01T00:00:00Z".parse().expect("a valid timestamp"))
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

    /// Go `Condition.Equal`: every field except `lastTransitionTime`, and `observedGeneration`
    /// only when both sides set it. Not `PartialEq`, so `==` keeps its usual meaning.
    pub fn equal(&self, other: &Condition) -> bool {
        let same_generation = self.observed_generation == 0
            || other.observed_generation == 0
            || self.observed_generation == other.observed_generation;
        self.type_ == other.type_
            && self.status == other.status
            && self.reason == other.reason
            && self.message == other.message
            && same_generation
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
    pub fn observe_matched() -> Self {
        Self::new(
            ConditionType::UP_TO_DATE,
            ConditionStatus::True,
            ConditionReason::OBSERVE_MATCHED,
        )
    }
    pub fn update_requested() -> Self {
        Self::new(
            ConditionType::UP_TO_DATE,
            ConditionStatus::False,
            ConditionReason::UPDATE_REQUESTED,
        )
    }
    pub fn update_restricted() -> Self {
        Self::new(
            ConditionType::UP_TO_DATE,
            ConditionStatus::False,
            ConditionReason::UPDATE_RESTRICTED,
        )
    }
    pub fn update_failed() -> Self {
        Self::new(
            ConditionType::UP_TO_DATE,
            ConditionStatus::False,
            ConditionReason::UPDATE_FAILED,
        )
    }
    pub fn paused_unknown() -> Self {
        Self::new(
            ConditionType::UP_TO_DATE,
            ConditionStatus::Unknown,
            ConditionReason::RECONCILE_PAUSED,
        )
    }
}

/// Go `IsSystemConditionType`: the types Crossplane owns.
pub fn is_system_condition_type(t: &ConditionType) -> bool {
    [
        ConditionType::READY,
        ConditionType::SYNCED,
        ConditionType::HEALTHY,
        ConditionType::UP_TO_DATE,
    ]
    .contains(t)
}

/// `xpv2.ConditionedStatus`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ConditionedStatus {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("x-kubernetes-list-type" = "map", "x-kubernetes-list-map-keys" = ["type"]))]
    pub conditions: Vec<Condition>,
}

impl ConditionedStatus {
    /// Go `NewConditionedStatus`.
    pub fn new(cs: impl IntoIterator<Item = Condition>) -> Self {
        let mut s = Self::default();
        s.set_conditions(cs);
        s
    }

    /// The condition of type `ct`; when absent, an Unknown condition of that type with every
    /// other field zero.
    pub fn get_condition(&self, ct: &ConditionType) -> Condition {
        self.conditions
            .iter()
            .find(|c| &c.type_ == ct)
            .cloned()
            .unwrap_or_else(|| Condition {
                type_: ct.clone(),
                status: ConditionStatus::Unknown,
                last_transition_time: zero_time(),
                reason: ConditionReason(Cow::Borrowed("")),
                message: String::new(),
                observed_generation: 0,
            })
    }

    /// Go `ConditionedStatus.Equal`, ignoring `lastTransitionTime` and order. A nil status is
    /// not representable: every status is a value on its kind.
    pub fn equal(&self, other: &ConditionedStatus) -> bool {
        if self.conditions.len() != other.conditions.len() {
            return false;
        }
        let mut sc: Vec<&Condition> = self.conditions.iter().collect();
        let mut oc: Vec<&Condition> = other.conditions.iter().collect();
        sc.sort_by(|a, b| a.type_.cmp(&b.type_));
        oc.sort_by(|a, b| a.type_.cmp(&b.type_));
        sc.iter().zip(oc).all(|(a, b)| a.equal(b))
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

#[cfg(test)]
mod tests {
    use super::*;

    const UNSET: ConditionReason = ConditionReason(Cow::Borrowed(""));

    /// A Go `Condition{...}` literal: the fields a case leaves out are zero. Go's empty status
    /// has no `ConditionStatus` value, so cases that do not set one use `Unknown` on both sides.
    fn cond(
        type_: ConditionType,
        status: ConditionStatus,
        reason: ConditionReason,
        message: &str,
        generation: i64,
    ) -> Condition {
        Condition {
            type_,
            status,
            last_transition_time: Time(Timestamp::now()),
            reason,
            message: message.to_owned(),
            observed_generation: generation,
        }
    }

    fn of_type(type_: ConditionType) -> Condition {
        cond(type_, ConditionStatus::Unknown, UNSET, "", 0)
    }

    fn of_status(status: ConditionStatus) -> Condition {
        cond(ConditionType(Cow::Borrowed("")), status, UNSET, "", 0)
    }

    fn of_reason(reason: ConditionReason) -> Condition {
        cond(
            ConditionType(Cow::Borrowed("")),
            ConditionStatus::Unknown,
            reason,
            "",
            0,
        )
    }

    fn ready_creating(message: &str, generation: i64) -> Condition {
        cond(
            ConditionType::READY,
            ConditionStatus::True,
            ConditionReason::CREATING,
            message,
            generation,
        )
    }

    #[test]
    fn condition_equal() {
        struct Case {
            name: &'static str,
            a: Condition,
            b: Condition,
            want: bool,
        }
        let cases = vec![
            // go: core/v2 TestConditionEqual/Identical
            Case {
                name: "Identical",
                a: ready_creating("UnitTest", 1),
                b: ready_creating("UnitTest", 1),
                want: true,
            },
            // go: core/v2 TestConditionEqual/IdenticalIgnoringTimestamp
            Case {
                name: "IdenticalIgnoringTimestamp",
                a: of_type(ConditionType::READY),
                b: of_type(ConditionType::READY),
                want: true,
            },
            // go: core/v2 TestConditionEqual/DifferentType
            Case {
                name: "DifferentType",
                a: of_type(ConditionType::READY),
                b: of_type(ConditionType::SYNCED),
                want: false,
            },
            // go: core/v2 TestConditionEqual/DifferentStatus
            Case {
                name: "DifferentStatus",
                a: of_status(ConditionStatus::True),
                b: of_status(ConditionStatus::False),
                want: false,
            },
            // go: core/v2 TestConditionEqual/DifferentReason
            Case {
                name: "DifferentReason",
                a: of_reason(ConditionReason::CREATING),
                b: of_reason(ConditionReason::DELETING),
                want: false,
            },
            // go: core/v2 TestConditionEqual/DifferentMessage
            Case {
                name: "DifferentMessage",
                a: of_reason(UNSET).with_message("cool"),
                b: of_reason(UNSET).with_message("uncool"),
                want: false,
            },
            // go: core/v2 TestConditionEqual/IdenticalWithMissingObservedGeneration
            Case {
                name: "IdenticalWithMissingObservedGeneration",
                a: ready_creating("UnitTest", 1),
                b: ready_creating("UnitTest", 0),
                want: true,
            },
            // go: core/v2 TestConditionEqual/DifferentObservedGeneration
            Case {
                name: "DifferentObservedGeneration",
                a: of_reason(UNSET).with_observed_generation(1),
                b: of_reason(UNSET).with_observed_generation(2),
                want: false,
            },
            // go: core/v2 TestConditionEqual/CheckReconcilePaused
            Case {
                name: "CheckReconcilePaused",
                a: Condition::reconcile_paused(),
                b: cond(
                    ConditionType::SYNCED,
                    ConditionStatus::False,
                    ConditionReason::RECONCILE_PAUSED,
                    "",
                    0,
                ),
                want: true,
            },
        ];
        for tc in cases {
            assert_eq!(tc.want, tc.a.equal(&tc.b), "{}: a.equal(b)", tc.name);
        }
    }

    #[test]
    fn conditioned_status_equal() {
        struct Case {
            name: &'static str,
            a: ConditionedStatus,
            b: ConditionedStatus,
            want: bool,
        }
        let cases = vec![
            // go: core/v2 TestConditionedStatusEqual/Identical
            Case {
                name: "Identical",
                a: ConditionedStatus::new([Condition::available(), Condition::reconcile_success()]),
                b: ConditionedStatus::new([Condition::available(), Condition::reconcile_success()]),
                want: true,
            },
            // go: core/v2 TestConditionedStatusEqual/IdenticalExceptOrder
            Case {
                name: "IdenticalExceptOrder",
                a: ConditionedStatus::new([Condition::unavailable(), Condition::reconcile_success()]),
                b: ConditionedStatus::new([Condition::reconcile_success(), Condition::unavailable()]),
                want: true,
            },
            // go: core/v2 TestConditionedStatusEqual/DifferentLength
            Case {
                name: "DifferentLength",
                a: ConditionedStatus::new([Condition::available(), Condition::reconcile_success()]),
                b: ConditionedStatus::new([Condition::reconcile_success()]),
                want: false,
            },
            // go: core/v2 TestConditionedStatusEqual/DifferentCondition
            Case {
                name: "DifferentCondition",
                a: ConditionedStatus::new([Condition::creating(), Condition::reconcile_success()]),
                b: ConditionedStatus::new([
                    Condition::creating(),
                    Condition::reconcile_error(&crate::errors::Error::new("boom")),
                ]),
                want: false,
            },
        ];
        for tc in cases {
            assert_eq!(tc.want, tc.a.equal(&tc.b), "{}: a.equal(b)", tc.name);
        }
    }

    #[test]
    fn set_conditions() {
        struct Case {
            name: &'static str,
            cs: ConditionedStatus,
            c: Vec<Condition>,
            want: ConditionedStatus,
        }
        let cases = vec![
            // go: core/v2 TestSetConditions/TypeIsIdentical
            Case {
                name: "TypeIsIdentical",
                cs: ConditionedStatus::new([Condition::available()]),
                c: vec![Condition::available()],
                want: ConditionedStatus::new([Condition::available()]),
            },
            // go: core/v2 TestSetConditions/ObservedGenerationIsUpdated
            Case {
                name: "ObservedGenerationIsUpdated",
                cs: ConditionedStatus::new([Condition::available().with_observed_generation(1)]),
                c: vec![Condition::available().with_observed_generation(2)],
                want: ConditionedStatus::new([Condition::available().with_observed_generation(2)]),
            },
            // go: core/v2 TestSetConditions/TypeIsDifferent
            Case {
                name: "TypeIsDifferent",
                cs: ConditionedStatus::new([Condition::creating()]),
                c: vec![Condition::available()],
                want: ConditionedStatus::new([Condition::available()]),
            },
            // go: core/v2 TestSetConditions/TypeDoesNotExist
            Case {
                name: "TypeDoesNotExist",
                cs: ConditionedStatus::new([Condition::reconcile_success()]),
                c: vec![Condition::available()],
                want: ConditionedStatus::new([Condition::reconcile_success(), Condition::available()]),
            },
        ];
        for mut tc in cases {
            tc.cs.set_conditions(tc.c);
            assert!(
                tc.want.equal(&tc.cs),
                "{}: set_conditions(...): want {:?}, got {:?}",
                tc.name,
                tc.want,
                tc.cs
            );
            assert_eq!(
                tc.want
                    .conditions
                    .iter()
                    .map(|c| c.observed_generation)
                    .collect::<Vec<_>>(),
                tc.cs
                    .conditions
                    .iter()
                    .map(|c| c.observed_generation)
                    .collect::<Vec<_>>(),
                "{}: the generation a replacement carries is not ignored here",
                tc.name
            );
        }
    }

    #[test]
    fn get_condition() {
        struct Case {
            name: &'static str,
            cs: ConditionedStatus,
            t: ConditionType,
            want: Condition,
        }
        let cases = vec![
            // go: core/v2 TestGetCondition/ConditionExists
            Case {
                name: "ConditionExists",
                cs: ConditionedStatus::new([Condition::available()]),
                t: ConditionType::READY,
                want: Condition::available(),
            },
            // go: core/v2 TestGetCondition/ConditionDoesNotExist
            Case {
                name: "ConditionDoesNotExist",
                cs: ConditionedStatus::new([Condition::available()]),
                t: ConditionType::SYNCED,
                want: of_type(ConditionType::SYNCED),
            },
        ];
        for tc in cases {
            let got = tc.cs.get_condition(&tc.t);
            assert!(
                tc.want.equal(&got),
                "{}: get_condition(...): want {:?}, got {:?}",
                tc.name,
                tc.want,
                got
            );
        }
    }

    #[test]
    fn condition_with_message() {
        struct Case {
            name: &'static str,
            c: Condition,
            msg: &'static str,
            want: Condition,
        }
        let test_msg = "Something went wrong on cloud side";
        let unavailable = |message: &str| {
            cond(
                ConditionType::READY,
                ConditionStatus::Unknown,
                ConditionReason::UNAVAILABLE,
                message,
                0,
            )
        };
        let cases = vec![
            // go: core/v2 TestConditionWithMessage/MessageAdded
            Case {
                name: "MessageAdded",
                c: unavailable(""),
                msg: test_msg,
                want: unavailable(test_msg),
            },
            // go: core/v2 TestConditionWithMessage/MessageChanged
            Case {
                name: "MessageChanged",
                c: unavailable("Some other message"),
                msg: test_msg,
                want: unavailable(test_msg),
            },
            // go: core/v2 TestConditionWithMessage/MessageCleared
            Case {
                name: "MessageCleared",
                c: unavailable(test_msg),
                msg: "",
                want: unavailable(""),
            },
        ];
        for tc in cases {
            let got = tc.c.with_message(tc.msg);
            assert!(tc.want.equal(&got), "{}: want {:?}, got {:?}", tc.name, tc.want, got);
            assert_eq!(tc.want.message, got.message, "{}: message", tc.name);
        }
    }

    #[test]
    fn condition_with_observed_generation() {
        struct Case {
            name: &'static str,
            c: Condition,
            observed_generation: i64,
            want: Condition,
        }
        let unavailable = |generation: i64| {
            cond(
                ConditionType::READY,
                ConditionStatus::Unknown,
                ConditionReason::UNAVAILABLE,
                "",
                generation,
            )
        };
        let cases = vec![
            // go: core/v2 TestConditionWithObservedGeneration/Added
            Case {
                name: "Added",
                c: unavailable(0),
                observed_generation: 10,
                want: unavailable(10),
            },
            // go: core/v2 TestConditionWithObservedGeneration/Changed
            Case {
                name: "Changed",
                c: unavailable(3),
                observed_generation: 10,
                want: unavailable(10),
            },
        ];
        for tc in cases {
            let got = tc.c.with_observed_generation(tc.observed_generation);
            assert!(tc.want.equal(&got), "{}: want {:?}, got {:?}", tc.name, tc.want, got);
            assert_eq!(
                tc.want.observed_generation, got.observed_generation,
                "{}: observed generation",
                tc.name
            );
        }
    }

    #[test]
    fn system_condition_type() {
        struct Case {
            name: &'static str,
            c: Condition,
            want: bool,
        }
        let typed = |t: &'static str| of_type(ConditionType(Cow::Borrowed(t)));
        let cases = vec![
            // go: core/v2 TestIsSystemConditionType/SystemReady
            Case {
                name: "SystemReady",
                c: typed("Ready"),
                want: true,
            },
            // go: core/v2 TestIsSystemConditionType/SystemSynced
            Case {
                name: "SystemSynced",
                c: typed("Synced"),
                want: true,
            },
            // go: core/v2 TestIsSystemConditionType/SystemHealthy
            Case {
                name: "SystemHealthy",
                c: typed("Healthy"),
                want: true,
            },
            // go: core/v2 TestIsSystemConditionType/Custom
            Case {
                name: "Custom",
                c: typed("Custom"),
                want: false,
            },
            // go: core/v2 TestIsSystemConditionType/UpToDate
            Case {
                name: "UpToDate",
                c: typed("UpToDate"),
                want: true,
            },
        ];
        for tc in cases {
            assert_eq!(
                tc.want,
                is_system_condition_type(&tc.c.type_),
                "{}: is_system_condition_type",
                tc.name
            );
        }
    }
}
