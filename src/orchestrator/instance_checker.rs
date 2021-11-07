use crate::{ipc, with_loc};
use anyhow::{anyhow, bail, Context};
use rusqlite::Connection;
use slog::{error, Logger};
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use url::Host;

use crate::orchestrator::db;

pub fn run(logger: Logger, instance: Host) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    println!("Checking {}", instance);

    let mut checker = CheckerHandle::new(logger.clone(), instance.clone())?;
    process_checker_response(&logger, &mut conn, &instance, &mut checker.inner)?;

    Ok(())
}

struct CheckerHandle {
    inner: Child,
    logger: Logger,
    instance: Host,
}

impl CheckerHandle {
    fn new(logger: Logger, instance: Host) -> anyhow::Result<Self> {
        let exe_path = env::current_exe()?;

        let inner = Command::new(exe_path)
            .arg("--check")
            .arg(instance.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context(with_loc!("Failed to spawn a checker"))?;

        Ok(Self {
            inner,
            logger,
            instance,
        })
    }
}

impl Drop for CheckerHandle {
    fn drop(&mut self) {
        match self.inner.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(e) => {
                error!(
                    self.logger,
                    "try_wait() for checker for {} failed: {}", self.instance, e
                );
            }
        }

        if let Err(e) = self.inner.kill() {
            error!(
                self.logger,
                "Failed to kill the checker for {}: {}", self.instance, e
            );
        }

        if let Err(e) = self.inner.wait() {
            error!(
                self.logger,
                "Failed to wait for the checker for {} to exit after kill: {}", self.instance, e
            );
        }
    }
}

fn process_checker_response(
    logger: &Logger,
    conn: &mut Connection,
    target: &Host,
    checker: &mut Child,
) -> anyhow::Result<()> {
    let output = checker
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to connect to checker's stdout"))?;
    let reader = BufReader::new(output);
    let mut lines = reader.lines();

    let state = {
        if let Some(line) = lines.next() {
            let line = line.context(with_loc!("Failed to read a line of checker's response"))?;
            serde_json::from_str(&line)
                .context(with_loc!("Failed to deserialize checker's response"))?
        } else {
            return db::on_sqlite_busy_retry(&mut || db::mark_dead(conn, target));
        }
    };

    match state {
        ipc::CheckerResponse::Peer { peer: _ } => {
            db::on_sqlite_busy_retry(&mut || db::mark_dead(conn, target))?;
            bail!("Expected the checker to respond with State, but it responded with Peer");
        }
        ipc::CheckerResponse::State { state } => match state {
            ipc::InstanceState::Alive { hide_from_list } => {
                db::on_sqlite_busy_retry(&mut || db::mark_alive(conn, target, hide_from_list))?;
                process_peers(logger, conn, target, lines)?;
            }
            ipc::InstanceState::Moving { to } => {
                println!(
                    "{} is moving to {}. This is a temporary redirect, so marking as dead",
                    target, to
                );
                db::on_sqlite_busy_retry(&mut || db::mark_dead(conn, target))?;
            }
            ipc::InstanceState::Moved { to } => {
                if &to == target {
                    println!("{} has moved to *itself*, marking as dead", target);
                    db::on_sqlite_busy_retry(&mut || db::mark_dead(conn, target))?;
                } else {
                    println!("{} has moved to {}", target, to);
                    db::on_sqlite_busy_retry(&mut || db::mark_moved(conn, target, &to))?;
                }
            }
        },
    }

    Ok(())
}

fn process_peers(
    _logger: &Logger,
    conn: &mut Connection,
    target: &Host,
    lines: impl Iterator<Item = std::io::Result<String>>,
) -> anyhow::Result<()> {
    let mut peers_count = 0;
    for response in lines {
        let response =
            response.context(with_loc!("Failed to read a line of checker's response"))?;

        let response: ipc::CheckerResponse = serde_json::from_str(&response)
            .context(with_loc!("Failed to deserialize checker's response"))?;

        match response {
            ipc::CheckerResponse::State { state: _ } => {
                bail!("Expected the checker to respond with Peer, but it responded with State")
            }
            ipc::CheckerResponse::Peer { peer } => {
                db::on_sqlite_busy_retry(&mut || db::add_instance(conn, &peer))?;
                peers_count += 1;
            }
        }
    }

    println!("{} has {} peers", target, peers_count);

    Ok(())
}
