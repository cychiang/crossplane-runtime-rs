//! The production `Client`: reads from the informer cache, writes straight to the API server
//! with the verbs Go uses, so the audit trace keeps Go's verbs.
//!
//! This file and `cache.rs` are the only ones that touch `kube::Api`, and [`convert_error`] is
//! the only place a `kube::Error` becomes an `Error`.

use crossplane_core::client::{Client, ListOptions, Patch};
use crossplane_core::errors::{Error, Result, StatusReason};
use crossplane_core::resource::{Object, ObjectKey, ObjectType};
use futures::FutureExt;
use futures::future::BoxFuture;
use kube::api::{Api, DeleteParams, DynamicObject, ListParams, Patch as KubePatch, PatchParams, PostParams};

use crate::cache::Cache;

pub(crate) struct KubeClient {
    kube: kube::Client,
    reads: Reads,
}

/// Go's manager client reads through the informer cache; `client.New` reads the API server.
enum Reads {
    Cached(Cache),
    Direct,
}

/// Builds the one `kube::Client` a process uses, sending `user_agent` on every request. Go sets
/// it through `rest.Config.UserAgent`; kube has no such field, so it is a default header.
pub(crate) fn connect(mut config: kube::Config, user_agent: &str) -> Result<kube::Client> {
    let value = http::HeaderValue::from_str(user_agent).map_err(Error::foreign)?;
    config.headers.push((http::header::USER_AGENT, value));
    kube::Client::try_from(config).map_err(convert_error)
}

impl KubeClient {
    pub(crate) fn new(kube: kube::Client, cache: Cache) -> Self {
        Self {
            kube,
            reads: Reads::Cached(cache),
        }
    }

    /// Reads go to the API server, as Go's `client.New` and `mgr.GetAPIReader()` do.
    pub(crate) fn uncached(kube: kube::Client) -> Self {
        Self {
            kube,
            reads: Reads::Direct,
        }
    }

    fn api_in(&self, object_type: &ObjectType, namespace: Option<&str>) -> Api<DynamicObject> {
        let resource = object_type.api_resource();
        match namespace {
            Some(namespace) => Api::namespaced_with(self.kube.clone(), namespace, resource),
            None => Api::all_with(self.kube.clone(), resource),
        }
    }

    fn api(&self, obj: &dyn Object) -> Api<DynamicObject> {
        self.api_in(&obj.object_type(), obj.meta().namespace.as_deref())
    }

    async fn get_direct(&self, object_type: &ObjectType, key: &ObjectKey) -> Result<Box<dyn Object>> {
        let found = self
            .api_in(object_type, key.namespace.as_deref())
            .get(&key.name)
            .await
            .map_err(convert_error)?;
        object_type.decode(serde_json::to_value(found).map_err(Error::foreign)?)
    }

    async fn list_direct(&self, object_type: &ObjectType, options: &ListOptions) -> Result<Vec<Box<dyn Object>>> {
        let selector = options
            .labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(",");
        let params = ListParams {
            label_selector: (!selector.is_empty()).then_some(selector),
            ..ListParams::default()
        };
        let list = self
            .api_in(object_type, options.namespace.as_deref())
            .list(&params)
            .await
            .map_err(convert_error)?;
        list.items
            .into_iter()
            .map(|item| object_type.decode(serde_json::to_value(item).map_err(Error::foreign)?))
            .collect()
    }
}

fn name(obj: &dyn Object) -> Result<String> {
    obj.meta().name.clone().ok_or_else(|| Error::new("object has no name"))
}

fn body(obj: &dyn Object) -> Result<DynamicObject> {
    serde_json::from_value(obj.to_json()?).map_err(Error::foreign)
}

fn reflect(obj: &mut dyn Object, response: DynamicObject) -> Result<()> {
    obj.set_from_json(serde_json::to_value(response).map_err(Error::foreign)?)
}

/// apimachinery's `IsNotFound` and friends: the reason when the server names a known one,
/// otherwise the HTTP code.
fn convert_error(err: kube::Error) -> Error {
    match &err {
        kube::Error::Api(status) => {
            let reason = match (status.reason.as_str(), status.code) {
                ("NotFound", _) => StatusReason::NotFound,
                ("AlreadyExists", _) => StatusReason::AlreadyExists,
                ("Conflict", _) => StatusReason::Conflict,
                ("Forbidden", _) => StatusReason::Forbidden,
                ("Invalid", _) => StatusReason::Invalid,
                (other, code) if !KNOWN_REASONS.contains(&other) => match code {
                    404 => StatusReason::NotFound,
                    409 => StatusReason::Conflict,
                    403 => StatusReason::Forbidden,
                    422 => StatusReason::Invalid,
                    _ => StatusReason::Other(other.to_owned()),
                },
                (other, _) => StatusReason::Other(other.to_owned()),
            };
            Error::status(reason, status.message.clone())
        }
        _ => Error::foreign(err),
    }
}

/// apimachinery's `knownReasons`, less the five matched by name above.
const KNOWN_REASONS: [&str; 14] = [
    "Unauthorized",
    "Gone",
    "ServerTimeout",
    "StoreReadError",
    "Timeout",
    "TooManyRequests",
    "BadRequest",
    "MethodNotAllowed",
    "NotAcceptable",
    "RequestEntityTooLarge",
    "UnsupportedMediaType",
    "InternalError",
    "Expired",
    "ServiceUnavailable",
];

impl Client for KubeClient {
    fn get<'a>(&'a self, object_type: &'a ObjectType, key: &'a ObjectKey) -> BoxFuture<'a, Result<Box<dyn Object>>> {
        match &self.reads {
            Reads::Cached(cache) => cache.get(object_type, key).boxed(),
            Reads::Direct => self.get_direct(object_type, key).boxed(),
        }
    }

    fn list<'a>(
        &'a self,
        object_type: &'a ObjectType,
        options: &'a ListOptions,
    ) -> BoxFuture<'a, Result<Vec<Box<dyn Object>>>> {
        match &self.reads {
            Reads::Cached(cache) => cache.list(object_type, options).boxed(),
            Reads::Direct => self.list_direct(object_type, options).boxed(),
        }
    }

    fn create<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            let response = self
                .api(obj)
                .create(&PostParams::default(), &body(obj)?)
                .await
                .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn update<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            let response = self
                .api(obj)
                .replace(&name(obj)?, &PostParams::default(), &body(obj)?)
                .await
                .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn update_status<'a>(&'a self, obj: &'a mut dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            let response = self
                .api(obj)
                .replace_status(&name(obj)?, &PostParams::default(), &body(obj)?)
                .await
                .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn patch<'a>(&'a self, obj: &'a mut dyn Object, patch: Patch) -> BoxFuture<'a, Result<()>> {
        async move {
            let (name, api) = (name(obj)?, self.api(obj));
            let response = match patch {
                Patch::Merge(body) => api.patch(&name, &PatchParams::default(), &KubePatch::Merge(body)).await,
                Patch::Apply {
                    body,
                    field_manager,
                    force,
                } => {
                    let mut params = PatchParams::apply(&field_manager);
                    params.force = force;
                    api.patch(&name, &params, &KubePatch::Apply(body)).await
                }
            }
            .map_err(convert_error)?;
            reflect(obj, response)
        }
        .boxed()
    }

    fn delete<'a>(&'a self, obj: &'a dyn Object) -> BoxFuture<'a, Result<()>> {
        async move {
            self.api(obj)
                .delete(&name(obj)?, &DeleteParams::default())
                .await
                .map(|_| ())
                .map_err(convert_error)
        }
        .boxed()
    }
}

#[cfg(test)]
#[path = "kube_client_test.rs"]
mod tests;
