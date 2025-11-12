use crate::{domain::Domain, ipc, orchestrator::db, with_loc};
use anyhow::{Context, anyhow, bail};
use rusqlite::Connection;
use slog::{Logger, error, info};
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

pub fn run(logger: Logger, instance: Domain) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    println!("Checking {instance}");

    let mut checker = CheckerHandle::new(logger.clone(), instance.clone())?;
    process_checker_response(&logger, &mut conn, &instance, &mut checker.inner)?;

    Ok(())
}

struct CheckerHandle {
    inner: Child,
    logger: Logger,
    instance: Domain,
}

impl CheckerHandle {
    fn new(logger: Logger, instance: Domain) -> anyhow::Result<Self> {
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
    target: &Domain,
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
            info!(
                logger,
                "No response from checker, marking the instance as dead"
            );

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
                info!(logger, "The instance is alive");

                db::on_sqlite_busy_retry(&mut || db::mark_alive(conn, target, hide_from_list))?;
                process_peers(logger, conn, target, lines)?;
            }
            ipc::InstanceState::Moving { to } => {
                let msg = format!(
                    "{target} is moving to {to}. This is a temporary redirect, so marking as dead"
                );
                info!(logger, "{}", msg);
                println!("{msg}");

                db::on_sqlite_busy_retry(&mut || db::mark_dead(conn, target))?;
            }
            ipc::InstanceState::Moved { to } => {
                match Domain::from_host(&to) {
                    Ok(to) => {
                        if &to == target {
                            let msg = format!("{target} has moved to *itself*, marking as dead");
                            info!(logger, "{}", msg);
                            println!("{msg}");
                            db::on_sqlite_busy_retry(&mut || db::mark_dead(conn, target))?;
                        } else {
                            let msg = format!("{target} has moved to {to}");
                            info!(logger, "{}", msg);
                            println!("{msg}");
                            db::on_sqlite_busy_retry(&mut || db::mark_moved(conn, target, &to))?;
                        }
                    }

                    Err(e) => {
                        let msg = format!(
                            "{target} has moved to {to}, which is not a valid domain name ({e}); marking as dead"
                        );
                        info!(logger, "{}", msg);
                        println!("{msg}");
                        db::on_sqlite_busy_retry(&mut || db::mark_dead(conn, target))?;
                    }
                };
            }
        },
    }

    Ok(())
}

fn process_peers(
    logger: &Logger,
    conn: &mut Connection,
    target: &Domain,
    lines: impl Iterator<Item = std::io::Result<String>>,
) -> anyhow::Result<()> {
    let mut peers_count: Option<u64> = Some(0);
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
                match Domain::from_host(&peer).and_then(|peer| {
                    db::on_sqlite_busy_retry(&mut || db::add_instance(conn, &peer))
                }) {
                    Err(e) => {
                        info!(logger, "Failed to add {} to the database: {:?}", peer, e);
                    }
                    _ => {
                        peers_count = peers_count.and_then(|x| x.checked_add(1));
                    }
                }
            }
        }
    }

    let msg = match peers_count {
        None => format!("{} has more than {} peers", target, u64::MAX),
        Some(count) => format!("{target} has {count} peers"),
    };
    info!(logger, "{}", msg);
    println!("{msg}");

    Ok(())
}
