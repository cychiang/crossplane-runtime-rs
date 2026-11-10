//! Go kingpin's command-line behaviour on top of clap, for a provider's `main`.
//!
//! A provider declares its flags with `#[derive(clap::Parser)]` and calls [`parse`] instead of
//! `Parser::parse`. What kingpin does that clap does not:
//!
//! - Every flag without an explicit `env` reads `<BINARY>_<FLAG>`, derived from the binary name
//!   as kingpin's `DefaultEnvars` does. An env var set to the empty string counts as unset.
//! - A bool flag also takes a `--no-<flag>` form, and the last one given wins. The provider
//!   declares each twin as a hidden field and combines the pair, because clap hands a derive
//!   struct only the fields it declares:
//!
//!   ```text
//!   #[arg(long, value_parser = cli::go_bool, overrides_with = "no_debug")]
//!   debug: bool,
//!   #[arg(long, hide = true, overrides_with = "debug")]
//!   no_debug: bool,
//!   // debug is on when `debug && !no_debug`
//!   ```
//! - A bool's env var is read with Go's `strconv.ParseBool`, so give the flag
//!   `value_parser = cli::go_bool`. A bool flag rejects an inline `--flag=false`.

use std::ffi::OsString;
use std::path::Path;

use clap::{CommandFactory, Parser};

/// Parses `args`, whose first item is the binary path, the way kingpin's `Parse` would. Take the
/// result's error to [`clap::Error::exit`] for clap's usual messages and exit codes.
pub fn parse<A: Parser>(args: impl IntoIterator<Item = OsString>) -> Result<A, clap::Error> {
    let mut args = args.into_iter().peekable();
    let bin = args
        .peek()
        .and_then(|a| Path::new(a).file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "provider".to_owned());
    A::from_arg_matches(&command::<A>(&bin).try_get_matches_from(args)?)
}

/// Go's `kingpin.New(filepath.Base(os.Args[0]), ...).DefaultEnvars()`: every flag without an
/// explicit env var reads `<BINARY>_<FLAG>`. kingpin ignores an env var set to the empty string,
/// where clap would parse it, so such a variable is not attached. A hidden flag, such as a
/// `--no-<flag>` twin, gets no derived env var.
pub fn command<A: CommandFactory>(bin: &str) -> clap::Command {
    A::command().name(bin.to_owned()).mut_args(|a| {
        let name: Option<OsString> = match (a.get_env(), a.is_hide_set(), a.get_long()) {
            (Some(env), _, _) => Some(env.to_owned()),
            (None, false, Some(long)) => Some(envar(&format!("{bin}_{long}")).into()),
            _ => None,
        };
        match name {
            Some(n) if std::env::var_os(&n).is_none_or(|v| !v.is_empty()) => a.env(n),
            _ => a.env(None),
        }
    })
}

/// Go `strconv.ParseBool`, which kingpin applies to a bool flag's env var.
pub fn go_bool(s: &str) -> Result<bool, String> {
    match s {
        "1" | "t" | "T" | "TRUE" | "true" | "True" => Ok(true),
        "0" | "f" | "F" | "FALSE" | "false" | "False" => Ok(false),
        _ => Err(format!("strconv.ParseBool: parsing {s:?}: invalid syntax")),
    }
}

/// Go kingpin's `envarTransform`: upper case, each run of characters outside `[A-Za-z0-9_]`
/// becomes one `_`.
fn envar(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c.to_ascii_uppercase());
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    out
}

#[cfg(test)]
#[path = "cli_test.rs"]
mod tests;
