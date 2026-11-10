use std::time::Duration;

use clap::Parser;
use crossplane_core::duration;

use super::*;

/// The shape a provider's `Args` has: a bool with a `--no-` twin, a bool that defaults on, an
/// explicit env var and a derived one.
#[derive(Parser, Debug)]
struct Args {
    #[arg(short = 'd', long, value_parser = go_bool, overrides_with = "no_debug")]
    debug: bool,
    #[arg(long, hide = true, overrides_with = "debug")]
    no_debug: bool,
    #[arg(
        long,
        env = "ENABLE_THING",
        default_value_t = true,
        value_parser = go_bool,
        overrides_with = "no_enable_thing"
    )]
    enable_thing: bool,
    #[arg(long, hide = true, overrides_with = "enable_thing")]
    no_enable_thing: bool,
    #[arg(short = 's', long, default_value = "1h", value_parser = duration::parse)]
    sync: Duration,
}

impl Args {
    fn debug(&self) -> bool {
        self.debug && !self.no_debug
    }

    fn enable_thing(&self) -> bool {
        self.enable_thing && !self.no_enable_thing
    }
}

fn parse_args(argv: &[&str]) -> Result<Args, clap::Error> {
    parse(argv.iter().map(OsString::from))
}

fn args(argv: &[&str]) -> Args {
    parse_args(argv).expect("flags parse")
}

#[test]
fn defaults_apply_without_flags() {
    let a = args(&["provider"]);
    assert!(!a.debug(), "a bool defaults off");
    assert!(a.enable_thing(), "a bool can default on");
    assert_eq!(a.sync, Duration::from_secs(3600));
}

#[test]
fn bools_take_a_no_prefix_and_the_last_one_wins() {
    assert!(!args(&["provider", "--no-enable-thing"]).enable_thing());
    assert!(args(&["provider", "--no-debug", "--debug"]).debug());
    assert!(!args(&["provider", "-d", "--no-debug"]).debug());
    assert!(args(&["provider", "--no-enable-thing", "--enable-thing"]).enable_thing());
}

#[test]
fn bools_reject_an_inline_value_like_kingpin() {
    assert!(parse_args(&["provider", "--enable-thing=false"]).is_err());
}

#[test]
fn durations_parse_like_go() {
    assert_eq!(args(&["provider", "-s", "1h30m"]).sync, Duration::from_secs(5400));
}

#[test]
fn default_envars_derive_from_the_binary_name() {
    let envs: Vec<_> = command::<Args>("crossplane-my-provider")
        .get_arguments()
        .filter_map(|a| a.get_env().map(|e| e.to_string_lossy().into_owned()))
        .collect();
    assert_eq!(
        envs,
        [
            "CROSSPLANE_MY_PROVIDER_DEBUG",
            "ENABLE_THING",
            "CROSSPLANE_MY_PROVIDER_SYNC"
        ]
    );
}

#[test]
fn a_binary_path_is_accepted_as_argv0() {
    assert!(parse_args(&["/usr/local/bin/provider", "--debug"]).is_ok());
}

#[test]
fn bool_env_values_parse_like_go() {
    for v in ["1", "t", "T", "TRUE", "true", "True"] {
        assert_eq!(go_bool(v), Ok(true), "{v}");
    }
    for v in ["0", "f", "F", "FALSE", "false", "False"] {
        assert_eq!(go_bool(v), Ok(false), "{v}");
    }
    assert!(go_bool("yes").is_err(), "Go rejects yes");
}

#[test]
fn envar_collapses_runs_like_kingpin() {
    assert_eq!(envar("a.-b_c"), "A_B_C");
    assert_eq!(envar("a_-b"), "A__B");
}
