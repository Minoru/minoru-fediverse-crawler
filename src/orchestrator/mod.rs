use crate::ipc;
use anyhow::{anyhow, bail};
use slog::{error, o, Drain, Logger};
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

pub fn main() -> anyhow::Result<()> {
    let logger = slog::Logger::root(slog_journald::JournaldDrain.ignore_res(), o!());
    run_checker(logger, "mastodon.social")?;
    Ok(())
}

fn run_checker(logger: Logger, target: &str) -> anyhow::Result<()> {
    let exe_path = env::args_os().nth(0).ok_or_else(|| {
        let msg = "Failed to determine the path to the executable";
        error!(logger, "{}", msg);
        anyhow!(msg)
    })?;

    let mut checker = Command::new(exe_path)
        .arg("--check")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            let msg = format!("Failed to spawn a checker: {}", err);
            error!(logger, "{}", &msg);
            anyhow!(msg)
        })?;

    let output = checker.stdout.take().ok_or_else(|| {
        let msg = "Failed to connect to checker's stdout";
        error!(logger, "{}", msg);
        anyhow!(msg)
    })?;
    let reader = BufReader::new(output);
    let mut lines = reader.lines();

    let state = {
        if let Some(line) = lines.next() {
            let line = line.map_err(|err| {
                let msg = format!("Failed to read a line of checker's response: {}", err);
                error!(logger, "{}", &msg);
                anyhow!(msg)
            })?;
            serde_json::from_str(&line).map_err(|err| {
                let msg = format!("failed to deserialize checker's response: {}", err);
                error!(logger, "{}", &msg);
                anyhow!(msg)
            })?
        } else {
            return Ok(());
        }
    };

    match state {
        ipc::CheckerResponse::Peer { hostname: _ } => {
            let msg = "Expected the checker to respond with State, but it responded with Peer";
            error!(logger, "{}", msg);
            bail!(msg);
        }
        ipc::CheckerResponse::State { state } => match state {
            ipc::InstanceState::Alive => process_peers(logger, target, lines)?,
            ipc::InstanceState::Moving { hostname } => {
                println!("{} is moving to {}", target, hostname)
            }
            ipc::InstanceState::Moved { hostname } => {
                println!("{} has moved to {}", target, hostname)
            }
        },
    }

    Ok(())
}

fn process_peers(
    logger: Logger,
    target: &str,
    lines: impl Iterator<Item = std::io::Result<String>>,
) -> anyhow::Result<()> {
    let mut peers_count = 0;
    for response in lines {
        let response = response.map_err(|err| {
            let msg = format!("Failed to read a line of checker's response: {}", err);
            error!(logger, "{}", &msg);
            anyhow!(msg)
        })?;

        let response: ipc::CheckerResponse = serde_json::from_str(&response).map_err(|err| {
            let msg = format!("Failed to deserialize checker's response: {}", err);
            error!(logger, "{}", &msg);
            anyhow!(msg)
        })?;

        match response {
            ipc::CheckerResponse::State { state: _ } => {
                let msg = "Expected the checker to respond with Peer, but it responded with State";
                error!(logger, "{}", msg);
                bail!(msg);
            }
            ipc::CheckerResponse::Peer { hostname: _ } => peers_count += 1,
        }
    }

    println!("{} has {} peers", target, peers_count);

    Ok(())
}
