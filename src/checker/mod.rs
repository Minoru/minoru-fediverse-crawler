mod http_client;

use crate::{
    checker::http_client::{HttpClient, HttpClientError},
    ipc, with_loc,
};
use anyhow::{anyhow, Context};
use serde::Deserialize;
use slog::{error, info, o, Logger};
use url::{Host, Url};

#[derive(Debug)]
struct UreqHttpStatusError {
    status: u16,
}

impl std::fmt::Display for UreqHttpStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "HTTP error {}", self.status)
    }
}

impl std::error::Error for UreqHttpStatusError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

/// Turns a reference to a response into an error if the server returned an HTTP error.
///
/// This mimics `reqwest::Response::error_for_status_ref()`.
fn error_for_status_ref(response: &ureq::Response) -> Result<&ureq::Response, UreqHttpStatusError> {
    let status = response.status();

    let is_client_error = (400..500).contains(&status);
    let is_server_error = (500..600).contains(&status);

    if is_client_error || is_server_error {
        Err(UreqHttpStatusError { status })
    } else {
        Ok(response)
    }
}

pub fn main(logger: Logger, host: Host) -> anyhow::Result<()> {
    let logger = logger.new(o!("host" => host.to_string()));
    info!(logger, "Started the checker");

    // Here we handle results of redirects. If we don't call `println!` here, the Orchestrator will
    // mark the host as dead.
    if let Err(e) = try_check(&logger, host) {
        if let Some(error) = e.downcast_ref::<HttpClientError>() {
            match error {
                HttpClientError::Moving { to, .. } => {
                    if let Some(to) = to.host().map(|h| h.to_owned()) {
                        info!(logger, "Instance is moving to {}", to);
                        let moving = serde_json::to_string(&ipc::CheckerResponse::State {
                            state: ipc::InstanceState::Moving { to },
                        })
                        .context(with_loc!("Serializing Moving message"))?;
                        println!("{}", moving);
                    }
                }

                HttpClientError::Moved { to, .. } => {
                    if let Some(to) = to.host().map(|h| h.to_owned()) {
                        info!(logger, "Instance has moved to {}", to);
                        let moved = serde_json::to_string(&ipc::CheckerResponse::State {
                            state: ipc::InstanceState::Moved { to },
                        })
                        .context(with_loc!("Serializing Moved message"))?;
                        println!("{}", moved);
                    }
                }

                // Propagate all other errors upwards. A lack of response from the checker will
                // make the orchestrator to mark this host as dead.
                _ => {
                    error!(logger, "The instance is dead: {:?}", error);
                }
            }
        } else {
            error!(
                logger,
                "Couldn't downcast the error to HttpClientError: {:?}", e
            );
        }

        return Err(e);
    }

    info!(logger, "Check finished");

    Ok(())
}

fn try_check(logger: &Logger, host: Host) -> anyhow::Result<()> {
    let client = HttpClient::new(logger.clone(), host.clone())
        .context(with_loc!("Initializing HTTP client"))?;

    let software = get_software(logger, &client, &host)
        .context(with_loc!("Determining instance's software"))?;
    info!(logger, "{} runs {}", host, software);

    let hide_from_list = {
        match is_instance_private(&client, &host, &software) {
            Ok(result) => result,
            Err(e) => {
                info!(logger, "Couldn't check if instance is private: {}", e);
                false
            }
        }
    };
    let alive = serde_json::to_string(&ipc::CheckerResponse::State {
        state: ipc::InstanceState::Alive { hide_from_list },
    })
    .context(with_loc!("Serializing Alive message"))?;
    info!(logger, "The instance is alive");
    println!("{}", alive);

    let peers = get_peers(logger, &client, &host, &software)
        .context(with_loc!("Fetching instance's peers list"))?;
    info!(logger, "{} has {} peers", host, peers.len());
    for instance in peers {
        let peer = serde_json::to_string(&ipc::CheckerResponse::Peer { peer: instance })
            .context(with_loc!("Serializing Peer message"))?;
        println!("{}", peer);
    }

    Ok(())
}

fn get_software(logger: &Logger, client: &HttpClient, host: &Host) -> anyhow::Result<String> {
    let nodeinfo = fetch_nodeinfo(logger, client, host).context(with_loc!("Fetching NodeInfo"))?;
    json::parse(&nodeinfo)
        .map(|obj| {
            // Indexing into JsonValue doesn't panic
            #[allow(clippy::indexing_slicing)]
            obj["software"]["name"].to_string()
        })
        .map_err(|err| {
            let msg = format!(
                "Failed to figure out the software name from the NodeInfo {}: {}",
                nodeinfo, err
            );
            error!(logger, "{}", &msg; "json_error" => err.to_string());
            anyhow!(msg)
        })
        .context(with_loc!("Extracting software make from NodeInfo"))
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NodeInfoPointerRaw {
    Bare { links: NodeInfoPointerLink },
    Array { links: Vec<NodeInfoPointerLink> },
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "NodeInfoPointerRaw")]
struct NodeInfoPointer {
    links: Vec<NodeInfoPointerLink>,
}

impl From<NodeInfoPointerRaw> for NodeInfoPointer {
    fn from(input: NodeInfoPointerRaw) -> NodeInfoPointer {
        match input {
            NodeInfoPointerRaw::Bare { links } => Self { links: vec![links] },
            NodeInfoPointerRaw::Array { links } => Self { links },
        }
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
struct NodeInfoPointerLink {
    rel: String,
    href: String,
}

fn fetch_nodeinfo(logger: &Logger, client: &HttpClient, host: &Host) -> anyhow::Result<String> {
    let pointer = fetch_nodeinfo_pointer(logger, client, host)
        .context(with_loc!("Fetching NodeInfo well-known document"))?;
    let url = pick_highest_supported_nodeinfo_version(&pointer).context(with_loc!(
        "Picking the highest supported NodeInfo version out of JRD document"
    ))?;
    fetch_nodeinfo_document(logger, client, &url).context(with_loc!("Fetching NodeInfo document"))
}

fn fetch_nodeinfo_pointer(
    logger: &Logger,
    client: &HttpClient,
    host: &Host,
) -> anyhow::Result<NodeInfoPointer> {
    let url = format!("https://{}/.well-known/nodeinfo", host);
    let url = Url::parse(&url).context(with_loc!(
        "Formatting URL of the well-known NodeInfo document"
    ))?;
    let response = client
        .get(&url)
        .context(with_loc!("Fetching the well-known NodeInfo document"))?;
    error_for_status_ref(&response).map_err(|err| {
        error!(
            logger, "Failed to fetch the well-known NodeInfo document: {}", err;
            "http_error" => err.to_string(), "url" => url.to_string());
        err
    })?;

    response
        .into_json::<NodeInfoPointer>()
        .context(with_loc!("Decoding NodeInfo pointer as JSON"))
}

fn pick_highest_supported_nodeinfo_version(pointer: &NodeInfoPointer) -> anyhow::Result<Url> {
    // This array in the ascending order of schema versions.
    const SUPPORTED_NODEINFO_SCHEMAS: [&str; 4] = [
        "http://nodeinfo.diaspora.software/ns/schema/1.0",
        "http://nodeinfo.diaspora.software/ns/schema/1.1",
        "http://nodeinfo.diaspora.software/ns/schema/2.0",
        "http://nodeinfo.diaspora.software/ns/schema/2.1",
    ];
    pointer
        .links
        .iter()
        .filter_map(|link| {
            SUPPORTED_NODEINFO_SCHEMAS
                .iter()
                .position(|el| el == &link.rel)
                .map(|priority| (priority, link))
        })
        .max_by_key(|result| result.0)
        .map(|result| &result.1.href)
        .ok_or_else(|| {
            anyhow!(
                "Failed to extract highest supported NodeInfo version's URL from {:?}",
                pointer.links
            )
        })
        .and_then(|u| Url::parse(u).context(with_loc!("Parsing NodeInfo href as Url")))
        .context(with_loc!("Picking highest supported NodeInfo version"))
}

fn fetch_nodeinfo_document(
    logger: &Logger,
    client: &HttpClient,
    url: &Url,
) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .context(with_loc!("Fetching NodeInfo document"))?;
    error_for_status_ref(&response).map_err(|err| {
        error!(
            logger, "Failed to fetch NodeInfo: {}", err;
            "http_error" => err.to_string(), "url" => url.to_string());
        err
    })?;

    response
        .into_string()
        .context(with_loc!("Getting NodeInfo document's body"))
}

fn get_peers(
    logger: &Logger,
    client: &HttpClient,
    host: &Host,
    software: &str,
) -> anyhow::Result<Vec<Host>> {
    match software {
        "mastodon" | "pleroma" | "misskey" | "bookwyrm" | "smithereen" => {
            get_peers_mastodonish(logger, client, host)
                .context(with_loc!("Fetching peers list via Mastodon-ish API"))
        }
        _ => Ok(vec![]),
    }
}

fn get_peers_mastodonish(
    logger: &Logger,
    client: &HttpClient,
    host: &Host,
) -> anyhow::Result<Vec<Host>> {
    let url = format!("https://{}/api/v1/instance/peers", host);
    let url = Url::parse(&url).context(with_loc!(
        "Formatting URL of the Mastodon-ish 'peers' endpoint"
    ))?;
    let response = client
        .get(&url)
        .context(with_loc!("Fetching Mastodon-ish peers list"))?;
    error_for_status_ref(&response).map_err(|err| {
        error!(
            logger, "Failed to fetch Mastodon-ish peers: {}", err;
            "http_error" => err.to_string(), "url" => url.to_string());
        err
    })?;

    Ok(response
        .into_json::<Vec<String>>()
        .context(with_loc!("Parsing Mastodon-ish peers list as JSON"))?
        .into_iter()
        .map(Host::Domain)
        .collect())
}

fn is_instance_private(client: &HttpClient, host: &Host, software: &str) -> anyhow::Result<bool> {
    match software {
        "gnusocial" | "friendica" => {
            let config = get_statusnet_config(client, host)
                .context(with_loc!("Fetching StatusNet config"))?;
            let config =
                json::parse(&config).context(with_loc!("Parsing StatusNet config as JSON"))?;

            // Indexing into JsonValue doesn't panic
            #[allow(clippy::indexing_slicing)]
            let is_private = config["site"]["private"].as_bool().unwrap_or(false);

            Ok(is_private)
        }

        "hubzilla" | "red" => {
            let siteinfo =
                get_siteinfo(client, host).context(with_loc!("Fetching Siteinfo.json"))?;
            let siteinfo = json::parse(&siteinfo).context(with_loc!("Parsing Siteinfo as JSON"))?;

            // Indexing into JsonValue doesn't panic
            #[allow(clippy::indexing_slicing)]
            let hide_in_statistics = siteinfo["hide_in_statistics"].as_bool().unwrap_or(false);

            Ok(hide_in_statistics)
        }

        _ => Ok(false),
    }
}

fn get_statusnet_config(client: &HttpClient, host: &Host) -> anyhow::Result<String> {
    let url = format!("https://{}/api/statusnet/config.json", host);
    let url = Url::parse(&url).context(with_loc!("Formatting URL StatusNet config"))?;
    let response = client
        .get(&url)
        .context(with_loc!("Requesting StatusNet config.json"))?
        .into_string()
        .context(with_loc!("Getting a body of config.json response"))?;
    Ok(response)
}

fn get_siteinfo(client: &HttpClient, host: &Host) -> anyhow::Result<String> {
    let url = format!("https://{}/siteinfo.json", host);
    let url = Url::parse(&url).context(with_loc!("Formatting URL of siteinfo document"))?;
    let response = client
        .get(&url)
        .context(with_loc!("Requesting siteinfo.json"))?
        .into_string()
        .context(with_loc!("Getting a body of siteinfo.json response"))?;
    Ok(response)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod test {
    use super::*;

    #[test]
    fn picks_highest_nodeinfo_version() {
        assert!(
            pick_highest_supported_nodeinfo_version(&NodeInfoPointer { links: vec![] }).is_err()
        );

        assert!(pick_highest_supported_nodeinfo_version(&NodeInfoPointer {
            links: vec![NodeInfoPointerLink {
                rel: "http://nodeinfo.diaspora.software/ns/schema/2.2".to_string(),
                href: "https://example.com/first".to_string()
            }],
        })
        .is_err());

        assert_eq!(
            pick_highest_supported_nodeinfo_version(&NodeInfoPointer {
                links: vec![NodeInfoPointerLink {
                    rel: "http://nodeinfo.diaspora.software/ns/schema/1.0".to_string(),
                    href: "https://example.com/first".to_string()
                }],
            })
            .unwrap(),
            Url::parse("https://example.com/first").unwrap()
        );

        assert_eq!(
            pick_highest_supported_nodeinfo_version(&NodeInfoPointer {
                links: vec![
                    NodeInfoPointerLink {
                        rel: "http://nodeinfo.diaspora.software/ns/schema/1.0".to_string(),
                        href: "https://example.org/first".into()
                    },
                    NodeInfoPointerLink {
                        rel: "http://nodeinfo.diaspora.software/ns/schema/2.1".to_string(),
                        href: "https://example.com/2.1".into()
                    }
                ],
            })
            .unwrap(),
            Url::parse("https://example.com/2.1").unwrap()
        );

        assert_eq!(
            pick_highest_supported_nodeinfo_version(&NodeInfoPointer {
                links: vec![
                    NodeInfoPointerLink {
                        rel: "http://nodeinfo.diaspora.software/ns/schema/2.0".to_string(),
                        href: "http://example.org/highest is the first".to_string()
                    },
                    NodeInfoPointerLink {
                        rel: "http://nodeinfo.diaspora.software/ns/schema/1.1".to_string(),
                        href: "http://example.org/lowest is the second".to_string()
                    }
                ],
            })
            .unwrap(),
            Url::parse("http://example.org/highest is the first").unwrap()
        );
    }

    #[test]
    fn broken_lemmy_nodeinfo_pointer() {
        let input = r#"{"links":{"rel":"http://nodeinfo.diaspora.software/ns/schema/2.0","href":"https://lemmy.ml/nodeinfo/2.0.json"}}"#;
        let parsed: NodeInfoPointer = serde_json::from_str(input).expect("Failed to parse");
        let expected = NodeInfoPointer {
            links: vec![NodeInfoPointerLink {
                rel: "http://nodeinfo.diaspora.software/ns/schema/2.0".to_string(),
                href: "https://lemmy.ml/nodeinfo/2.0.json".to_string(),
            }],
        };
        assert_eq!(expected, parsed);
    }
}
