use crate::orchestrator::time;
use anyhow::{anyhow, Context};
use rusqlite::{params, Connection};
use url::Host;

/// Instance states which are stored in the DB.
#[derive(PartialEq, Eq, Debug)]
pub enum InstanceState {
    Discovered = 0,
    Alive = 1,
    Dying = 2,
    Dead = 3,
    Moving = 4,
    Moved = 5,
}

impl InstanceState {
    pub fn from(i: u8) -> Option<Self> {
        match i {
            0 => Some(Self::Discovered),
            1 => Some(Self::Alive),
            2 => Some(Self::Dying),
            3 => Some(Self::Dead),
            4 => Some(Self::Moving),
            5 => Some(Self::Moved),
            _ => None,
        }
    }
}

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
    // These states are mapped to `InstanceState`.
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

pub fn mark_alive(_conn: &Connection, _instance: &Host) -> anyhow::Result<()> {
    Ok(())
}

pub fn mark_dead(_conn: &Connection, _instance: &Host) -> anyhow::Result<()> {
    Ok(())
}

pub fn mark_moved(_conn: &Connection, _instance: &Host, _to: &Host) -> anyhow::Result<()> {
    Ok(())
}

pub fn add_instance(_conn: &Connection, _instance: &Host) -> anyhow::Result<()> {
    Ok(())
}

/// Reschedule the instance according to its state.
///
/// This is meant to be used when the checker fails. In that case, we want to reschedule the
/// instance sometime in the future, so we keep tracking it. We do this according to the current
/// state of the instance, preserving the frequency of the checks.
pub fn reschedule(conn: &mut Connection, instance: &Host) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    let state = tx.query_row(
        "SELECT state FROM instances WHERE hostname = ?1",
        params![instance.to_string()],
        |row| row.get(0),
    )?;
    let state = InstanceState::from(state)
        .ok_or_else(|| anyhow!("Got invalid instance state from the DB: {}", state))?;

    let next_check_datetime = match state {
        InstanceState::Discovered => time::rand_datetime_daily()?,
        InstanceState::Alive => time::rand_datetime_daily()?,
        InstanceState::Dying => time::rand_datetime_daily()?,
        InstanceState::Dead => time::rand_datetime_weekly()?,
        InstanceState::Moving => time::rand_datetime_daily()?,
        InstanceState::Moved => time::rand_datetime_weekly()?,
    };

    tx.execute(
        "UPDATE instances SET next_check_datetime = ?1 WHERE hostname = ?2",
        params![next_check_datetime, instance.to_string()],
    )?;

    Ok(tx.commit()?)
}
