use serde_json::{json, Value};
use std::collections::BTreeSet;
use trace_diff::allow;
use trace_diff::diff::{Difference, Kind, Layer};
use trace_diff::run::{Call, Report, Run, Write};
use trace_diff::{compare, render, Options};

fn write(verb: &str, group: &str, resource: &str, sub: Option<&str>, name: &str, ts: &str, obj: Value) -> Write {
    serde_json::from_value(json!({
        "verb": verb, "apiGroup": group, "resource": resource, "subresource": sub,
        "namespace": "ns", "name": name, "code": if verb == "create" { 201 } else { 200 },
        "requestObject": obj, "requestReceivedTimestamp": ts,
    }))
    .unwrap()
}

fn mr(uid: &str, rv: &str, finalizers: &[&str], conditions: Value) -> Value {
    json!({
        "apiVersion": "sample.example.com/v1alpha1", "kind": "MyType",
        "metadata": {"name": "a", "namespace": "ns", "uid": uid, "resourceVersion": rv,
                     "creationTimestamp": "2026-11-09T22:43:19Z", "finalizers": finalizers,
                     "managedFields": [{"manager": "m", "time": "2026-11-09T22:43:19Z"}]},
        "spec": {"forProvider": {"configurableField": "v1"}},
        "status": {"conditions": conditions}
    })
}

fn cond(kind: &str, status: &str, reason: &str, message: Option<&str>) -> Value {
    let mut c = json!({"type": kind, "status": status, "reason": reason, "lastTransitionTime": "2026-11-09T22:43:19Z"});
    if let Some(m) = message {
        c["message"] = json!(m);
    }
    c
}

fn event(uid: &str, name: &str, kind: &str, reason: &str, message: &str) -> Value {
    json!({"kind": "Event", "metadata": {"name": name, "namespace": "ns"},
           "involvedObject": {"kind": "MyType", "namespace": "ns", "name": "a", "uid": uid},
           "type": kind, "reason": reason, "message": message, "count": 1})
}

fn call(seq: u64, method: &str, status: u16, body: Option<Value>) -> Call {
    serde_json::from_value(json!({"seq": seq, "method": method, "path": "/things/a", "body": body,
                                  "status": status, "userAgent": "Go-http-client/1.1"}))
    .unwrap()
}

fn report(step_status: &str) -> Report {
    serde_json::from_value(json!({"tests": [{"name": "lifecycle", "status": step_status,
        "steps": [{"name": "create", "status": "passed"}, {"name": "delete", "status": step_status}]}]}))
    .unwrap()
}

/// A run in the shape of the lifecycle scenario. `rv` and the timestamps vary
/// so the baseline already exercises the normalization rules.
fn base(rv: u32, ts_minute: u32) -> Run {
    let ts = |s: u32| format!("2026-11-09T22:{ts_minute:02}:{s:02}.{:06}Z", s * 1000);
    let r = |n: u32| (rv + n).to_string();
    let uid = format!("{rv:08x}-8ac6-4054-85ed-8ddb3651eb02");
    let uid = uid.as_str();
    let available = json!([
        cond("Ready", "True", "Available", None),
        cond("Synced", "True", "ReconcileSuccess", None)
    ]);
    let writes = vec![
        write(
            "update",
            "sample.example.com",
            "mytypes",
            None,
            "a",
            &ts(1),
            mr(uid, &r(1), &["finalizer.managedresource.crossplane.io"], json!([])),
        ),
        write(
            "update",
            "sample.example.com",
            "mytypes",
            Some("status"),
            "a",
            &ts(2),
            mr(
                uid,
                &r(2),
                &["finalizer.managedresource.crossplane.io"],
                json!([cond("Ready", "False", "Creating", None)]),
            ),
        ),
        write(
            "update",
            "sample.example.com",
            "mytypes",
            Some("status"),
            "a",
            &ts(3),
            mr(
                uid,
                &r(3),
                &["finalizer.managedresource.crossplane.io"],
                available.clone(),
            ),
        ),
        write(
            "update",
            "sample.example.com",
            "mytypes",
            Some("status"),
            "a",
            &ts(4),
            mr(uid, &r(4), &["finalizer.managedresource.crossplane.io"], available),
        ),
        write(
            "create",
            "example.com",
            "providerconfigusages",
            None,
            uid,
            &ts(1),
            json!({"metadata": {"name": uid, "ownerReferences": [{"kind": "MyType", "name": "a", "uid": uid}]}}),
        ),
        write(
            "create",
            "",
            "events",
            None,
            &format!("a.18e681c8{rv:08x}"),
            &ts(2),
            event(
                uid,
                &format!("a.18e681c8{rv:08x}"),
                "Normal",
                "CreatedExternalResource",
                "Successfully requested creation of external resource",
            ),
        ),
    ];
    let calls = vec![
        call(1, "GET", 404, None),
        call(2, "PUT", 201, Some(json!({"configurableField": "v1"}))),
        call(3, "GET", 200, None),
        call(4, "DELETE", 204, None),
        call(5, "GET", 404, None),
    ];
    Run {
        writes,
        calls,
        report: report("passed"),
    }
}

fn opts() -> Options {
    Options {
        layers: [Layer::Chainsaw, Layer::Audit, Layer::Events, Layer::Calls].into(),
        disabled: BTreeSet::new(),
        allowances: vec![],
    }
}

fn diffs(a: &Run, b: &Run) -> Vec<Difference> {
    let o = compare(a, b, &opts());
    assert!(!o.equal(), "expected a difference, got:\n{}", render(&o, &opts()));
    o.differences.into_iter().map(|(d, _)| d).collect()
}

fn mr_writes(run: &mut Run) -> impl Iterator<Item = &mut Write> {
    run.writes.iter_mut().filter(|w| w.resource == "mytypes")
}

#[test]
fn two_runs_that_differ_only_in_identity_and_timing_are_equal() {
    let (a, mut b) = (base(100, 43), base(5000, 51));
    b.writes.insert(4, b.writes[3].clone());
    b.calls.insert(3, call(9, "GET", 200, None));
    let o = compare(&a, &b, &opts());
    assert!(o.equal(), "{}", render(&o, &opts()));
}

#[test]
fn a_missing_status_write_fails() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    b.writes.remove(1);
    let d = diffs(&a, &b);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(
        (d[0].layer, d[0].kind, d[0].a.as_str()),
        (Layer::Audit, Kind::Missing, "#2 update status")
    );
}

#[test]
fn a_different_condition_reason_fails() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    let w = mr_writes(&mut b).nth(1).unwrap();
    w.request_object.as_mut().unwrap()["status"]["conditions"][0]["reason"] = json!("Unavailable");
    let d = diffs(&a, &b);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].kind, Kind::Changed);
    assert_eq!(
        d[0].detail,
        vec![r#".status.conditions[0].reason: "Creating" -> "Unavailable""#]
    );
}

#[test]
fn an_extra_finalizer_fails() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    for w in mr_writes(&mut b) {
        w.request_object.as_mut().unwrap()["metadata"]["finalizers"]
            .as_array_mut()
            .unwrap()
            .push(json!("example.com/extra"));
    }
    let d = diffs(&a, &b);
    assert!(
        d.iter().all(|d| d.layer == Layer::Audit && d.kind == Kind::Changed),
        "{d:?}"
    );
    assert!(
        d[0].detail[0].starts_with(".metadata.finalizers: "),
        "{:?}",
        d[0].detail
    );
}

#[test]
fn a_reordered_put_and_delete_in_the_call_log_fails() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    b.calls[1].seq = 4;
    b.calls[3].seq = 2;
    let d = diffs(&a, &b);
    assert!(d.iter().all(|d| d.layer == Layer::Calls), "{d:?}");
    assert!(!d.is_empty());
}

#[test]
fn an_update_replaced_by_a_patch_on_a_managed_resource_fails() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    mr_writes(&mut b).next().unwrap().verb = "patch".into();
    let d = diffs(&a, &b);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(
        (d[0].kind, d[0].a.as_str(), d[0].b.as_str()),
        (Kind::Verb, "#1 update", "#1 patch")
    );
}

#[test]
fn a_changed_event_reason_fails() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    b.writes[5].request_object.as_mut().unwrap()["reason"] = json!("CannotCreateExternalResource");
    let d = diffs(&a, &b);
    let got: Vec<_> = d.iter().map(|d| (d.layer, d.kind, d.key.as_str())).collect();
    assert_eq!(
        got,
        vec![
            (
                Layer::Events,
                Kind::Extra,
                "MyType ns/a Normal CannotCreateExternalResource"
            ),
            (
                Layer::Events,
                Kind::Missing,
                "MyType ns/a Normal CreatedExternalResource"
            ),
        ]
    );
}

#[test]
fn a_failed_chainsaw_step_fails() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    b.report = report("failed");
    let d = diffs(&a, &b);
    assert!(d.iter().all(|d| d.layer == Layer::Chainsaw) && d.len() == 2, "{d:?}");
}

#[test]
fn a_missing_provider_get_status_fails_but_a_scenario_call_does_not_count() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    b.calls[2].forwarded_for = Some("172.18.0.1".into());
    let d = diffs(&a, &b);
    assert_eq!((d.len(), d[0].key.as_str()), (1, "thing a GET"), "{d:?}");
}

#[test]
fn a_changed_runtime_authored_message_fails_but_a_wrapped_client_error_tail_does_not() {
    let with_msg = |m: &str| {
        let mut r = base(100, 43);
        let w = mr_writes(&mut r).nth(1).unwrap();
        w.request_object.as_mut().unwrap()["status"]["conditions"] =
            json!([cond("Synced", "False", "ReconcileError", Some(m))]);
        r
    };
    let go = with_msg(
        "connect failed: cannot apply ProviderConfigUsage: Operation cannot be fulfilled on providerconfigusages \"x\"",
    );
    let rust = with_msg("connect failed: cannot apply ProviderConfigUsage: ApiError: Conflict (409)");
    let o = compare(&go, &rust, &opts());
    assert!(o.equal(), "{}", render(&o, &opts()));

    let other_prefix = with_msg("observe failed: cannot apply ProviderConfigUsage: ApiError: Conflict (409)");
    assert_eq!(diffs(&go, &other_prefix).len(), 1);

    let contract = with_msg("create failed: PUT /things/a: status 500");
    let other_status = with_msg("create failed: PUT /things/a: status 503");
    assert_eq!(diffs(&contract, &other_status).len(), 1);
}

#[test]
fn an_allowance_accepts_the_difference_it_names_and_reports_unused_ones() {
    let (a, mut b) = (base(100, 43), base(100, 43));
    mr_writes(&mut b).next().unwrap().verb = "patch".into();
    let allowances = allow::parse(concat!(
        r#"{"layer":"audit","key":"sample.example.com/mytypes *","kind":"verb","a":"* update","b":"* patch","reason":"kube-rs patches metadata"}"#,
        "\n",
        r#"{"layer":"events","key":"*","kind":"extra","reason":"never needed"}"#,
    ))
    .unwrap();
    let opts = Options { allowances, ..opts() };
    let o = compare(&a, &b, &opts);
    let text = render(&o, &opts);
    assert!(o.equal(), "{text}");
    assert!(text.contains("1: used 1x"), "{text}");
    assert!(text.contains("2: UNUSED"), "{text}");
}

#[test]
fn a_missing_retry_round_on_a_managed_resource_fails() {
    let (mut a, b) = (base(100, 43), base(100, 43));
    let mut round = [a.writes[0].clone(), a.writes[1].clone()];
    round[0].request_received_timestamp = "2026-11-09T22:43:02.5Z".into();
    round[1].request_received_timestamp = "2026-11-09T22:43:02.6Z".into();
    a.writes.splice(2..2, round);
    let d = diffs(&a, &b);
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(d.iter().all(|d| d.kind == Kind::Missing), "{d:?}");
}
