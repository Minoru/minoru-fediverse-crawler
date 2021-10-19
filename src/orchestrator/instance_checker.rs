use crate::ipc;
use anyhow::{anyhow, bail, Context};
use rusqlite::Connection;
use slog::{error, o, Logger};
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use url::Host;

use crate::orchestrator::db;

pub fn run(logger: Logger) -> anyhow::Result<()> {
    let conn = db::open()?;
    loop {
        if let Some(instance) =
            db::pick_next_instance(&conn).context("Picking an instance to check")?
        {
            println!("Checking {}", instance);

            let logger = logger.new(o!("host" => instance.to_string()));
            InstanceChecker::new(logger, instance)?.run()?;
        } else {
            println!("Waiting for some checks to come due...");
            std::thread::sleep(std::time::Duration::new(1, 0));
        }
    }
}

struct CheckerHandle {
    inner: Child,
    logger: Logger,
    instance: Host,
}

impl CheckerHandle {
    fn new(logger: Logger, instance: Host) -> anyhow::Result<Self> {
        let exe_path = env::args_os()
            .next()
            .ok_or_else(|| anyhow!("Failed to determine the path to the executable"))?;

        let inner = Command::new(exe_path)
            .arg("--check")
            .arg(instance.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn a checker")?;

        Ok(Self {
            inner,
            logger,
            instance,
        })
    }
}

impl Drop for CheckerHandle {
    fn drop(&mut self) {
        if self.inner.try_wait().is_err() {
            if let Err(e) = self.inner.kill() {
                error!(
                    self.logger,
                    "Failed to kill the checker for {}: {}", self.instance, e
                );
            }
            if let Err(e) = self.inner.try_wait() {
                error!(
                    self.logger,
                    "The checker for {} survived the kill() somehow: {}", self.instance, e
                );
            }
        }
    }
}

pub struct InstanceChecker {
    conn: Connection,
    logger: Logger,
    instance: Host,
}

impl InstanceChecker {
    pub fn new(logger: Logger, instance: Host) -> anyhow::Result<Self> {
        let conn = db::open()?;
        db::start_checking(&conn, &instance)?;
        Ok(Self {
            conn,
            logger,
            instance,
        })
    }

    pub fn run(&mut self) -> anyhow::Result<()> {
        let mut checker = CheckerHandle::new(self.logger.clone(), self.instance.clone())?;
        if let Err(e) = process_checker_response(&self.logger, &self.instance, &mut checker.inner) {
            db::reschedule(&mut self.conn, &self.instance)
                .with_context(|| format!("While handling a checker error: {}", e))?;
        }
        Ok(())
    }
}

impl Drop for InstanceChecker {
    fn drop(&mut self) {
        if let Err(e) = db::finish_checking(&self.conn, &self.instance) {
            error!(
                self.logger,
                "Error marking {} as checked in the DB: {}",
                self.instance.to_string(),
                e
            );
        }
    }
}

fn process_checker_response(
    logger: &Logger,
    target: &Host,
    checker: &mut Child,
) -> anyhow::Result<()> {
    let output = checker
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to connect to checker's stdout"))?;
    let reader = BufReader::new(output);
    let mut lines = reader.lines();

    let mut conn = db::open()?;

    let state = {
        if let Some(line) = lines.next() {
            let line = line.context("Failed to read a line of checker's response")?;
            serde_json::from_str(&line).context("Failed to deserialize checker's response")?
        } else {
            return db::mark_dead(&mut conn, target);
        }
    };

    match state {
        ipc::CheckerResponse::Peer { peer: _ } => {
            db::mark_dead(&mut conn, target)?;
            bail!("Expected the checker to respond with State, but it responded with Peer");
        }
        ipc::CheckerResponse::State { state } => match state {
            ipc::InstanceState::Alive => {
                db::mark_alive(&mut conn, target)?;
                process_peers(logger, &mut conn, target, lines)?;
            }
            ipc::InstanceState::Moving { to } => {
                println!("{} is moving to {}", target, to);
                db::reschedule(&mut conn, target)?;
            }
            ipc::InstanceState::Moved { to } => {
                println!("{} has moved to {}", target, to);
                db::mark_moved(&mut conn, target, &to)?;
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
        let response = response.context("Failed to read a line of checker's response")?;

        let response: ipc::CheckerResponse =
            serde_json::from_str(&response).context("Failed to deserialize checker's response")?;

        match response {
            ipc::CheckerResponse::State { state: _ } => {
                bail!("Expected the checker to respond with Peer, but it responded with State")
            }
            ipc::CheckerResponse::Peer { peer } => {
                db::add_instance(conn, target, &peer)?;
                peers_count += 1;
            }
        }
    }

    println!("{} has {} peers", target, peers_count);

    Ok(())
}
