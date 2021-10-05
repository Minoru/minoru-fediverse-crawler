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

fn orchestrator_main() {
    println!("The orchestrator welcomes you!");
}

fn checker_main(host: String) {
    println!("Checking Fediverse instance at {}", host);
}

fn main() -> Result<(), lexopt::Error> {
    let args = parse_args()?;
    match args.host_to_check {
        None => orchestrator_main(),
        Some(host) => checker_main(host),
    }
    Ok(())
}
