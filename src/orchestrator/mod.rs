use crate::{db, with_loc};
use anyhow::Context;
use slog::{error, o, Logger};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

mod instance_checker;

/// This has to be a large-ish number, so Orchestrator can out-starve any other thread
const SQLITE_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Minimum amount of checkers that are always present (waiting for work or performing it).
const CONSTANT_WORKERS: usize = 1;
/// Maximum number of checkers that can run.
// 10 million checks —which is 10 times more than our design goal— over 24 hours means 116 checks
// per second. Let's round that up to the nearest power of two, just because.
const MAX_WORKERS: usize = 128;
/// How long a worker will wait for work before shutting down its thread.
const MAX_WORKER_IDLE_TIME: std::time::Duration = std::time::Duration::from_secs(30);

pub fn main(logger: Logger) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    db::init(&mut conn)?;
    db::reschedule_missed_checks(&mut conn)?;

    let pool = rusty_pool::ThreadPool::new(CONSTANT_WORKERS, MAX_WORKERS, MAX_WORKER_IDLE_TIME);

    let terminate = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, terminate.clone())
        .context(with_loc!("Setting up a SIGINT hook"))?;

    let mut iteration = || -> anyhow::Result<()> {
        let (instance, check_time) = db::pick_next_instance(&conn)
            .context(with_loc!("Orchestrator picking next instance"))?;
        let wait = check_time - chrono::offset::Utc::now();
        if wait > chrono::Duration::seconds(30) {
            std::thread::sleep(std::time::Duration::from_secs(30));
            return Ok(());
        }
        if wait > chrono::Duration::zero() {
            std::thread::sleep(wait.to_std()?);
        }
        db::reschedule(&mut conn, &instance)
            .context(with_loc!("Orchestrator rescheduling an instance"))?;

        let logger = logger.new(o!("host" => instance.to_string()));
        pool.execute(move || {
            if let Err(e) = instance_checker::run(logger.clone(), instance) {
                error!(logger, "Checker error: {:?}", e);
            }
        });

        Ok(())
    };

    loop {
        db::on_sqlite_busy_retry_indefinitely(&mut iteration)?;
        if terminate.load(Ordering::Relaxed) {
            println!("Shutting down gracefully...");
            break;
        }
    }

    pool.join();
    Ok(())
}
