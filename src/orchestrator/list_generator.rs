//! Produce a JSON list of alive instances.
use crate::{db, with_loc};
use anyhow::Context;
use slog::{info, Logger};
use std::io::Write;

/// Writes a JSON array of alive instances into _instances.json_.
pub fn generate(logger: Logger) -> anyhow::Result<()> {
    info!(logger, "Generating a list of instances");

    let mut instances: Vec<String> = vec![];

    let conn = db::open()?;
    let mut statement = conn
        .prepare(
            "SELECT hostname
            FROM instances
                JOIN hidden_instances ON instances.id = hidden_instances.instance
            WHERE state = 1
                AND hide_from_list = 0

            UNION

            SELECT hostname
            FROM instances
                JOIN dying_state_data ON instances.id = dying_state_data.instance
                JOIN hidden_instances ON instances.id = hidden_instances.instance
            WHERE state = 2
                AND previous_state = 1
                AND hide_from_list = 0",
        )
        .context(with_loc!("Preparing a SELECT"))?;
    let mut ids = statement.query([])?;
    while let Some(row) = ids.next()? {
        let hostname: String = row.get(0).context(with_loc!("Getting `hostname`"))?;
        instances.push(hostname);
    }

    let instances = json::stringify(instances);

    let mut file = tempfile::NamedTempFile::new_in(".")
        .context(with_loc!("Creating a temporary file in current directory"))?;
    file.write_all(instances.as_bytes())
        .context(with_loc!("Writing instances into a temporary file"))?;
    file.persist("instances.json")
        .context(with_loc!("Renaming temporary file to 'instances.json'"))?;

    Ok(())
}
