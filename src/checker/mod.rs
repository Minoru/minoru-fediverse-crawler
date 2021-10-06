use slog::{info, o, Logger};
use sloggers::{
    terminal::{Destination, TerminalLoggerBuilder},
    types::Severity,
    Build,
};
use tokio::runtime::Runtime;

pub fn main(host: String) -> anyhow::Result<()> {
    let mut builder = TerminalLoggerBuilder::new();
    builder.level(Severity::Info);
    builder.destination(Destination::Stderr);

    let logger = builder.build()?;

    let rt = Runtime::new()?;
    info!(logger, "Started Tokio runtime");
    rt.block_on(async_main(logger.new(o!("host" => host.clone())), &host))
}

async fn async_main(logger: Logger, _host: &str) -> anyhow::Result<()> {
    info!(logger, "Started the checker");
    Ok(())
}
