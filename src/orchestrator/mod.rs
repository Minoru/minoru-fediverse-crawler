use crate::db;
use anyhow::Context;
use slog::{error, Logger};
use url::Host;

mod instance_checker;

const QUEUE_SIZE: usize = 5;
const SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);
// This has to be a large-ish number, so Orchestrator can out-starve any other thread
const SQLITE_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub fn main(logger: Logger) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    db::init(&mut conn)?;
    db::reschedule_missed_checks(&mut conn)?;

    let (tx, rx) = crossbeam_channel::bounded(QUEUE_SIZE);

    spawn_worker(&logger, &rx);

    loop {
        let (instance, check_time) =
            db::pick_next_instance(&conn).context("Orchestrator picking next instance")?;
        let wait = check_time - chrono::offset::Utc::now();
        if wait > chrono::Duration::seconds(30) {
            std::thread::sleep(std::time::Duration::from_secs(30));
            continue;
        }
        if wait > chrono::Duration::zero() {
            std::thread::sleep(wait.to_std()?);
        }
        db::reschedule(&mut conn, &instance).context("Orchestrator rescheduling an instance")?;
        if tx.send_timeout(instance.clone(), SEND_TIMEOUT).is_err() {
            spawn_worker(&logger, &rx);
            tx.send(instance)?;
        }
    }
}

fn spawn_worker(logger: &Logger, rx: &crossbeam_channel::Receiver<Host>) {
    println!("{} Spawning a worker", ">".repeat(35));

    let logger = logger.clone();
    let rx = rx.clone();
    std::thread::spawn(move || {
        if let Err(e) = instance_checker::run(logger.clone(), rx) {
            error!(logger, "Checker error: {}", e);
        }

        println!("{} A worker finished", "<".repeat(35));
    });
}
