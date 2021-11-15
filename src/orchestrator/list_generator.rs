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
                AND hide_from_list = 0

            UNION

            SELECT instances.hostname
            FROM instances
                JOIN moving_state_data ON instances.id = moving_state_data.instance
                JOIN hidden_instances ON instances.id = hidden_instances.instance
                JOIN instances AS moved_to_instance ON moving_state_data.moving_to = moved_to_instance.id
            WHERE instances.state = 4
                AND previous_state = 1
                AND moved_to_instance.state != 1
                AND hide_from_list = 0",
        )
        .context(with_loc!("Preparing a SELECT"))?;
    let mut ids = statement.query([])?;
    while let Some(row) = ids.next()? {
        let hostname: String = row.get(0).context(with_loc!("Getting `hostname`"))?;
        instances.push(hostname);
    }

    let instances = json::stringify(instances);
    write("instances.json", instances.as_bytes()).context(with_loc!("Writing instances.json"))?;

    let gzipped_instances = {
        use flate2::{write::GzEncoder, Compression};

        let mut e = GzEncoder::new(Vec::new(), Compression::best());
        e.write_all(instances.as_bytes())
            .context(with_loc!("Compressing instances list"))?;
        e.finish().context(with_loc!("Finishing gzip stream"))?
    };
    write("instances.json.gz", &gzipped_instances)
        .context(with_loc!("Writing instances.json.gz"))?;

    Ok(())
}

fn write(filename: &str, data: &[u8]) -> anyhow::Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(".")
        .context(with_loc!("Creating a temporary file in current directory"))?;
    file.write_all(data)
        .context(with_loc!("Writing data into a temporary file"))?;

    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file
            .as_file()
            .metadata()
            .context(with_loc!("Getting metadata of the temporary file"))?
            .permissions();
        perms.set_mode(0o644);
        file.as_file()
            .set_permissions(perms)
            .context(with_loc!("Setting permissions for the temporary file"))?;
    }

    file.persist(filename)
        .context(with_loc!("Renaming temporary file to the desired filename"))?;
    Ok(())
}
