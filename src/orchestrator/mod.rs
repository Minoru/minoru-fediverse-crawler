use slog::{error, info, Logger};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{sleep, spawn};
use std::time::Duration;

mod db;
mod instance_checker;
mod time;

static CHECKERS_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn main(logger: Logger) -> anyhow::Result<()> {
    let conn = db::open()?;
    db::init(&conn)?;
    db::reschedule_missed_checks(&conn)?;
    db::disengage_previous_checks(&conn)?;

    loop {
        if db::outstanding_checks_count(&conn)? > 2 * CHECKERS_COUNT.load(Ordering::SeqCst) {
            CHECKERS_COUNT.fetch_add(1, Ordering::SeqCst);
            info!(
                logger,
                "Spawning a new checker for a total of {}",
                CHECKERS_COUNT.load(Ordering::SeqCst)
            );
            println!(
                "{} Spawning a new checker for a total of {}",
                ">".repeat(35),
                CHECKERS_COUNT.load(Ordering::SeqCst)
            );

            let logger = logger.clone();
            spawn(move || {
                if let Err(e) = instance_checker::run(logger.clone()) {
                    error!(logger, "Checker error: {}", e);
                }

                CHECKERS_COUNT.fetch_sub(1, Ordering::SeqCst);
                info!(
                    logger,
                    "Checker finished, {} remain",
                    CHECKERS_COUNT.load(Ordering::SeqCst)
                );
                println!(
                    "{} Checker finished, {} remain",
                    "<".repeat(35),
                    CHECKERS_COUNT.load(Ordering::SeqCst)
                );
            });
        }

        sleep(Duration::from_secs(10));
    }
}
