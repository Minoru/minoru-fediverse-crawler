//! Functions to query and update the database, plus some helpers.

use crate::{domain::Domain, time, with_loc};
use anyhow::{anyhow, Context};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use rusqlite::{
    params,
    types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef},
    Connection, ToSql, Transaction,
};

fn is_sqlite_busy_error(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<rusqlite::Error>() {
        if let Some(code) = error.sqlite_error_code() {
            return code == rusqlite::ErrorCode::DatabaseBusy;
        }
    }

    false
}

/// A helper that, upon encountering `SQLITE_BUSY`, just waits a bit and retries.
pub fn on_sqlite_busy_retry_indefinitely<T, F>(f: &mut F) -> anyhow::Result<T>
where
    F: FnMut() -> anyhow::Result<T>,
{
    loop {
        match f() {
            result @ Ok(_) => return result,
            Err(e) => {
                if is_sqlite_busy_error(&e) {
                    let duration = fastrand::u64(1..50);
                    std::thread::sleep(std::time::Duration::from_millis(duration));
                } else {
                    return Err(e);
                }
            }
        }
    }
}

/// A helper that, upon encountering `SQLITE_BUSY`, just waits a bit and retries, up to 100 times.
pub fn on_sqlite_busy_retry<T, F>(f: &mut F) -> anyhow::Result<T>
where
    F: FnMut() -> anyhow::Result<T>,
{
    for _ in 0..100 {
        match f() {
            result @ Ok(_) => return result,
            Err(e) => {
                if is_sqlite_busy_error(&e) {
                    let duration = fastrand::u64(1..50);
                    std::thread::sleep(std::time::Duration::from_millis(duration));
                } else {
                    return Err(e);
                }
            }
        }
    }

    f()
}

/// Wrapper over `chrono::DateTime<Utc>`. In SQL, it's stored as an integer number of seconds since
/// January 1, 1970.
struct UnixTimestamp(DateTime<Utc>);

impl ToSql for UnixTimestamp {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.timestamp()))
    }
}

impl FromSql for UnixTimestamp {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let t = value.as_i64()?;
        let t = NaiveDateTime::from_timestamp_opt(t, 0).ok_or(FromSqlError::OutOfRange(t))?;
        let t = t.and_utc();
        let t = UnixTimestamp(t);
        Ok(t)
    }
}

/// Possible states of a Fediverse instance, mapped to integers used in the database.
#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum InstanceState {
    Discovered = 0,
    Alive = 1,
    Dying = 2,
    Dead = 3,
    Moving = 4,
    Moved = 5,
}

impl ToSql for InstanceState {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(*self as i64))
    }
}

impl FromSql for InstanceState {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let v = value.as_i64()?;
        match v {
            0 => Ok(Self::Discovered),
            1 => Ok(Self::Alive),
            2 => Ok(Self::Dying),
            3 => Ok(Self::Dead),
            4 => Ok(Self::Moving),
            5 => Ok(Self::Moved),
            _ => Err(rusqlite::types::FromSqlError::OutOfRange(v)),
        }
    }
}

/// Connect to the database.
pub fn open() -> anyhow::Result<Connection> {
    let conn = Connection::open("minoru-fediverse-crawler.db")
        .context(with_loc!("Failed to initialize the database"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context(with_loc!("Switching to WAL mode"))?;
    Ok(conn)
}

/// Initialize the database.
///
/// This is safe to run concurrently with other processes; it will do nothing if the database is
/// already initialized.
pub fn init(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context(with_loc!("Beginning a transaction"))?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS states(
            id INTEGER PRIMARY KEY NOT NULL,
            state TEXT UNIQUE NOT NULL
        )",
        [],
    )
    .context(with_loc!("Creating table 'states'"))?;
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
    .context(with_loc!("Filling table 'states'"))?;
    tx.execute(
        "CREATE TABLE IF NOT EXISTS instances(
            id INTEGER PRIMARY KEY NOT NULL,
            hostname TEXT UNIQUE NOT NULL,
            state REFERENCES states(id) NOT NULL DEFAULT 0,
            next_check_datetime INTEGER DEFAULT (strftime('%s', CURRENT_TIMESTAMP))
        )",
        [],
    )
    .context(with_loc!("Creating table 'instances'"))?;
    tx.execute(
        r#"INSERT OR IGNORE
        INTO instances(hostname)
        VALUES ("mastodon.social")"#,
        [],
    )
    .context(with_loc!("Adding mastodon.social to the 'instances' table"))?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS instances_next_check_datetime_idx
        ON instances(next_check_datetime)",
        [],
    )
    .context(with_loc!(
        "Creating index on instances(next_check_datetime)"
    ))?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS instances_states_hostname
        ON instances(state, hostname)",
        [],
    )
    .context(with_loc!("Creating index 'instances_states_hostname'"))?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS dying_state_data(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE,
            previous_state REFERENCES states(id) NOT NULL,
            dying_since INTEGER NOT NULL,
            failed_checks_count INTEGER NOT NULL DEFAULT 1
        )",
        [],
    )
    .context(with_loc!("Creating table 'dying_state_data'"))?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS dying_state_data_previous_state_instance
        ON dying_state_data(previous_state, instance)",
        [],
    )
    .context(with_loc!(
        "Creating index 'dying_state_data_previous_state_instance'"
    ))?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS moving_state_data(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE,
            previous_state REFERENCES states(id) NOT NULL,
            moving_since INTEGER NOT NULL,
            redirects_count INTEGER NOT NULL DEFAULT 1,
            moving_to REFERENCES instances(id) NOT NULL
        )",
        [],
    )
    .context(with_loc!("Creating table 'moving_state_data'"))?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS moving_state_data_previous_state_instance
        ON moving_state_data(previous_state, instance)",
        [],
    )
    .context(with_loc!(
        "Creating index 'moving_state_data_previous_state_instance'"
    ))?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS moved_state_data(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE,
            moved_to REFERENCES instances(id) NOT NULL
        )",
        [],
    )
    .context(with_loc!("Creating table 'moved_state_data'"))?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS hidden_instances(
            id INTEGER PRIMARY KEY NOT NULL,
            instance REFERENCES instances(id) NOT NULL UNIQUE,
            hide_from_list INTEGER NOT NULL DEFAULT 0
        )",
        [],
    )
    .context(with_loc!("Creating table hidden_instances"))?;
    tx.execute(
        "CREATE INDEX IF NOT EXISTS hidden_instances_hide_from_list_instance
        ON hidden_instances(hide_from_list, instance)",
        [],
    )
    .context(with_loc!(
        "Creating index 'hidden_instances_hide_from_list_instance'"
    ))?;

    tx.commit().context(with_loc!("Committing the transaction"))
}

/// For any check whose time has already passed, move that check up to 24 hours from now.
pub fn reschedule_missed_checks(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context(with_loc!("Beginning a transaction"))?;

    {
        let mut statement = tx
            .prepare(
                "SELECT id
                FROM instances
                WHERE next_check_datetime < strftime('%s', CURRENT_TIMESTAMP)",
            )
            .context(with_loc!("Preparing a SELECT"))?;
        let mut ids = statement.query([])?;
        while let Some(row) = ids.next()? {
            let instance_id: i64 = row.get(0).context(with_loc!("Getting `instance_id`"))?;
            let next_check =
                time::sometime_today().context(with_loc!("Picking next check's datetime"))?;
            reschedule_instance_to(&tx, instance_id, next_check)
                .context(with_loc!("Rescheduling instance"))?;
        }
    }

    tx.commit().context(with_loc!("Committing the transaction"))
}

/// Note down that the instance is alive.
pub fn mark_alive(
    conn: &mut Connection,
    instance: &Domain,
    hide_from_list: bool,
) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context(with_loc!("Beginning a transaction"))?;

    let (instance_id, state) =
        get_instance(&tx, instance).context(with_loc!("Getting instance id and state"))?;

    set_hide_instance_from_list(&tx, instance_id, hide_from_list)
        .context(with_loc!("Updating the flag in `hidden_instances`"))?;

    if state == InstanceState::Alive {
        return tx
            .commit()
            .context(with_loc!("Committing the transaction early"));
    }

    assert_ne!(state, InstanceState::Alive);

    // Delete any previous state data related to this instance
    match state {
        InstanceState::Dying => delete_dying_state_data(&tx, instance_id)
            .context(with_loc!("Deleting from table `dying_state_data'"))?,
        InstanceState::Moving => delete_moving_state_data(&tx, instance_id)
            .context(with_loc!("Deleting from table 'moving_state_data'"))?,
        InstanceState::Moved => delete_moved_state_data(&tx, instance_id)
            .context(with_loc!("Deleting from table 'moved_state_data'"))?,
        _ => {}
    }

    set_instance_state(&tx, instance_id, InstanceState::Alive)
        .context(with_loc!("Marking instance as alive"))?;

    if state == InstanceState::Dead || state == InstanceState::Moved {
        let next_check =
            time::about_a_day_from_now().context(with_loc!("Picking next check's datetime"))?;
        reschedule_instance_to(&tx, instance_id, next_check)
            .context(with_loc!("Rescheduling instance"))?;
    }

    tx.commit().context(with_loc!("Committing the transaction"))
}

/// Note down that the instance is dead.
///
/// This will first move the instance into a "dying" state, and after a week of calling this
/// function, it will finally move the instance into the "dead" state.
pub fn mark_dead(conn: &mut Connection, instance: &Domain) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context(with_loc!("Beginning a transaction"))?;

    let now = Utc::now();
    let (instance_id, state) =
        get_instance(&tx, instance).context(with_loc!("Getting instance id and state"))?;
    if state == InstanceState::Dead {
        return Ok(());
    }

    assert_ne!(state, InstanceState::Dead);

    // Delete any unrelated state data for this instance
    match state {
        InstanceState::Moving => delete_moving_state_data(&tx, instance_id)
            .context(with_loc!("Deleting from table 'moving_state_data'"))?,
        InstanceState::Moved => delete_moved_state_data(&tx, instance_id)
            .context(with_loc!("Deleting from table 'moved_state_data'"))?,
        _ => {}
    }

    match state {
        InstanceState::Dead => {}

        InstanceState::Discovered
        | InstanceState::Alive
        | InstanceState::Moving
        | InstanceState::Moved => {
            tx.execute(
                "INSERT
                INTO dying_state_data(instance, previous_state, dying_since)
                VALUES (?1, ?2, ?3)",
                params![instance_id, state, UnixTimestamp(now)],
            )
            .context(with_loc!("Inserting into table 'dying_state_data'"))?;

            set_instance_state(&tx, instance_id, InstanceState::Dying)
                .context(with_loc!("Marking instance as dying"))?;
        }

        InstanceState::Dying => {
            tx.execute(
                "UPDATE dying_state_data
                SET failed_checks_count = failed_checks_count + 1
                WHERE instance = ?1",
                params![instance_id],
            )
            .context(with_loc!("Updating table 'dying_state_data'"))?;

            let (checks_count, since): (u64, DateTime<Utc>) = tx
                .query_row(
                    "SELECT failed_checks_count, dying_since
                    FROM dying_state_data
                    WHERE instance = ?1",
                    params![instance_id],
                    |row| {
                        let failed_checks_count = row.get(0)?;
                        let dying_since: UnixTimestamp = row.get(1)?;
                        Ok((failed_checks_count, dying_since.0))
                    },
                )
                .context(with_loc!("Selecting data from 'dying_state_data'"))?;
            let week_ago = now
                .checked_sub_signed(Duration::weeks(1))
                .ok_or_else(|| anyhow!("Couldn't subtract a week from today's datetime"))?;
            // "Daily" checks are run every 29 hours; 1 week = 7 days = 168 hours, that's 5.8
            // "daily" checks per peal week. So 6 failed checks means "we've been failing for about
            // a week".
            if checks_count > 6 && since < week_ago {
                delete_from_hidden_instances(&tx, instance_id)
                    .context(with_loc!("Deleting from 'hidden_instances'"))?;
                delete_dying_state_data(&tx, instance_id)
                    .context(with_loc!("Deleting from table 'dying_state_data'"))?;
                let next_check = time::about_a_week_from_now()
                    .context(with_loc!("Picking next check's datetime"))?;
                reschedule_instance_to(&tx, instance_id, next_check)
                    .context(with_loc!("Rescheduling instance"))?;
                set_instance_state(&tx, instance_id, InstanceState::Dead)
                    .context(with_loc!("Marking instance as dead"))?;
            }
        }
    }

    tx.commit().context(with_loc!("Committing the transaction"))
}

fn is_moving_to_that_host_already(tx: &Transaction, from: i64, to: i64) -> anyhow::Result<bool> {
    Ok(tx.query_row(
        "SELECT count(id)
        FROM moving_state_data
        WHERE instance = ?1
            AND moving_to = ?2",
        params![from, to],
        |row| {
            let count: u64 = row.get(0)?;
            Ok(count > 0)
        },
    )?)
}

fn has_moved_to_that_host_already(tx: &Transaction, from: i64, to: i64) -> anyhow::Result<bool> {
    Ok(tx.query_row(
        "SELECT count(id)
        FROM moved_state_data
        WHERE instance = ?1
            AND moved_to = ?2",
        params![from, to],
        |row| {
            let count: u64 = row.get(0)?;
            Ok(count > 0)
        },
    )?)
}

/// Note down that the instance has moved to another.
///
/// This will initially mark the instance with the "moving" state, and after calling this function
/// for a week, it will finally mark the instance as "moved". Changing the target instance resets
/// the count.
pub fn mark_moved(conn: &mut Connection, instance: &Domain, to: &Domain) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context(with_loc!("Beginning a transaction"))?;

    let now = Utc::now();
    let (instance_id, state) =
        get_instance(&tx, instance).context(with_loc!("Getting instance id and state"))?;
    if state == InstanceState::Moved {
        let (to_instance_id, _) =
            get_instance(&tx, to).context(with_loc!("Getting instance id"))?;
        let already_moved_there = has_moved_to_that_host_already(&tx, instance_id, to_instance_id)
            .context(with_loc!("Checking if moved to that instance already"))?;
        if !already_moved_there {
            // Redirect's target changed; change the state back to "moving"

            delete_moved_state_data(&tx, instance_id)
                .context(with_loc!("Deleting from table 'moved_state_data'"))?;

            tx.execute(
                "INSERT INTO moving_state_data(instance, previous_state, moving_since, moving_to)
                VALUES (?1, ?2, ?3, ?4)",
                params![instance_id, state, UnixTimestamp(now), to_instance_id],
            )
            .context(with_loc!("Inserting into 'moving_state_data'"))?;

            set_instance_state(&tx, instance_id, InstanceState::Moving)
                .context(with_loc!("Marking instance as moving"))?;
        }

        return tx.commit().context(with_loc!("Committing the transaction"));
    }

    assert_ne!(state, InstanceState::Moved);

    if state == InstanceState::Dying {
        delete_dying_state_data(&tx, instance_id)
            .context(with_loc!("Deleting from table 'dying_state_data'"))?;
    }

    match state {
        InstanceState::Moved => {}

        InstanceState::Discovered
        | InstanceState::Alive
        | InstanceState::Dying
        | InstanceState::Dead => {
            let next_check =
                time::sometime_today().context(with_loc!("Picking next check's datatime"))?;
            tx.execute(
                "INSERT OR IGNORE
                INTO instances(hostname, next_check_datetime)
                VALUES (?1, ?2)",
                params![to.to_string(), UnixTimestamp(next_check)],
            )
            .context(with_loc!("Inserting into table 'instances'"))?;
            let (to_instance_id, _) = get_instance(&tx, to)
                .context(with_loc!("Getting id of the newly inserted instance"))?;

            tx.execute(
                "INSERT INTO moving_state_data(instance, previous_state, moving_since, moving_to)
                VALUES (?1, ?2, ?3, ?4)",
                params![instance_id, state, UnixTimestamp(now), to_instance_id],
            )
            .context(with_loc!("Inserting into 'moving_state_data'"))?;

            set_instance_state(&tx, instance_id, InstanceState::Moving)
                .context(with_loc!("Marking instance as moving"))?;
        }

        InstanceState::Moving => {
            let (to_instance_id, _) =
                get_instance(&tx, to).context(with_loc!("Getting instance id"))?;
            let already_moving_there =
                is_moving_to_that_host_already(&tx, instance_id, to_instance_id)
                    .context(with_loc!("Checking if moving to that instance already"))?;
            if already_moving_there {
                // We're being redirected to the same host as before; update the counts
                tx.execute(
                    "UPDATE moving_state_data
                    SET redirects_count = redirects_count + 1
                    WHERE instance = ?1",
                    params![instance_id],
                )
                .context(with_loc!("Updating table 'moving_state_data'"))?;

                // If the instance is in "moving" state for over a week, consider it moved
                let (redirects_count, since): (u64, DateTime<Utc>) = tx
                    .query_row(
                        "SELECT redirects_count, moving_since
                        FROM moving_state_data
                        WHERE instance = ?1",
                        params![instance_id],
                        |row| {
                            let redirects_count = row.get(0)?;
                            let moving_since: UnixTimestamp = row.get(1)?;
                            Ok((redirects_count, moving_since.0))
                        },
                    )
                    .context(with_loc!("Getting data from 'moving_state_data'"))?;
                let week_ago = now
                    .checked_sub_signed(Duration::weeks(1))
                    .ok_or_else(|| anyhow!("Couldn't subtract a week from today's datetime"))?;
                // "Daily" checks are run every 29 hours; 1 week = 7 days = 168 hours, that's 5.8
                // "daily" checks per peal week. So 6 redirects mean "we've been redirected for
                // about a week".
                if redirects_count > 6 && since < week_ago {
                    delete_from_hidden_instances(&tx, instance_id)
                        .context(with_loc!("Deleting from 'hidden_instances'"))?;
                    delete_moving_state_data(&tx, instance_id)
                        .context(with_loc!("Deleting from 'moving_state_data'"))?;
                    tx.execute(
                        "INSERT INTO moved_state_data(instance, moved_to)
                        VALUES (?1, ?2)",
                        params![instance_id, to_instance_id],
                    )
                    .context(with_loc!("Inserting into 'moved_state_data'"))?;
                    let next_check = time::about_a_week_from_now()
                        .context(with_loc!("Picking next check's datetime"))?;
                    reschedule_instance_to(&tx, instance_id, next_check)
                        .context(with_loc!("Rescheduling instance"))?;
                    set_instance_state(&tx, instance_id, InstanceState::Moved)
                        .context(with_loc!("Marking instance as moved"))?;
                }
            } else {
                // Previous checks got redirected to another host; restart the counts
                tx.execute(
                    "UPDATE moving_state_data
                    SET moving_since = ?1,
                        redirects_count = 1,
                        moving_to = ?2
                    WHERE instance = ?3",
                    params![UnixTimestamp(now), to_instance_id, instance_id],
                )
                .context(with_loc!("Updating table 'moving_state_data'"))?;
            }
        }
    };

    tx.commit().context(with_loc!("Committing the transaction"))
}

/// Attempt to add an instance to the database. Does nothing if the instance is already known.
pub fn add_instance(conn: &Connection, instance: &Domain) -> anyhow::Result<()> {
    let mut statement = conn
        .prepare_cached(
            "INSERT OR IGNORE
            INTO instances(hostname, next_check_datetime)
            VALUES (?1, ?2)",
        )
        .context(with_loc!("Preparing cached INSERT OR IGNORE statement"))?;
    let next_check = time::sometime_today().context(with_loc!("Picking next check's datetime"))?;
    statement
        .execute(params![instance.to_string(), UnixTimestamp(next_check)])
        .context(with_loc!("Executing the statement"))?;

    Ok(())
}

/// Reschedule the instance according to its state.
pub fn reschedule(conn: &mut Connection, instance: &Domain) -> anyhow::Result<()> {
    let tx = conn
        .transaction()
        .context(with_loc!("Beginning a transaction"))?;

    let (instance_id, state) =
        get_instance(&tx, instance).context(with_loc!("Getting instance id and state"))?;

    let next_check_datetime = match state {
        InstanceState::Discovered => time::about_a_day_from_now(),
        InstanceState::Alive => time::about_a_day_from_now(),
        InstanceState::Dying => time::about_a_day_from_now(),
        InstanceState::Dead => time::about_a_week_from_now(),
        InstanceState::Moving => time::about_a_day_from_now(),
        InstanceState::Moved => time::about_a_week_from_now(),
    }
    .context(with_loc!("Picking next check's datetiem"))?;

    tx.execute(
        "UPDATE instances
        SET next_check_datetime = ?1
        WHERE id = ?2",
        params![UnixTimestamp(next_check_datetime), instance_id],
    )
    .context(with_loc!("Updating table 'instances'"))?;

    tx.commit().context(with_loc!("Committing the transaction"))
}

fn get_instance(tx: &Transaction, instance: &Domain) -> anyhow::Result<(i64, InstanceState)> {
    tx.query_row(
        "SELECT id, state
        FROM instances
        WHERE hostname = ?1",
        params![instance.to_string()],
        |row| {
            let id = row.get(0)?;
            let state = row.get(1)?;
            Ok((id, state))
        },
    )
    .context(with_loc!("Getting instance's id and state"))
}

fn delete_dying_state_data(tx: &Transaction, id: i64) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM dying_state_data
        WHERE instance = ?1",
        params![id],
    )
    .map(|_| ())
    .context(with_loc!("Deleting from table `dying_state_data'"))
}

fn delete_moving_state_data(tx: &Transaction, id: i64) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM moving_state_data
        WHERE instance = ?1",
        params![id],
    )
    .map(|_| ())
    .context(with_loc!("Deleting from table 'moving_state_data'"))
}

fn delete_moved_state_data(tx: &Transaction, id: i64) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM moved_state_data
        WHERE instance = ?1",
        params![id],
    )
    .map(|_| ())
    .context(with_loc!("Deleting from table 'moved_state_data'"))
}

fn reschedule_instance_to(
    tx: &Transaction,
    id: i64,
    next_check_datetime: DateTime<Utc>,
) -> anyhow::Result<()> {
    tx.execute(
        "UPDATE instances
        SET next_check_datetime = ?1
        WHERE id = ?2",
        params![UnixTimestamp(next_check_datetime), id],
    )
    .map(|_| ())
    .context(with_loc!("Updating table 'instances'"))
}

fn set_instance_state(tx: &Transaction, id: i64, state: InstanceState) -> anyhow::Result<()> {
    tx.execute(
        "UPDATE instances
        SET state = ?1
        WHERE id = ?2",
        params![state, id],
    )
    .map(|_| ())
    .context(with_loc!("Updating table 'instances'"))
}

/// Pick the next instance to check, i.e. the one with the smallest `next_check_datetime` value.
pub fn pick_next_instance(conn: &Connection) -> anyhow::Result<(Domain, DateTime<Utc>)> {
    let (hostname, next_check_datetime): (String, DateTime<Utc>) = conn
        .query_row(
            "SELECT hostname, next_check_datetime
            FROM instances
            ORDER BY next_check_datetime ASC
            LIMIT 1",
            [],
            |row| {
                let hostname = row.get(0)?;
                let next_check_datetime: UnixTimestamp = row.get(1)?;
                Ok((hostname, next_check_datetime.0))
            },
        )
        .context(with_loc!("Picking next instance"))?;
    let domain = Domain::from_str(&hostname)?;
    Ok((domain, next_check_datetime))
}

fn set_hide_instance_from_list(
    tx: &Transaction,
    instance: i64,
    hide_from_list: bool,
) -> anyhow::Result<()> {
    tx.execute(
        "INSERT OR REPLACE
        INTO hidden_instances(instance, hide_from_list)
        VALUES (?1, ?2)",
        params![instance, hide_from_list],
    )?;
    Ok(())
}

fn delete_from_hidden_instances(tx: &Transaction, instance: i64) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM hidden_instances
        WHERE instance = ?1",
        params![instance],
    )?;
    Ok(())
}
