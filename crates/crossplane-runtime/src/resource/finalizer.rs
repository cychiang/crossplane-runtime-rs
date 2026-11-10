use std::sync::Arc;

use crossplane_core::client::Client;
use crossplane_core::errors::{Result, ResultExt, ignore_not_found};
use crossplane_core::meta;
use crossplane_core::resource::Kind;
use futures::FutureExt;
use futures::future::BoxFuture;

/// Go `resource.Finalizer`. Add and remove are separate so a table can stub one and pin the
/// other as "not called".
pub trait Finalizer<K>: Send + Sync + 'static {
    fn add_finalizer<'a>(&'a self, obj: &'a mut K) -> BoxFuture<'a, Result<()>>;
    fn remove_finalizer<'a>(&'a self, obj: &'a mut K) -> BoxFuture<'a, Result<()>>;
}

/// Go `resource.APIFinalizer`. It writes with PUT, not kube's JSON-patch finalizer helper, so
/// the audit trace keeps Go's verb.
pub struct ApiFinalizer {
    client: Arc<dyn Client>,
    finalizer: &'static str,
}

impl ApiFinalizer {
    pub fn new(client: Arc<dyn Client>, finalizer: &'static str) -> Self {
        Self { client, finalizer }
    }
}

const ERR_UPDATE_OBJECT: &str = "cannot update object";

impl<K: Kind> Finalizer<K> for ApiFinalizer {
    fn add_finalizer<'a>(&'a self, obj: &'a mut K) -> BoxFuture<'a, Result<()>> {
        async move {
            if meta::finalizer_exists(obj, self.finalizer) {
                return Ok(());
            }
            meta::add_finalizer(obj, self.finalizer);
            self.client.update(obj).await.wrap_err(ERR_UPDATE_OBJECT)
        }
        .boxed()
    }

    fn remove_finalizer<'a>(&'a self, obj: &'a mut K) -> BoxFuture<'a, Result<()>> {
        async move {
            if !meta::finalizer_exists(obj, self.finalizer) {
                return Ok(());
            }
            meta::remove_finalizer(obj, self.finalizer);
            ignore_not_found(self.client.update(obj).await).wrap_err(ERR_UPDATE_OBJECT)
        }
        .boxed()
    }
}

type FinalizerFn<K> = Box<dyn Fn(&mut K) -> Result<()> + Send + Sync>;

/// Go `resource.FinalizerFns`. A `None` field panics when called.
pub struct FinalizerFns<K> {
    pub add_finalizer: Option<FinalizerFn<K>>,
    pub remove_finalizer: Option<FinalizerFn<K>>,
}

impl<K> Default for FinalizerFns<K> {
    fn default() -> Self {
        Self {
            add_finalizer: None,
            remove_finalizer: None,
        }
    }
}

impl<K: Send + 'static> Finalizer<K> for FinalizerFns<K> {
    fn add_finalizer<'a>(&'a self, obj: &'a mut K) -> BoxFuture<'a, Result<()>> {
        let f = self
            .add_finalizer
            .as_ref()
            .expect("unexpected call to FinalizerFns::add_finalizer");
        futures::future::ready(f(obj)).boxed()
    }

    fn remove_finalizer<'a>(&'a self, obj: &'a mut K) -> BoxFuture<'a, Result<()>> {
        let f = self
            .remove_finalizer
            .as_ref()
            .expect("unexpected call to FinalizerFns::remove_finalizer");
        futures::future::ready(f(obj)).boxed()
    }
}

#[cfg(test)]
#[path = "finalizer_test.rs"]
mod tests;
