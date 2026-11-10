//! `pkg/reconciler/providerconfig`: count usages, block deletion while in use.

use std::borrow::Cow;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::{Client, ListOptions};
use crossplane_core::errors::{Error, Result, ResultExt, ignore_not_found};
use crossplane_core::meta;
use crossplane_core::resource::{Kind, ObjectType, ProviderConfig, ProviderConfigUsage};
use crossplane_core::v2::{
    Condition, ConditionReason, ConditionStatus, ConditionType, LABEL_KEY_PROVIDER_KIND, LABEL_KEY_PROVIDER_NAME,
};
use futures::FutureExt;
use futures::future::BoxFuture;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::jiff::Timestamp;

use crate::event::{Event, NopRecorder, Reason, Recorder};
use crate::reconciler::{Outcome, Reconcile, Request};

pub const FINALIZER_NAME: &str = "in-use.crossplane.io";
pub const SHORT_WAIT: Duration = Duration::from_secs(30);
pub const TIMEOUT: Duration = Duration::from_secs(120);

const ERR_GET_PC: &str = "cannot get ProviderConfig";
const ERR_LIST_PCUS: &str = "cannot list ProviderConfigUsages";
const ERR_DELETE_PCU: &str = "cannot delete ProviderConfigUsage";
const ERR_UPDATE: &str = "cannot update ProviderConfig";
const ERR_UPDATE_STATUS: &str = "cannot update ProviderConfig status";

const BLOCKING_MESSAGE: &str = "Blocking deletion while usages still exist";

pub const REASON_ACCOUNT: Reason = Reason(Cow::Borrowed("UsageAccounting"));
pub const TYPE_TERMINATING: ConditionType = ConditionType(Cow::Borrowed("Terminating"));
pub const REASON_IN_USE: ConditionReason = ConditionReason(Cow::Borrowed("InUse"));

/// Go `Terminating()`: a deleted ProviderConfig whose deletion is blocked because it is in use.
pub fn terminating() -> Condition {
    Condition {
        type_: TYPE_TERMINATING,
        status: ConditionStatus::True,
        last_transition_time: Time(Timestamp::now()),
        reason: REASON_IN_USE,
        message: String::new(),
        observed_generation: 0,
    }
}

/// Go `ControllerName(kind)`: `"providerconfig/" + lower(<Kind>.<group>)`.
pub fn controller_name<K: Kind>() -> String {
    format!("providerconfig/{}", ObjectType::of::<K>().group_kind()).to_lowercase()
}

/// Go `providerconfig.Reconciler`, typed by config and usage kind instead of `ProviderConfigKinds`
/// GVKs.
pub struct Reconciler<PC, U> {
    client: Arc<dyn Client>,
    recorder: Arc<dyn Recorder>,
    _kinds: PhantomData<fn() -> (PC, U)>,
}

pub struct ReconcilerBuilder<PC, U> {
    client: Arc<dyn Client>,
    recorder: Option<Arc<dyn Recorder>>,
    _kinds: PhantomData<fn() -> (PC, U)>,
}

impl<PC: ProviderConfig, U: ProviderConfigUsage> Reconciler<PC, U> {
    pub fn builder(client: Arc<dyn Client>) -> ReconcilerBuilder<PC, U> {
        ReconcilerBuilder {
            client,
            recorder: None,
            _kinds: PhantomData,
        }
    }
}

impl<PC: ProviderConfig, U: ProviderConfigUsage> ReconcilerBuilder<PC, U> {
    pub fn recorder(mut self, r: Arc<dyn Recorder>) -> Self {
        self.recorder = Some(r);
        self
    }

    pub fn build(self) -> Reconciler<PC, U> {
        Reconciler {
            client: self.client,
            recorder: self.recorder.unwrap_or_else(|| Arc::new(NopRecorder)),
            _kinds: PhantomData,
        }
    }
}

impl<PC: ProviderConfig, U: ProviderConfigUsage> Reconcile for Reconciler<PC, U> {
    /// PC1-PC8 (parity spec 4.1). Unconditional PUT and status PUT on every pass, as Go does; no
    /// conflict handling.
    ///
    /// OPEN (docs/design/core.md): at `TIMEOUT` this drops the whole run with a deadline error.
    /// Go lets each remaining Kubernetes call fail on the expired context and carries on through
    /// the branch that handles that failure, as the managed reconciler's note says.
    fn reconcile<'a>(&'a self, req: &'a Request) -> BoxFuture<'a, Result<Outcome>> {
        async move {
            tokio::time::timeout(TIMEOUT, self.run(req))
                .await
                .unwrap_or_else(|_| Err(Error::deadline_exceeded()))
        }
        .boxed()
    }
}

impl<PC: ProviderConfig, U: ProviderConfigUsage> Reconciler<PC, U> {
    async fn run(&self, req: &Request) -> Result<Outcome> {
        let mut pc = match self.client.get_as::<PC>(req).await {
            Ok(pc) => pc,
            Err(e) if e.is_not_found() => return Ok(Outcome::Done),
            Err(e) => return Err(e.wrap(ERR_GET_PC)),
        };

        let mut options = ListOptions::default();
        options
            .labels
            .insert(LABEL_KEY_PROVIDER_NAME.to_owned(), meta::name(&pc).to_owned());
        options
            .labels
            .insert(LABEL_KEY_PROVIDER_KIND.to_owned(), PC::kind(&()).into_owned());
        options.namespace = Some(meta::namespace(&pc).to_owned()).filter(|ns| !ns.is_empty());

        let usages = match self.client.list_as::<U>(&options).await {
            Ok(usages) => usages,
            Err(e) => {
                self.warn(&pc, e.wrap(ERR_LIST_PCUS));
                return Ok(Outcome::RequeueAfter(SHORT_WAIT));
            }
        };

        let mut users = usages.len() as i64;
        for usage in &usages {
            let controlled = kube::Resource::meta(usage)
                .owner_references
                .iter()
                .flatten()
                .any(|o| o.controller == Some(true));
            if controlled {
                continue;
            }
            if let Err(e) = ignore_not_found(self.client.delete(usage).await) {
                self.warn(&pc, e.wrap(ERR_DELETE_PCU));
                return Ok(Outcome::RequeueAfter(SHORT_WAIT));
            }
            users -= 1;
        }

        if meta::was_deleted(&pc) {
            if users > 0 {
                self.warn(&pc, Error::new(BLOCKING_MESSAGE));
                pc.set_users(users);
                pc.set_conditions([terminating().with_message(BLOCKING_MESSAGE)]);
                return self
                    .client
                    .update_status(&mut pc)
                    .await
                    .wrap_err(ERR_UPDATE_STATUS)
                    .map(|()| Outcome::Done);
            }

            meta::remove_finalizer(&mut pc, FINALIZER_NAME);
            return Ok(self.update_or_wait(&mut pc).await.unwrap_or(Outcome::Done));
        }

        meta::add_finalizer(&mut pc, FINALIZER_NAME);
        if let Some(wait) = self.update_or_wait(&mut pc).await {
            return Ok(wait);
        }

        pc.set_users(users);
        self.client
            .update_status(&mut pc)
            .await
            .wrap_err(ERR_UPDATE_STATUS)
            .map(|()| Outcome::Done)
    }

    /// The PUT of the ProviderConfig. A failure is not an error: Go logs it and requeues.
    async fn update_or_wait(&self, pc: &mut PC) -> Option<Outcome> {
        match self.client.update(pc).await {
            Ok(()) => None,
            Err(e) => {
                tracing::debug!(error = %e.wrap(ERR_UPDATE), "cannot update ProviderConfig");
                Some(Outcome::RequeueAfter(SHORT_WAIT))
            }
        }
    }

    fn warn(&self, pc: &PC, err: Error) {
        self.recorder.event(pc, Event::warning(REASON_ACCOUNT, &err));
    }
}

#[cfg(test)]
#[path = "providerconfig_test.rs"]
mod tests;
