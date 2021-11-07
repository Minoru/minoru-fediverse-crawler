use anyhow::{anyhow, bail};
use slog::{error, o, Drain, Logger};
use url::Host;

mod checker;
mod db;
mod instance_adder;
mod ipc;
mod logging_helpers;
mod orchestrator;
mod time;

struct Args {
    add_instances: bool,
    host_to_check: Option<String>,
}

fn parse_args() -> anyhow::Result<Args> {
    use lexopt::prelude::*;

    let mut add_instances = false;
    let mut host_to_check = None;
    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("add-instances") => add_instances = true,
            Long("check") => {
                let value = parser.value()?;
                // .into_string() returns Result<String, OsString> , and OsString can't be
                // converted to anyhow::Error. To fix this, we convert the error into String.
                let value = value
                    .into_string()
                    .map_err(|ostr| anyhow!("{}", ostr.to_string_lossy()))?;
                host_to_check = Some(value);
            }
            _ => return Err(arg.unexpected().into()),
        }
    }

    if add_instances && host_to_check.is_some() {
        bail!("--add-instances and --check are mutually exclusive");
    }

    Ok(Args {
        add_instances,
        host_to_check,
    })
}

fn main() -> anyhow::Result<()> {
    let logger = slog::Logger::root(slog_journald::JournaldDrain.ignore_res(), o!());
    logged_main(logger.clone()).map_err(|err| {
        error!(logger, "{:?}", err);
        err
    })
}

fn logged_main(logger: Logger) -> anyhow::Result<()> {
    let args = parse_args()?;
    if args.add_instances {
        instance_adder::main(logger)
    } else {
        match args.host_to_check {
            None => orchestrator::main(logger),
            Some(host) => {
                let host = Host::parse(&host)?;
                checker::main(logger, host)
            }
        }
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn what_addr_accepts_and_rejects() {
        use addr::parse_domain_name;

        // Full URLs
        assert!(parse_domain_name("http://example.org/hello").is_err());
        assert!(parse_domain_name("http://bar.example.org/goodbye#world").is_err());
        assert!(parse_domain_name("https://example.com/and/a/path?with=option").is_err());
        assert!(parse_domain_name("https://foo.example.com:81/and/a/path?with=option").is_err());

        // IP addresses
        assert!(parse_domain_name("8.8.8.8").is_err());
        assert!(parse_domain_name("127.0.0.1").is_err());
        assert!(parse_domain_name("2001:4860:4860::8888").is_err());
        assert!(parse_domain_name("[2001:4860:4860::8888]").is_err());
        assert!(parse_domain_name("::1").is_err());
        assert!(parse_domain_name("[::1]").is_err());

        // Onion hidden services
        assert!(parse_domain_name("yzw45do3yrjfnbpr.onion")
            .unwrap()
            .is_icann());
        assert!(parse_domain_name(
            "zlzvfg5zcehs2t4qcm7woogyywfzwvrduqujsnehrjeg3tndn6a55nqd.onion"
        )
        .unwrap()
        .is_icann());

        // I2P
        assert!(!parse_domain_name("example.i2p").unwrap().is_icann());

        // OpenNIC
        assert!(!parse_domain_name("outdated.bbs").unwrap().is_icann());
        // This one is dropped from OpenNIC and is coming to "real" DNS soon
        assert!(parse_domain_name("this.one.is.free").unwrap().is_icann());
    }
}
