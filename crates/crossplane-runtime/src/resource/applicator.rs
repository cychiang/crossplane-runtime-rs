use std::sync::Arc;
use std::time::Duration;

use crossplane_core::client::{Client, Patch};
use crossplane_core::errors::{Error, Result, ResultExt};
use crossplane_core::resource::Object;
use futures::FutureExt;
use futures::future::BoxFuture;

pub use super::apply_option::ApplyOption;

/// Go `resource.Applicator`.
pub trait Applicator: Send + Sync + 'static {
    fn apply<'a>(&'a self, desired: &'a mut dyn Object, opts: &'a [ApplyOption]) -> BoxFuture<'a, Result<()>>;
}

type ApplyFnBox = Box<dyn Fn(&mut dyn Object, &[ApplyOption]) -> Result<()> + Send + Sync>;

/// Go `resource.ApplyFn`.
pub struct ApplyFn(pub ApplyFnBox);

impl Applicator for ApplyFn {
    fn apply<'a>(&'a self, desired: &'a mut dyn Object, opts: &'a [ApplyOption]) -> BoxFuture<'a, Result<()>> {
        futures::future::ready((self.0)(desired, opts)).boxed()
    }
}

fn create_without_name(desired: &dyn Object) -> bool {
    let meta = desired.meta();
    meta.name.as_deref().unwrap_or("").is_empty() && !meta.generate_name.as_deref().unwrap_or("").is_empty()
}

/// Go `APIPatchingApplicator`.
pub struct ApiPatchingApplicator {
    client: Arc<dyn Client>,
}

impl ApiPatchingApplicator {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }

    /// Creates the object when it does not exist, else runs the options and sends a merge patch
    /// whose body is the whole desired object. As in Go the caller's object becomes what the
    /// server holds, so it is the current object while the options run.
    pub async fn apply(&self, desired: &mut dyn Object, opts: &[ApplyOption]) -> Result<()> {
        if create_without_name(desired) {
            return self.client.create(desired).await.wrap_err("cannot create object");
        }

        let body = desired.to_json()?;
        let snapshot = desired.object_type().decode(body.clone())?;

        match self.client.get(&desired.object_type(), &desired.key()).await {
            Err(err) if err.is_not_found() => {
                return self.client.create(desired).await.wrap_err("cannot create object");
            }
            Err(err) => return Err(err.wrap("cannot get object")),
            Ok(current) => desired.set_from_json(current.to_json()?)?,
        }

        for opt in opts {
            opt.run(desired, &*snapshot)?;
        }

        self.client
            .patch(desired, Patch::Merge(body))
            .await
            .wrap_err("cannot patch object")
    }
}

impl Applicator for ApiPatchingApplicator {
    fn apply<'a>(&'a self, desired: &'a mut dyn Object, opts: &'a [ApplyOption]) -> BoxFuture<'a, Result<()>> {
        ApiPatchingApplicator::apply(self, desired, opts).boxed()
    }
}

/// Go `APIUpdatingApplicator`.
pub struct ApiUpdatingApplicator {
    client: Arc<dyn Client>,
}

impl ApiUpdatingApplicator {
    pub fn new(client: Arc<dyn Client>) -> Self {
        Self { client }
    }

    /// Creates the object when it does not exist, else runs the options and replaces it with
    /// the desired object carrying the current resource version.
    pub async fn apply(&self, desired: &mut dyn Object, opts: &[ApplyOption]) -> Result<()> {
        if create_without_name(desired) {
            return self.client.create(desired).await.wrap_err("cannot create object");
        }

        let mut current = match self.client.get(&desired.object_type(), &desired.key()).await {
            Err(err) if err.is_not_found() => {
                return self.client.create(desired).await.wrap_err("cannot create object");
            }
            Err(err) => return Err(err.wrap("cannot get object")),
            Ok(current) => current,
        };

        for opt in opts {
            opt.run(&mut *current, desired)?;
        }

        desired.meta_mut().resource_version = current.meta().resource_version.clone();
        self.client.update(desired).await.wrap_err("cannot update object")
    }
}

impl Applicator for ApiUpdatingApplicator {
    fn apply<'a>(&'a self, desired: &'a mut dyn Object, opts: &'a [ApplyOption]) -> BoxFuture<'a, Result<()>> {
        ApiUpdatingApplicator::apply(self, desired, opts).boxed()
    }
}

/// Go `wait.Backoff`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Backoff {
    pub duration: Duration,
    pub factor: f64,
    pub jitter: f64,
    pub steps: i32,
    pub cap: Duration,
}

/// Go `retry.DefaultRetry`.
pub const DEFAULT_RETRY: Backoff = Backoff {
    duration: Duration::from_millis(10),
    factor: 1.0,
    jitter: 0.1,
    steps: 5,
    cap: Duration::ZERO,
};

impl Backoff {
    /// Go `Backoff.Step`: the next wait, advancing the schedule.
    fn step(&mut self) -> Duration {
        if self.steps < 1 {
            return self.jittered(self.duration);
        }
        self.steps -= 1;
        let wait = self.duration;
        if self.factor != 0.0 {
            self.duration = self.duration.mul_f64(self.factor);
            if !self.cap.is_zero() && self.duration > self.cap {
                self.duration = self.cap;
                self.steps = 0;
            }
        }
        self.jittered(wait)
    }

    fn jittered(&self, d: Duration) -> Duration {
        if self.jitter > 0.0 {
            d + d.mul_f64(random_unit() * self.jitter)
        } else {
            d
        }
    }
}

fn random_unit() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let bits = std::collections::hash_map::RandomState::new().build_hasher().finish();
    (bits >> 11) as f64 / (1u64 << 53) as f64
}

type ShouldRetry = Box<dyn Fn(&Error) -> bool + Send + Sync>;

/// Go `ApplicatorWithRetry`: retries the nested applicator on the errors `should_retry` names.
pub struct ApplicatorWithRetry {
    applicator: Arc<dyn Applicator>,
    should_retry: ShouldRetry,
    backoff: Backoff,
}

impl ApplicatorWithRetry {
    /// A `None` backoff is [`DEFAULT_RETRY`].
    pub fn new(
        applicator: Arc<dyn Applicator>,
        should_retry: impl Fn(&Error) -> bool + Send + Sync + 'static,
        backoff: Option<Backoff>,
    ) -> Self {
        Self {
            applicator,
            should_retry: Box::new(should_retry),
            backoff: backoff.unwrap_or(DEFAULT_RETRY),
        }
    }

    pub fn backoff(&self) -> &Backoff {
        &self.backoff
    }
}

impl Applicator for ApplicatorWithRetry {
    /// Go `retry.OnError`: at most `steps` attempts, the last retriable error returned when
    /// they run out.
    fn apply<'a>(&'a self, desired: &'a mut dyn Object, opts: &'a [ApplyOption]) -> BoxFuture<'a, Result<()>> {
        async move {
            let mut backoff = self.backoff;
            let mut last = None;
            while backoff.steps > 0 {
                match self.applicator.apply(&mut *desired, opts).await {
                    Ok(()) => return Ok(()),
                    Err(err) if (self.should_retry)(&err) => last = Some(err),
                    Err(err) => return Err(err),
                }
                if backoff.steps == 1 {
                    break;
                }
                tokio::time::sleep(backoff.step()).await;
            }
            last.map_or(Ok(()), Err)
        }
        .boxed()
    }
}

#[cfg(test)]
#[path = "applicator_test.rs"]
mod tests;
