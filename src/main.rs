use slog::{error, o, Drain, Logger};
use url::Host;

mod checker;
mod ipc;
mod orchestrator;

struct Args {
    host_to_check: Option<String>,
}

fn parse_args() -> Result<Args, lexopt::Error> {
    use lexopt::prelude::*;

    let mut host_to_check = None;
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("check") => {
                host_to_check = Some(parser.value()?.into_string()?);
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Args { host_to_check })
}

fn main() -> anyhow::Result<()> {
    let logger = slog::Logger::root(slog_journald::JournaldDrain.ignore_res(), o!());
    logged_main(logger.clone()).map_err(|err| {
        error!(logger, "{}", err);
        err
    })
}

fn logged_main(logger: Logger) -> anyhow::Result<()> {
    let args = parse_args()?;
    match args.host_to_check {
        None => orchestrator::main(logger),
        Some(host) => {
            let host = Host::parse(&host)?;
            checker::main(logger, host)
        }
    }
}
