use crate::ipc;
use anyhow::{anyhow, bail, Context};
use slog::Logger;
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

pub fn main(_logger: Logger) -> anyhow::Result<()> {
    run_checker("mastodon.social")
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
        ipc::CheckerResponse::Peer { hostname: _ } => {
            bail!("Expected the checker to respond with State, but it responded with Peer");
        }
        ipc::CheckerResponse::State { state } => match state {
            ipc::InstanceState::Alive => process_peers(target, lines)?,
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
            ipc::CheckerResponse::Peer { hostname: _ } => peers_count += 1,
        }
    }

    println!("{} has {} peers", target, peers_count);

    Ok(())
}
