use crossplane_runtime::logging;

#[test]
fn init_installs_the_global_subscriber() {
    assert!(!tracing::dispatcher::has_been_set(), "no subscriber before init");
    logging::init(false);
    assert!(tracing::dispatcher::has_been_set(), "a subscriber after init");
    tracing::info!(controller = "managed/mytype", "reconciled");
}
