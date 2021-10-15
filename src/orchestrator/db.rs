use crate::orchestrator::time;
use anyhow::Context;
use rusqlite::{params, Connection};

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
