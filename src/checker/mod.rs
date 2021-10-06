use slog::{info, o, Drain, Logger};
use tokio::runtime::Runtime;

pub fn main(host: String) -> anyhow::Result<()> {
    let logger = slog::Logger::root(slog_journald::JournaldDrain.ignore_res(), o!());

    let rt = Runtime::new()?;
    info!(logger, "Started Tokio runtime");
    rt.block_on(async_main(logger.new(o!("host" => host.clone())), &host))
}

async fn async_main(logger: Logger, _host: &str) -> anyhow::Result<()> {
    info!(logger, "Started the checker");
    Ok(())
}
