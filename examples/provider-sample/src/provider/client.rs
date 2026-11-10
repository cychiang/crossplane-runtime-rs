//! Go `internal/provider/client.go`. THIS FILE IS YOURS: the template never overwrites it.
//!
//! The client of the fake external service under `parity/fake-external`. Its status handling and
//! error texts are the parity contract (`parity/README.md`), so they copy Go's.

use std::sync::Arc;
use std::time::Duration;

use crossplane_core::errors::{Error, Result};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::Client as HttpClient;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::{Deserialize, Serialize};

use crate::apis::v1alpha1::ProviderConfigSpec;

/// Go's `http.Client{Timeout: 10 * time.Second}`: the whole exchange, body included.
const TIMEOUT: Duration = Duration::from_secs(10);

/// What `Client::new` gets from the resolved ProviderConfig.
pub struct ClientConfig {
    pub spec: ProviderConfigSpec,
    /// Extracted per the ProviderConfig's credentials source.
    pub credentials: Vec<u8>,
    pub kube: Arc<dyn crossplane_core::client::Client>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thing {
    pub configurable_field: String,
}

/// Talks to the external API this provider manages.
pub struct Client {
    endpoint: String,
    http: HttpClient<HttpConnector, Full<Bytes>>,
}

impl Client {
    pub async fn new(_cfg: ClientConfig) -> Result<Self> {
        Ok(Self::with_endpoint(super::options::endpoint()))
    }

    fn with_endpoint(endpoint: String) -> Self {
        Self {
            endpoint,
            http: HttpClient::builder(TokioExecutor::new()).build_http(),
        }
    }

    /// `None` when the thing does not exist.
    pub async fn get(&self, name: &str) -> Result<Option<Thing>> {
        match self.call(Method::GET, name, None).await? {
            (StatusCode::NOT_FOUND, _) => Ok(None),
            (StatusCode::OK, thing) => Ok(thing),
            (code, _) => Err(status_error(&Method::GET, name, code)),
        }
    }

    pub async fn put(&self, name: &str, thing: Thing) -> Result<Thing> {
        match self.call(Method::PUT, name, Some(&thing)).await? {
            (StatusCode::OK | StatusCode::CREATED, Some(thing)) => Ok(thing),
            (code, _) => Err(status_error(&Method::PUT, name, code)),
        }
    }

    /// A thing that is already gone counts as deleted.
    pub async fn delete(&self, name: &str) -> Result<()> {
        match self.call(Method::DELETE, name, None).await? {
            (StatusCode::NO_CONTENT | StatusCode::NOT_FOUND, _) => Ok(()),
            (code, _) => Err(status_error(&Method::DELETE, name, code)),
        }
    }

    /// Go's `do`: the status code, and the decoded thing when the status is 200 or 201.
    async fn call(&self, method: Method, name: &str, thing: Option<&Thing>) -> Result<(StatusCode, Option<Thing>)> {
        // Go's `json.Encoder` ends the body with a newline.
        let body = match thing {
            Some(t) => {
                let mut b = serde_json::to_vec(t).map_err(Error::foreign)?;
                b.push(b'\n');
                Bytes::from(b)
            }
            None => Bytes::new(),
        };
        // Kubernetes object names need no path escaping, so Go's `url.PathEscape` is the identity.
        let url = format!("{}/things/{name}", self.endpoint);
        let request = Request::builder()
            .method(method.clone())
            .uri(&url)
            .header(hyper::header::CONTENT_TYPE, "application/json")
            .body(Full::new(body))
            .map_err(Error::foreign)?;
        let exchange = async {
            let response = self
                .http
                .request(request)
                .await
                .map_err(|e| url_error(&method, &url, chain(&e)))?;
            let code = response.status();
            if code != StatusCode::OK && code != StatusCode::CREATED {
                return Ok((code, None));
            }
            let bytes = response
                .into_body()
                .collect()
                .await
                .map_err(|e| url_error(&method, &url, chain(&e)))?
                .to_bytes();
            let thing = serde_json::from_slice(&bytes)
                .map_err(|e| Error::new(format!("{method} /things/{name}: cannot decode response: {e}")))?;
            Ok((code, Some(thing)))
        };
        tokio::time::timeout(TIMEOUT, exchange).await.unwrap_or_else(|_| {
            Err(url_error(
                &method,
                &url,
                "context deadline exceeded (Client.Timeout exceeded while awaiting headers)".into(),
            ))
        })
    }
}

fn status_error(method: &Method, name: &str, code: StatusCode) -> Error {
    Error::new(format!("{method} /things/{name}: status {}", code.as_u16()))
}

/// Go's `url.Error` text, `<Method> "<url>": <cause>`.
fn url_error(method: &Method, url: &str, cause: String) -> Error {
    let op = match *method {
        Method::GET => "Get",
        Method::PUT => "Put",
        Method::DELETE => "Delete",
        _ => method.as_str(),
    };
    Error::new(format!("{op} \"{url}\": {cause}"))
}

/// An error and its sources, joined as Go's wrapped errors print. hyper's own text alone is only
/// "client error (Connect)".
fn chain(err: &dyn std::error::Error) -> String {
    std::iter::successors(Some(err), |e| e.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
#[path = "client_test.rs"]
mod tests;
