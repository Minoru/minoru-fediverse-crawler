use anyhow::Context;
use slog::{o, Logger};

mod checker_handle;
mod db;
mod time;

use checker_handle::CheckerHandle;

pub fn main(logger: Logger) -> anyhow::Result<()> {
    let conn = db::open()?;
    db::init(&conn)?;
    db::reschedule_missed_checks(&conn)?;
    db::disengage_previous_checks(&conn)?;

    loop {
        if let Some(instance) =
            db::pick_next_instance(&conn).context("Picking an instance to check")?
        {
            println!("Checking {}", instance);

            let logger = logger.new(o!("host" => instance.to_string()));
            CheckerHandle::new(logger, instance)?.run()?;
        } else {
            println!("Waiting for some checks to come due...");
            std::thread::sleep(std::time::Duration::new(1, 0));
        }
    }
}
