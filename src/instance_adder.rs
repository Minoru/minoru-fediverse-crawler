use crate::db;
use slog::{info, Logger};
use std::io::{self, BufRead};
use url::Host;

pub fn main(logger: Logger) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    db::init(&mut conn)?;

    let stdin = io::stdin();
    let stdin = stdin.lock();
    let reader = io::BufReader::new(stdin);

    for instance in reader.lines() {
        let instance = instance?;
        info!(logger, "Manually adding {} to the database", instance);
        db::add_instance(&conn, &Host::Domain(instance))?;
        // This is a pretty tight loop that hammers the database, but it's low-priority. Yield to
        // other threads in the hope that they have work to do.
        std::thread::yield_now();
    }

    Ok(())
}
