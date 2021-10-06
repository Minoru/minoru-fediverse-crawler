mod checker;
mod orchestrator;

struct Args {
    host_to_check: Option<String>,
}

fn parse_args() -> Result<Args, lexopt::Error> {
    use lexopt::prelude::*;

    let mut host_to_check = None;
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("check") => {
                host_to_check = Some(parser.value()?.into_string()?);
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Args { host_to_check })
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    match args.host_to_check {
        None => orchestrator::main(),
        Some(host) => checker::main(host),
    }
    Ok(())
}
