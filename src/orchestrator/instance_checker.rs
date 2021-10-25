use crate::ipc;
use anyhow::{anyhow, bail, Context};
use rusqlite::Connection;
use slog::{error, o, Logger};
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use url::Host;

use crate::orchestrator::db;

const RECEIVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub fn run(logger: Logger, rx: crossbeam_channel::Receiver<Host>) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    loop {
        let instance = rx.recv_timeout(RECEIVE_TIMEOUT)?;

        println!("Checking {}", instance);

        let logger = logger.new(o!("host" => instance.to_string()));
        if let Err(e) = check(logger.clone(), &mut conn, &instance) {
            error!(logger, "{}", e);
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
        let exe_path = env::current_exe()?;

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

fn check(logger: Logger, conn: &mut Connection, instance: &Host) -> anyhow::Result<()> {
    let mut checker = CheckerHandle::new(logger.clone(), instance.clone())?;
    process_checker_response(&logger, conn, instance, &mut checker.inner)?;
    Ok(())
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
            let line = line.context("Failed to read a line of checker's response")?;
            serde_json::from_str(&line).context("Failed to deserialize checker's response")?
        } else {
            return db::mark_dead(conn, target);
        }
    };

    match state {
        ipc::CheckerResponse::Peer { peer: _ } => {
            db::mark_dead(conn, target)?;
            bail!("Expected the checker to respond with State, but it responded with Peer");
        }
        ipc::CheckerResponse::State { state } => match state {
            ipc::InstanceState::Alive => {
                db::mark_alive(conn, target)?;
                process_peers(logger, conn, target, lines)?;
            }
            ipc::InstanceState::Moving { to } => {
                println!("{} is moving to {}", target, to);
            }
            ipc::InstanceState::Moved { to } => {
                println!("{} has moved to {}", target, to);
                db::mark_moved(conn, target, &to)?;
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
                db::add_instance(conn, &peer)?;
                peers_count += 1;
            }
        }
    }

    println!("{} has {} peers", target, peers_count);

    Ok(())
}
