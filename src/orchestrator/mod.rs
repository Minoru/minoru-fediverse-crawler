use slog::Logger;

mod db;
mod instance_checker;
mod time;

pub fn main(logger: Logger) -> anyhow::Result<()> {
    init()?;
    instance_checker::run(logger)
}

fn init() -> anyhow::Result<()> {
    let conn = db::open()?;
    db::init(&conn)?;
    db::reschedule_missed_checks(&conn)?;
    db::disengage_previous_checks(&conn)?;
    Ok(())
}
