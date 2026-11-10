use crossplane_core::errors::Error;
use crossplane_core::v2::Condition;
use crossplane_test::fake::ModernManaged;
use crossplane_test::{MockClient, new_mock_list_err_fn, new_mock_list_fn};
use prometheus::Encoder;

use super::*;

fn managed(conditions: Vec<Condition>) -> ModernManaged {
    let mut mr = ModernManaged::default();
    mr.set_conditions(conditions);
    mr
}

fn scrape(metrics: &MrStateMetrics) -> String {
    let registry = prometheus::Registry::new();
    registry.register(Box::new(metrics.clone())).unwrap();
    let mut out = Vec::new();
    prometheus::TextEncoder::new()
        .encode(&registry.gather(), &mut out)
        .unwrap();
    String::from_utf8(out).unwrap()
}

#[tokio::test]
async fn record_counts_existing_ready_and_synced_resources_per_gvk() {
    let client = MockClient {
        list: Some(new_mock_list_fn(|| {
            vec![
                managed(vec![Condition::available(), Condition::reconcile_success()]),
                managed(vec![Condition::unavailable(), Condition::reconcile_success()]),
                managed(vec![Condition::creating()]),
            ]
        })),
        ..Default::default()
    };
    let metrics = MrStateMetrics::new();
    let recorder = MrStateRecorder::<ModernManaged>::new(Arc::new(client), metrics.clone(), Duration::from_secs(1));
    recorder.record().await.unwrap();
    let text = scrape(&metrics);
    let label = r#"gvk="fake.crossplane.io/v1, Kind=ModernManaged""#;
    for line in [
        "# HELP crossplane_managed_resource_exists The number of managed resources that exist".to_owned(),
        "# TYPE crossplane_managed_resource_exists gauge".to_owned(),
        format!("crossplane_managed_resource_exists{{{label}}} 3"),
        "# HELP crossplane_managed_resource_ready The number of managed resources in Ready=True state".to_owned(),
        format!("crossplane_managed_resource_ready{{{label}}} 1"),
        "# HELP crossplane_managed_resource_synced The number of managed resources in Synced=True state".to_owned(),
        format!("crossplane_managed_resource_synced{{{label}}} 2"),
    ] {
        assert!(text.lines().any(|l| l == line), "missing {line:?} in\n{text}");
    }
}

#[tokio::test]
async fn a_failed_list_is_wrapped() {
    let client = MockClient {
        list: Some(new_mock_list_err_fn(Error::new("boom"))),
        ..Default::default()
    };
    let recorder =
        MrStateRecorder::<ModernManaged>::new(Arc::new(client), MrStateMetrics::new(), Duration::from_secs(1));
    let err = recorder.record().await.unwrap_err();
    assert_eq!(err.to_string(), "failed to list managed resources: boom");
}

#[tokio::test(start_paused = true)]
async fn start_records_on_each_tick_and_stops_on_its_first_error() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let client = MockClient {
        list: Some(Box::new(move |_, _| {
            let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < 2 { Ok(vec![]) } else { Err(Error::new("gone")) }
        })),
        ..Default::default()
    };
    let recorder =
        MrStateRecorder::<ModernManaged>::new(Arc::new(client), MrStateMetrics::new(), Duration::from_secs(10));
    let begun = tokio::time::Instant::now();
    let err = Box::new(recorder).start(CancellationToken::new()).await.unwrap_err();
    assert_eq!(err.to_string(), "failed to list managed resources: gone");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(
        begun.elapsed(),
        Duration::from_secs(30),
        "first record one interval after start, as Go's ticker"
    );
}
