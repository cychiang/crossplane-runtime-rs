//! Go `cmd/provider/main.go`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use crossplane_core::duration;
use crossplane_runtime::cli::{self, go_bool};
use crossplane_runtime::controller::{self, ChangeLogOptions, MetricOptions};
use crossplane_runtime::feature::{self, Features};
use crossplane_runtime::gate::Gate;
use crossplane_runtime::manager::{self, LeaderElection, Manager, ManagerOptions, RestConfig};
use crossplane_runtime::reconciler::{customresourcesgate, managed};
use crossplane_runtime::{logging, ratelimiter, statemetrics};
use provider_sample::provider;

/// Go `internal/version/version.go`.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The flags of Go's kingpin app, flag for flag. Each kingpin bool is `--[no-]<name>`, so each
/// bool here has a hidden `--no-<name>` twin and the last one given wins.
#[derive(Parser, Debug)]
#[command(
    about = "provider-baseline support for Crossplane.",
    after_help = "Each boolean flag also takes a --no-<flag> form; the last one given wins."
)]
struct Args {
    /// Run with debug logging.
    #[arg(short = 'd', long, value_parser = go_bool, overrides_with = "no_debug")]
    debug: bool,
    #[arg(long, hide = true, overrides_with = "debug")]
    no_debug: bool,
    /// Use leader election for the controller manager.
    #[arg(short = 'l', long, env = "LEADER_ELECTION", value_parser = go_bool, overrides_with = "no_leader_election")]
    leader_election: bool,
    #[arg(long, hide = true, overrides_with = "leader_election")]
    no_leader_election: bool,
    /// How often all resources will be double-checked for drift from the desired state.
    #[arg(short = 's', long, default_value = "1h", value_parser = duration::parse)]
    sync: Duration,
    /// How often individual resources will be checked for drift from the desired state.
    #[arg(long, default_value = "1m", value_parser = duration::parse)]
    poll: Duration,
    /// State metric recording interval.
    #[arg(long, default_value = "5s", value_parser = duration::parse)]
    poll_state_metric: Duration,
    /// The global maximum rate per second at which resources may be checked for drift.
    #[arg(long, default_value_t = 10)]
    max_reconcile_rate: u32,
    /// Enable support for Management Policies.
    #[arg(
        long,
        env = "ENABLE_MANAGEMENT_POLICIES",
        default_value_t = true,
        value_parser = go_bool,
        overrides_with = "no_enable_management_policies"
    )]
    enable_management_policies: bool,
    #[arg(long, hide = true, overrides_with = "enable_management_policies")]
    no_enable_management_policies: bool,
    /// Enable support for capturing change logs during reconciliation.
    #[arg(long, env = "ENABLE_CHANGE_LOGS", value_parser = go_bool, overrides_with = "no_enable_changelogs")]
    enable_changelogs: bool,
    #[arg(long, hide = true, overrides_with = "enable_changelogs")]
    no_enable_changelogs: bool,
    /// Path for changelogs socket (if enabled).
    #[arg(
        long,
        env = "CHANGELOGS_SOCKET_PATH",
        default_value = "/var/run/changelogs/changelogs.sock"
    )]
    changelogs_socket_path: String,
    #[command(flatten)]
    provider: provider::Flags,
}

impl Args {
    fn debug(&self) -> bool {
        self.debug && !self.no_debug
    }

    fn leader_election(&self) -> bool {
        self.leader_election && !self.no_leader_election
    }

    fn enable_management_policies(&self) -> bool {
        self.enable_management_policies && !self.no_enable_management_policies
    }

    fn enable_changelogs(&self) -> bool {
        self.enable_changelogs && !self.no_enable_changelogs
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = cli::parse::<Args>(std::env::args_os()).unwrap_or_else(|e| e.exit());
    logging::init(args.debug());

    let cfg = RestConfig::infer("provider-baseline", VERSION)
        .await
        .context("Cannot get API server rest config")?;
    let mgr = Manager::new(
        ratelimiter::limit_rest_config(cfg, args.max_reconcile_rate),
        ManagerOptions {
            sync_period: args.sync,
            leader_election: args.leader_election().then(|| {
                LeaderElection::new("crossplane-leader-election-provider-baseline")
                    .lease_duration(Duration::from_secs(60))
                    .renew_deadline(Duration::from_secs(50))
            }),
            ..Default::default()
        },
    )
    .context("Cannot create controller manager")?;

    let metric_recorder = managed::MrMetricRecorder::new();
    let state_metrics = statemetrics::MrStateMetrics::new();
    mgr.register_metrics(Box::new(metric_recorder.clone()))?;
    mgr.register_metrics(Box::new(state_metrics.clone()))?;

    let mut o = controller::Options {
        max_concurrent_reconciles: args.max_reconcile_rate as usize,
        poll_interval: args.poll,
        global_rate_limiter: ratelimiter::new_global(args.max_reconcile_rate),
        features: Features::default(),
        gate: Arc::new(Gate::default()),
        metric_options: Some(MetricOptions {
            poll_state_metric_interval: args.poll_state_metric,
            mr_metrics: Arc::new(metric_recorder),
            mr_state_metrics: Some(state_metrics),
        }),
        change_log_options: None,
    };
    if args.enable_management_policies() {
        o.features.enable(feature::ENABLE_BETA_MANAGEMENT_POLICIES);
        tracing::info!(flag = %feature::ENABLE_BETA_MANAGEMENT_POLICIES, "Beta feature enabled");
    }
    if args.enable_changelogs() {
        o.features.enable(feature::ENABLE_ALPHA_CHANGE_LOGS);
        tracing::info!(flag = %feature::ENABLE_ALPHA_CHANGE_LOGS, "Alpha feature enabled");
        let cl = managed::GrpcChangeLogger::connect_unix(
            &args.changelogs_socket_path,
            format!("provider-baseline:{VERSION}"),
        );
        o.change_log_options = Some(ChangeLogOptions {
            change_logger: Arc::new(cl),
        });
    }
    provider::configure(&mut o, &args.provider).context("Cannot configure provider options")?;

    customresourcesgate::setup(&mgr, &o).context("Cannot setup CRD gate controller")?;
    provider_sample::controller::setup(&mgr, &o).context("Cannot setup provider-baseline controllers")?;
    mgr.start(manager::setup_signal_handler())
        .await
        .context("Cannot start controller manager")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn parse(argv: &[&str]) -> Args {
        cli::parse(argv.iter().map(OsString::from)).expect("flags parse")
    }

    #[test]
    fn defaults_match_the_go_baseline() {
        let a = parse(&["provider-sample"]);
        assert!(!a.debug() && !a.leader_election() && !a.enable_changelogs());
        assert!(a.enable_management_policies(), "management policies default on");
        assert_eq!(a.sync, Duration::from_secs(3600));
        assert_eq!(a.poll, Duration::from_secs(60));
        assert_eq!(a.max_reconcile_rate, 10);
    }

    #[test]
    fn bools_take_a_no_prefix_and_the_last_one_wins() {
        assert!(!parse(&["provider-sample", "--no-enable-management-policies"]).enable_management_policies());
        assert!(parse(&["provider-sample", "--no-debug", "--debug"]).debug());
        assert!(!parse(&["provider-sample", "-d", "--no-debug"]).debug());
    }
}
