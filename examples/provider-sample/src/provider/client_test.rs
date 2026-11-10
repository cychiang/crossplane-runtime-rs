use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Response, header};
use hyper_util::rt::TokioIo;
use serde_json::Value;

use super::*;

/// One request as the stub saw it: method, path, Content-Type and body.
type Seen = (String, String, String, String);

/// The fake external service's thing API (`parity/README.md`), in process. `fail` answers the
/// next thing request with that status, as the service's injected faults do.
#[derive(Clone, Default)]
struct Stub {
    things: Arc<Mutex<HashMap<String, Value>>>,
    seen: Arc<Mutex<Vec<Seen>>>,
    fail: Arc<Mutex<Option<u16>>>,
}

impl Stub {
    async fn serve(self) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let stub = self.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let stub = stub.clone();
                        async move { Ok::<_, Infallible>(stub.answer(req).await) }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        format!("http://{address}")
    }

    async fn answer(&self, req: Request<Incoming>) -> Response<Full<Bytes>> {
        let method = req.method().to_string();
        let path = req.uri().path().to_owned();
        let content_type = req
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_owned())
            .unwrap_or_default();
        let body = req.into_body().collect().await.unwrap().to_bytes();
        self.seen.lock().unwrap().push((
            method.clone(),
            path.clone(),
            content_type,
            String::from_utf8(body.to_vec()).unwrap(),
        ));
        if let Some(status) = self.fail.lock().unwrap().take() {
            return respond(status, None);
        }
        let name = path.strip_prefix("/things/").unwrap().to_owned();
        let mut things = self.things.lock().unwrap();
        match method.as_str() {
            "GET" => match things.get(&name) {
                Some(t) => respond(200, Some(t)),
                None => respond(404, None),
            },
            "PUT" => match serde_json::from_slice::<Thing>(&body) {
                Ok(t) => {
                    let t = serde_json::to_value(t).unwrap();
                    let status = if things.insert(name, t.clone()).is_some() {
                        200
                    } else {
                        201
                    };
                    respond(status, Some(&t))
                }
                Err(_) => respond(400, None),
            },
            "DELETE" => match things.remove(&name) {
                Some(_) => respond(204, None),
                None => respond(404, None),
            },
            _ => respond(405, None),
        }
    }
}

fn respond(status: u16, thing: Option<&Value>) -> Response<Full<Bytes>> {
    let body = thing.map(|t| Bytes::from(t.to_string())).unwrap_or_default();
    Response::builder().status(status).body(Full::new(body)).unwrap()
}

fn thing(value: &str) -> Thing {
    Thing {
        configurable_field: value.into(),
    }
}

#[tokio::test]
async fn the_thing_lifecycle_follows_the_contract() {
    let stub = Stub::default();
    let client = Client::with_endpoint(stub.clone().serve().await);

    assert_eq!(client.get("a").await.unwrap(), None, "404 is no thing");
    assert_eq!(client.put("a", thing("x")).await.unwrap(), thing("x"), "201 creates");
    assert_eq!(client.get("a").await.unwrap(), Some(thing("x")));
    assert_eq!(client.put("a", thing("y")).await.unwrap(), thing("y"), "200 replaces");
    client.delete("a").await.unwrap();
    client.delete("a").await.expect("404 on delete counts as deleted");

    let seen = stub.seen.lock().unwrap().clone();
    let json = "application/json".to_owned();
    let want: Vec<Seen> = vec![
        ("GET".into(), "/things/a".into(), json.clone(), String::new()),
        (
            "PUT".into(),
            "/things/a".into(),
            json.clone(),
            "{\"configurableField\":\"x\"}\n".into(),
        ),
        ("GET".into(), "/things/a".into(), json.clone(), String::new()),
        (
            "PUT".into(),
            "/things/a".into(),
            json.clone(),
            "{\"configurableField\":\"y\"}\n".into(),
        ),
        ("DELETE".into(), "/things/a".into(), json.clone(), String::new()),
        ("DELETE".into(), "/things/a".into(), json, String::new()),
    ];
    assert_eq!(seen, want);
}

#[tokio::test]
async fn an_unexpected_status_is_the_contract_error() {
    let stub = Stub::default();
    let client = Client::with_endpoint(stub.clone().serve().await);

    *stub.fail.lock().unwrap() = Some(500);
    let err = client.put("a", thing("x")).await.unwrap_err();
    assert_eq!(err.to_string(), "PUT /things/a: status 500");

    *stub.fail.lock().unwrap() = Some(503);
    assert_eq!(
        client.get("a").await.unwrap_err().to_string(),
        "GET /things/a: status 503"
    );

    *stub.fail.lock().unwrap() = Some(500);
    assert_eq!(
        client.delete("a").await.unwrap_err().to_string(),
        "DELETE /things/a: status 500"
    );
}

#[tokio::test]
async fn an_unreachable_service_names_the_request() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let err = Client::with_endpoint(endpoint.clone()).get("a").await.unwrap_err();
    let text = err.to_string();
    assert!(
        text.starts_with(&format!("Get \"{endpoint}/things/a\": client error (Connect)")),
        "{text}"
    );
}

#[tokio::test]
async fn the_name_is_path_escaped_as_go_escapes_it() {
    let stub = Stub::default();
    let client = Client::with_endpoint(stub.clone().serve().await);
    client.get("a b/c?d;e,f%$&+:=@~").await.unwrap();
    let seen = stub.seen.lock().unwrap();
    assert_eq!(seen[0].1, "/things/a%20b%2Fc%3Fd%3Be%2Cf%25$&+:=@~");
}
