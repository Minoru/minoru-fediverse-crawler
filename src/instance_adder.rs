use crate::{db, domain::Domain};
use slog::{Logger, error, info};
use std::io::{self, BufRead};

pub fn main(logger: Logger) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    db::init(&mut conn)?;

    let stdin = io::stdin();
    let stdin = stdin.lock();
    let reader = io::BufReader::new(stdin);

    for domain in reader.lines() {
        let domain = domain?;
        let domain = match Domain::from_str(&domain) {
            Err(e) => {
                let msg =
                    format!("Couldn't manually add {domain}, it's not a valid domain name: {e}");
                error!(logger, "{}", msg);
                println!("{msg}");
                continue;
            }

            Ok(domain) => domain,
        };
        match db::on_sqlite_busy_retry_indefinitely(&mut || db::add_instance(&conn, &domain)) {
            Err(e) => {
                let msg = format!("Failed to add {domain} to the database: {e}");
                error!(logger, "{}", msg);
                println!("{msg}");
            }

            Ok(_) => {
                let msg = format!("Manually added {domain} to the database");
                info!(logger, "{}", msg);
            }
        }
        // This is a pretty tight loop that hammers the database, but it's low-priority. Yield to
        // other threads in the hope that they have work to do.
        std::thread::yield_now();
    }

    Ok(())
}
