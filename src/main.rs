#![deny(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::ok_expect,
    clippy::integer_division,
    clippy::indexing_slicing,
    clippy::integer_arithmetic,
    clippy::panic,
    clippy::match_on_vec_items
)]

use anyhow::{anyhow, bail};
use slog::{error, o, Drain, Logger};
use url::Host;

mod checker;
mod db;
mod domain;
mod instance_adder;
mod ipc;
mod logging_helpers;
mod orchestrator;
mod time;

struct Args {
    add_instances: bool,
    host_to_check: Option<String>,
}

fn parse_args() -> anyhow::Result<Args> {
    use lexopt::prelude::*;

    let mut add_instances = false;
    let mut host_to_check = None;
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("add-instances") => add_instances = true,
            Long("check") => {
                let value = parser.value()?;
                // .into_string() returns Result<String, OsString> , and OsString can't be
                // converted to anyhow::Error. To fix this, we convert the error into String.
                let value = value
                    .into_string()
                    .map_err(|ostr| anyhow!("{}", ostr.to_string_lossy()))?;
                host_to_check = Some(value);
            }
            _ => return Err(arg.unexpected().into()),
        }
    }

    if add_instances && host_to_check.is_some() {
        bail!("--add-instances and --check are mutually exclusive");
    }

    Ok(Args {
        add_instances,
        host_to_check,
    })
}

fn main() -> anyhow::Result<()> {
    let logger = slog::Logger::root(slog_journald::JournaldDrain.ignore_res(), o!());
    logged_main(logger.clone()).map_err(|err| {
        error!(logger, "{:?}", err);
        err
    })
}

fn logged_main(logger: Logger) -> anyhow::Result<()> {
    let args = parse_args()?;
    if args.add_instances {
        instance_adder::main(logger)
    } else {
        match args.host_to_check {
            None => orchestrator::main(logger),
            Some(host) => {
                let host = Host::parse(&host)?;
                checker::main(logger, host)
            }
        }
    }
}
