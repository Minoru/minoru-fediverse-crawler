use crate::orchestrator::time;
use anyhow::{bail, Context};
use rusqlite::{params, Connection};
use slog::{error, Logger};
use url::Host;

pub fn open() -> anyhow::Result<Connection> {
    Connection::open("fediverse.observer.db").context("Failed to initialize the database")
}

pub fn init(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS states(
            id INTEGER PRIMARY KEY NOT NULL,
            state TEXT UNIQUE NOT NULL
        )",
        [],
    )?;
    conn.execute(
        r#"INSERT OR IGNORE INTO states (id, state)
        VALUES
            (0, "discovered"),
            (1, "alive"),
            (2, "dying"),
            (3, "dead"),
            (4, "moving"),
            (5, "moved")"#,
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS instances(
            id INTEGER PRIMARY KEY NOT NULL,
            hostname TEXT UNIQUE NOT NULL,
            state REFERENCES states(id) NOT NULL DEFAULT 0,
            next_check_datetime INTEGER DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;
    conn.execute(
        r#"INSERT OR IGNORE INTO instances(hostname) VALUES ("mastodon.social")"#,
        [],
    )?;

    Ok(())
}

pub fn reschedule_missed_checks(conn: &Connection) -> anyhow::Result<()> {
    let mut statement =
        conn.prepare("SELECT id FROM instances WHERE next_check_datetime < CURRENT_TIMESTAMP")?;
    let mut ids = statement.query([])?;
    while let Some(row) = ids.next()? {
        let instance_id: u64 = row.get(0)?;
        conn.execute(
            "UPDATE instances SET next_check_datetime = ?1 WHERE id = ?2",
            params![time::rand_datetime_daily()?, instance_id],
        )?;
    }
    Ok(())
}

/// Reschedule the instance according to its state.
///
/// This is meant to be used when the checker fails. In that case, we want to reschedule the
/// instance sometime in the future, so we keep tracking it. We do this according to the current
/// state of the instance, preserving the frequency of the checks.
pub fn reschedule(logger: &Logger, conn: &mut Connection, instance: &Host) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    let state: u64 = tx.query_row(
        "SELECT state FROM instances WHERE hostname = ?1",
        params![instance.to_string()],
        |row| row.get(0),
    )?;

    let next_check_datetime = match state {
        // TODO: replace those constants with an enum
        0 => {
            // discovered
            time::rand_datetime_daily()?
        }
        1 => {
            // alive
            time::rand_datetime_daily()?
        }
        2 => {
            // dying
            time::rand_datetime_daily()?
        }
        3 => {
            // dead
            time::rand_datetime_weekly()?
        }
        4 => {
            // moving
            time::rand_datetime_daily()?
        }
        5 => {
            // moved
            time::rand_datetime_weekly()?
        }
        _ => {
            let msg = format!(
                "Instance {} has invalid state in the DB: {}",
                instance, state
            );
            error!(logger, "{}", &msg);
            bail!(msg)
        }
    };

    tx.execute(
        "UPDATE instances SET next_check_datetime = ?1 WHERE hostname = ?2",
        params![next_check_datetime, instance.to_string()],
    )?;

    Ok(tx.commit()?)
}
