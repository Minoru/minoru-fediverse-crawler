use crate::time;
use anyhow::{anyhow, Context};
use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use slog::{info, Logger};
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
    let conn =
        Connection::open("fediverse.observer.db").context("Failed to initialize the database")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    Ok(conn)
}

pub fn init(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS states(
            id INTEGER PRIMARY KEY NOT NULL,
            state TEXT UNIQUE NOT NULL
        )",
        [],
    )
    .context("Creating table states")?;
    // These states are mapped to `InstanceState`.
    tx.execute(
        r#"INSERT OR IGNORE INTO states (id, state)
        VALUES
            (0, "discovered"),
            (1, "alive"),
            (2, "dying"),
            (3, "dead"),
            (4, "moving"),
            (5, "moved")"#,
        [],
    )
    .context("Filling table states")?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS instances(
            id INTEGER PRIMARY KEY NOT NULL,
            hostname TEXT UNIQUE NOT NULL,
            state REFERENCES states(id) NOT NULL DEFAULT 0,
            last_check_datetime INTEGER DEFAULT NULL,
            next_check_datetime INTEGER DEFAULT (strftime('%s', CURRENT_TIMESTAMP)),
            check_started INTEGER DEFAULT NULL
        )",
        [],
    )
    .context("Creating table instances")?;
    tx.execute(
        r#"INSERT OR IGNORE
        INTO instances(hostname)
        VALUES ("mastodon.social")"#,
        [],
    )
    .context("Adding mastodon.social to the instances table")?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS instances_next_check_datetime_idx
        ON instances(next_check_datetime)",
        [],
    )
    .context("Creating index on instances(next_check_datetime)")?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS alive_state_data(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE
        )",
        [],
    )
    .context("Creating table alive_state_data")?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS dying_state_data(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE,
            dying_since INTEGER NOT NULL,
            failed_checks_count INTEGER NOT NULL DEFAULT 1
        )",
        [],
    )
    .context("Creating table dying_state_data")?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS moving_state_data(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE,
            moving_since INTEGER NOT NULL,
            redirects_count INTEGER NOT NULL DEFAULT 1,
            moving_to REFERENCES instances(id) NOT NULL
        )",
        [],
    )
    .context("Creating table moving_state_data")?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS moved_state_data(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE,
            moved_to REFERENCES instances(id) NOT NULL
        )",
        [],
    )
    .context("Creating table moved_state_data")?;

    Ok(tx.commit()?)
}

/// If the server was stopped mid-way, some entries in the database can still be marked as "being
/// checked". Remove those marks.
pub fn disengage_previous_checks(conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE instances
        SET check_started = NULL
        WHERE check_started IS NOT NULL",
        [],
    )?;
    Ok(())
}

pub fn reschedule_missed_checks(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    {
        let mut statement = tx.prepare(
            "SELECT id
            FROM instances
            WHERE next_check_datetime < strftime('%s', CURRENT_TIMESTAMP)",
        )?;
        let mut ids = statement.query([])?;
        while let Some(row) = ids.next()? {
            let instance_id: u64 = row.get(0)?;
            tx.execute(
                "UPDATE instances SET next_check_datetime = ?1 WHERE id = ?2",
                params![time::rand_datetime_today()?.timestamp(), instance_id],
            )?;
        }
    }

    Ok(tx.commit()?)
}

pub fn mark_alive(conn: &mut Connection, instance: &Host) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    let instance_id: u64 = tx.query_row(
        "SELECT id FROM instances WHERE hostname = ?1",
        params![instance.to_string()],
        |row| row.get(0),
    )?;

    // Delete any previous state data related to this instance
    tx.execute(
        "DELETE FROM dying_state_data
        WHERE instance = ?1",
        params![instance_id],
    )?;
    tx.execute(
        "DELETE FROM moving_state_data
        WHERE instance = ?1",
        params![instance_id],
    )?;
    tx.execute(
        "DELETE FROM moved_state_data
        WHERE instance = ?1",
        params![instance_id],
    )?;

    // Create/update alive state data
    tx.execute(
        "INSERT OR REPLACE
        INTO alive_state_data(instance)
        VALUES (?1)",
        params![instance_id],
    )?;

    // Mark the instance alive and schedule the next check
    tx.execute(
        "UPDATE instances
        SET state = ?1,
            last_check_datetime = strftime('%s', CURRENT_TIMESTAMP),
            next_check_datetime = ?2
        WHERE id = ?3",
        params![
            InstanceState::Alive as u8,
            time::rand_datetime_daily()?.timestamp(),
            instance_id
        ],
    )?;

    Ok(tx.commit()?)
}

pub fn mark_dead(conn: &mut Connection, instance: &Host) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    let instance_id: u64 = tx.query_row(
        "SELECT id FROM instances WHERE hostname = ?1",
        params![instance.to_string()],
        |row| row.get(0),
    )?;
    let now = Utc::now();

    match get_instance_state(&tx, instance)? {
        InstanceState::Discovered
        | InstanceState::Alive
        | InstanceState::Moving
        | InstanceState::Moved => {
            tx.execute(
                "DELETE FROM alive_state_data
                WHERE instance = ?1",
                params![instance_id],
            )?;
            tx.execute(
                "DELETE FROM moving_state_data
                WHERE instance = ?1",
                params![instance_id],
            )?;
            tx.execute(
                "DELETE FROM moved_state_data
                WHERE instance = ?1",
                params![instance_id],
            )?;

            tx.execute(
                "INSERT
                INTO dying_state_data(instance, dying_since)
                VALUES (?1, ?2)",
                params![instance_id, now],
            )?;
            tx.execute(
                "UPDATE instances
                SET state = ?1,
                    last_check_datetime = ?2,
                    next_check_datetime = ?3
                WHERE id = ?4",
                params![
                    InstanceState::Dying as u8,
                    now.timestamp(),
                    time::rand_datetime_daily()?.timestamp(),
                    instance_id
                ],
            )?;
        }
        InstanceState::Dying => {
            tx.execute(
                "UPDATE dying_state_data
                SET failed_checks_count = failed_checks_count + 1
                WHERE instance = ?1",
                params![instance_id],
            )?;
            let (checks_count, since): (u64, chrono::DateTime<Utc>) = tx.query_row(
                "SELECT failed_checks_count, dying_since
                FROM dying_state_data
                WHERE instance = ?1",
                params![instance_id],
                |row| match (row.get(0), row.get(1)) {
                    (Ok(a), Ok(b)) => Ok((a, b)),
                    (Err(a), _) => Err(a),
                    (_, Err(b)) => Err(b),
                },
            )?;
            let week_ago = now
                .checked_sub_signed(Duration::weeks(1))
                .ok_or_else(|| anyhow!("Couldn't subtract a week from today's datetime"))?;
            if checks_count > 7 && since > week_ago {
                tx.execute(
                    "DELETE FROM dying_state_data
                    WHERE instance = ?1",
                    params![instance_id],
                )?;
                tx.execute(
                    "UPDATE instances
                    SET state = ?1,
                        last_check_datetime = ?2,
                        next_check_datetime = ?3
                    WHERE id = ?4",
                    params![
                        InstanceState::Dead as u8,
                        now.timestamp(),
                        time::rand_datetime_weekly()?.timestamp(),
                        instance_id
                    ],
                )?;
            } else {
                tx.execute(
                    "UPDATE instances
                    SET last_check_datetime = ?1,
                        next_check_datetime = ?2
                    WHERE id = ?3",
                    params![
                        now.timestamp(),
                        time::rand_datetime_daily()?.timestamp(),
                        instance_id
                    ],
                )?;
            }
        }
        InstanceState::Dead => {
            tx.execute(
                "UPDATE instances
                SET last_check_datetime = ?1,
                    next_check_datetime = ?2
                WHERE id = ?3",
                params![
                    now.timestamp(),
                    time::rand_datetime_weekly()?.timestamp(),
                    instance_id
                ],
            )?;
        }
    }

    Ok(tx.commit()?)
}

pub fn mark_moved(conn: &mut Connection, instance: &Host, to: &Host) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    let instance_id: u64 = tx.query_row(
        "SELECT id FROM instances WHERE hostname = ?1",
        params![instance.to_string()],
        |row| row.get(0),
    )?;
    let now = Utc::now();

    match get_instance_state(&tx, instance)? {
        InstanceState::Discovered
        | InstanceState::Alive
        | InstanceState::Dying
        | InstanceState::Dead => {
            tx.execute(
                "DELETE FROM alive_state_data
                WHERE instance = ?1",
                params![instance_id],
            )?;
            tx.execute(
                "DELETE FROM dying_state_data
                WHERE instance = ?1",
                params![instance_id],
            )?;

            tx.execute(
                "INSERT OR IGNORE
                INTO instances(hostname, next_check_datetime)
                VALUES (?1, ?2)",
                params![to.to_string(), time::rand_datetime_today()?.timestamp()],
            )?;
            let to_instance_id: u64 = tx.query_row(
                "SELECT id FROM instances WHERE hostname = ?1",
                params![to.to_string()],
                |row| row.get(0),
            )?;

            tx.execute(
                "INSERT INTO moving_state_data(instance, moving_since, moving_to)
                VALUES (?1, ?2, ?3)",
                params![instance_id, to_instance_id, now.timestamp()],
            )?;
            tx.execute(
                "UPDATE instances
                SET state = ?1,
                    last_check_datetime = ?2,
                    next_check_datetime = ?3
                WHERE id = ?4",
                params![
                    InstanceState::Moving as u8,
                    now.timestamp(),
                    time::rand_datetime_daily()?.timestamp(),
                    instance_id
                ],
            )?;
        }
        InstanceState::Moving => {
            let to_instance_id: u64 = tx.query_row(
                "SELECT id FROM instances WHERE hostname = ?1",
                params![to.to_string()],
                |row| row.get(0),
            )?;
            let is_moving_to_that_host_already: u64 = tx.query_row(
                "SELECT count(id)
                FROM moving_state_data
                WHERE instance = ?1
                    AND moving_to = ?2",
                params![instance_id, to_instance_id],
                |row| row.get(0),
            )?;
            if is_moving_to_that_host_already > 0 {
                // We're being redirected to the same host as before; update the counts
                tx.execute(
                    "UPDATE moving_state_data
                    SET redirects_count = redirects_count + 1
                    WHERE instance = ?1",
                    params![instance_id],
                )?;

                // If the instance is in "moving" state for over a week, consider it moved
                let (redirects_count, since): (u64, chrono::DateTime<Utc>) = tx.query_row(
                    "SELECT redirects_count, moving_since
                    FROM moving_state_data
                    WHERE instance = ?1",
                    params![instance_id],
                    |row| match (row.get(0), row.get(1)) {
                        (Ok(a), Ok(b)) => Ok((a, b)),
                        (Err(a), _) => Err(a),
                        (_, Err(b)) => Err(b),
                    },
                )?;
                let week_ago = now
                    .checked_sub_signed(Duration::weeks(1))
                    .ok_or_else(|| anyhow!("Couldn't subtract a week from today's datetime"))?;
                if redirects_count > 7 && since > week_ago {
                    tx.execute(
                        "DELETE FROM moving_state_data
                        WHERE instance = ?1",
                        params![instance_id],
                    )?;
                    tx.execute(
                        "INSERT INTO moved_state_data(instance, moved_to)
                        VALUES (?1, ?2)",
                        params![instance_id, to_instance_id],
                    )?;
                    tx.execute(
                        "UPDATE instances
                        SET state = ?1,
                            last_check_datetime = ?2,
                            next_check_datetime = ?3
                        WHERE id = ?4",
                        params![
                            InstanceState::Moved as u8,
                            now.timestamp(),
                            time::rand_datetime_weekly()?.timestamp(),
                            instance_id
                        ],
                    )?;
                } else {
                    tx.execute(
                        "UPDATE instances
                        SET last_check_datetime = ?1,
                            next_check_datetime = ?2
                        WHERE id = ?3",
                        params![
                            now.timestamp(),
                            time::rand_datetime_daily()?.timestamp(),
                            instance_id
                        ],
                    )?;
                }
            } else {
                // Previous checks got redirected to another host; restart the counts
                tx.execute(
                    "UPDATE moving_state_data
                    SET moving_since = ?1,
                        redirects_count = 1,
                        moving_to = ?2
                    WHERE instance = ?3",
                    params![now.timestamp(), to_instance_id, instance_id],
                )?;
                tx.execute(
                    "UPDATE instances
                    SET last_check_datetime = ?1,
                        next_check_datetime = ?2
                    WHERE id = ?3",
                    params![
                        now.timestamp(),
                        time::rand_datetime_daily()?.timestamp(),
                        instance_id
                    ],
                )?;
            }
        }
        InstanceState::Moved => {
            tx.execute(
                "UPDATE instances
                SET last_check_datetime = ?1,
                    next_check_datetime = ?2
                WHERE id = ?3",
                params![
                    now.timestamp(),
                    time::rand_datetime_weekly()?.timestamp(),
                    instance_id
                ],
            )?;
        }
    };

    Ok(tx.commit()?)
}

pub fn add_instance(logger: &Logger, conn: &Connection, instance: &Host) -> anyhow::Result<()> {
    let started = chrono::offset::Utc::now();
    let mut statement = conn.prepare_cached(
        "INSERT OR IGNORE
        INTO instances(hostname, next_check_datetime)
        VALUES (?1, ?2)",
    )?;
    statement.execute(params![
        instance.to_string(),
        time::rand_datetime_today()?.timestamp()
    ])?;
    let finished = chrono::offset::Utc::now();
    info!(logger, "add_instance() took {}", finished - started);

    Ok(())
}

/// Reschedule the instance according to its state.
///
/// This is meant to be used when the checker fails. In that case, we want to reschedule the
/// instance sometime in the future, so we keep tracking it. We do this according to the current
/// state of the instance, preserving the frequency of the checks.
pub fn reschedule(conn: &mut Connection, instance: &Host) -> anyhow::Result<()> {
    let tx = conn.transaction()?;

    let next_check_datetime = match get_instance_state(&tx, instance)? {
        InstanceState::Discovered => time::rand_datetime_daily()?,
        InstanceState::Alive => time::rand_datetime_daily()?,
        InstanceState::Dying => time::rand_datetime_daily()?,
        InstanceState::Dead => time::rand_datetime_weekly()?,
        InstanceState::Moving => time::rand_datetime_daily()?,
        InstanceState::Moved => time::rand_datetime_weekly()?,
    };

    tx.execute(
        "UPDATE instances
        SET next_check_datetime = ?1
        WHERE hostname = ?2",
        params![next_check_datetime.timestamp(), instance.to_string()],
    )?;

    Ok(tx.commit()?)
}

fn get_instance_state(tx: &Transaction, instance: &Host) -> anyhow::Result<InstanceState> {
    let state = tx.query_row(
        "SELECT state FROM instances WHERE hostname = ?1",
        params![instance.to_string()],
        |row| row.get(0),
    )?;
    InstanceState::from(state)
        .ok_or_else(|| anyhow!("Got invalid instance state from the DB: {}", state))
}

/// Picks the next instance to check, marks it as "being checked" in the database, and returns its
/// hostname.
///
/// The user has to call `finish_checking()` to un-mark the instance when the check is done.
pub fn pick_next_instance(conn: &mut Connection) -> Option<Host> {
    let mut get_hostname = || -> rusqlite::Result<Option<Host>> {
        let tx = conn.transaction()?;

        let result: Option<(u64, String)> = tx
            .query_row(
                "SELECT id, hostname
                FROM instances
                WHERE next_check_datetime < strftime('%s', CURRENT_TIMESTAMP)
                    AND check_started IS NULL
                ORDER BY random()
                LIMIT 1",
                [],
                |row| {
                    let id = row.get(0)?;
                    let hostname = row.get(1)?;
                    Ok((id, hostname))
                },
            )
            .optional()?;

        let (id, hostname) = match result {
            Some(r) => r,
            None => return Ok(None),
        };

        tx.execute(
            "UPDATE instances
            SET check_started = strftime('%s', CURRENT_TIMESTAMP)
            WHERE id = ?1",
            params![id],
        )?;

        tx.commit()?;
        Ok(Some(Host::Domain(hostname)))
    };

    get_hostname().ok().flatten()
}

pub fn finish_checking(conn: &Connection, instance: &Host) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE instances
        SET check_started = NULL
        WHERE hostname = ?1",
        params![instance.to_string()],
    )?;
    Ok(())
}

pub fn outstanding_checks_count(conn: &Connection) -> anyhow::Result<u64> {
    Ok(conn.query_row(
        "SELECT count(id)
        FROM instances
        WHERE next_check_datetime < strftime('%s', CURRENT_TIMESTAMP)
            AND check_started IS NULL",
        [],
        |row| row.get(0),
    )?)
}
