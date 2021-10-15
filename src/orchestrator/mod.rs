use crate::ipc;
use anyhow::{anyhow, bail, Context};
use chrono::prelude::*;
use rusqlite::{params, Connection};
use slog::Logger;
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

pub fn main(_logger: Logger) -> anyhow::Result<()> {
    let db = init_database().context("Failed to initialize the database")?;
    reschedule_missed_checks(db)?;
    run_checker("mastodon.social").context("Failed to check mastodon.social")
}

fn init_database() -> anyhow::Result<Connection> {
    let conn = Connection::open("fediverse.observer.db")?;

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

    Ok(conn)
}

/// Random datetime about a day away from now (±10%).
fn rand_datetime_daily() -> anyhow::Result<DateTime<Utc>> {
    use chrono::Duration;

    const OFFSET: i64 = 24 * 60 * 60 / 10; // 10% of the day
    let offset = Duration::days(1) + Duration::seconds(fastrand::i64(-OFFSET..OFFSET));
    Utc::now().checked_add_signed(offset).ok_or_else(|| {
        anyhow!(
            "Failed to add {} to the current datetime, as it will lead to overflow",
            offset
        )
    })
}

fn reschedule_missed_checks(db: Connection) -> anyhow::Result<()> {
    let mut statement =
        db.prepare("SELECT id FROM instances WHERE next_check_datetime < CURRENT_TIMESTAMP")?;
    let mut ids = statement.query([])?;
    while let Some(row) = ids.next()? {
        let instance_id: u64 = row.get(0)?;
        db.execute(
            "UPDATE instances SET next_check_datetime = ?1 WHERE id = ?2",
            params![rand_datetime_daily()?, instance_id],
        )?;
    }
    Ok(())
}

fn run_checker(target: &str) -> anyhow::Result<()> {
    let exe_path = env::args_os()
        .nth(0)
        .ok_or_else(|| anyhow!("Failed to determine the path to the executable"))?;

    let mut checker = Command::new(exe_path)
        .arg("--check")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to spawn a checker")?;

    let output = checker
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to connect to checker's stdout"))?;
    let reader = BufReader::new(output);
    let mut lines = reader.lines();

    let state = {
        if let Some(line) = lines.next() {
            let line = line.context("Failed to read a line of checker's response")?;
            serde_json::from_str(&line).context("Failed to deserialize checker's response")?
        } else {
            return Ok(());
        }
    };

    match state {
        ipc::CheckerResponse::Peer { peer: _ } => {
            bail!("Expected the checker to respond with State, but it responded with Peer");
        }
        ipc::CheckerResponse::State { state } => match state {
            ipc::InstanceState::Alive => process_peers(target, lines)?,
            ipc::InstanceState::Moving { to } => {
                println!("{} is moving to {}", target, to)
            }
            ipc::InstanceState::Moved { to } => {
                println!("{} has moved to {}", target, to)
            }
        },
    }

    Ok(())
}

fn process_peers(
    target: &str,
    lines: impl Iterator<Item = std::io::Result<String>>,
) -> anyhow::Result<()> {
    let mut peers_count = 0;
    for response in lines {
        let response = response.context("Failed to read a line of checker's response")?;

        let response: ipc::CheckerResponse =
            serde_json::from_str(&response).context("Failed to deserialize checker's response")?;

        match response {
            ipc::CheckerResponse::State { state: _ } => {
                bail!("Expected the checker to respond with Peer, but it responded with State")
            }
            ipc::CheckerResponse::Peer { peer: _ } => peers_count += 1,
        }
    }

    println!("{} has {} peers", target, peers_count);

    Ok(())
}
