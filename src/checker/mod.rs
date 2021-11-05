mod http_client;

use crate::{
    checker::http_client::{HttpClient, HttpClientError},
    ipc, with_loc,
};
use anyhow::{anyhow, Context};
use serde::Deserialize;
use slog::{error, info, o, Logger};
use tokio::runtime::Runtime;
use url::{Host, Url};

pub fn main(logger: Logger, host: Host) -> anyhow::Result<()> {
    let rt = Runtime::new()?;
    info!(logger, "Started Tokio runtime");

    let logger = logger.new(o!("host" => host.to_string()));
    rt.block_on(async_main(&logger, &host))
}

async fn async_main(logger: &Logger, host: &Host) -> anyhow::Result<()> {
    info!(logger, "Started the checker");

    // Here we handle results of redirects. If we don't call `println!` here, the Orchestrator will
    // mark the host as dead.
    if let Err(e) = try_check(logger, host).await {
        if let Some(error) = e.downcast_ref::<HttpClientError>() {
            match error {
                HttpClientError::Moving { to, .. } => {
                    if let Some(to) = to.host().map(|h| h.to_owned()) {
                        let moving = serde_json::to_string(&ipc::CheckerResponse::State {
                            state: ipc::InstanceState::Moving { to },
                        })?;
                        println!("{}", moving);
                    }
                }

                HttpClientError::Moved { to, .. } => {
                    if let Some(to) = to.host().map(|h| h.to_owned()) {
                        let moved = serde_json::to_string(&ipc::CheckerResponse::State {
                            state: ipc::InstanceState::Moved { to },
                        })?;
                        println!("{}", moved);
                    }
                }

                // Propagate all other errors upwards. A lack of response from the checker will
                // make the orchestrator to mark this host as dead.
                _ => {}
            }
        }

        return Err(e);
    }

    Ok(())
}

async fn try_check(logger: &Logger, host: &Host) -> anyhow::Result<()> {
    let client = HttpClient::new(host).await?;

    let software = get_software(logger, &client, host).await?;
    info!(logger, "{} runs {}", host, software);

    let hide_from_list = {
        match is_instance_private(&client, host, &software).await {
            Ok(result) => result,
            Err(e) => {
                info!(logger, "Couldn't check if instance is private: {}", e);
                false
            }
        }
    };
    let alive = serde_json::to_string(&ipc::CheckerResponse::State {
        state: ipc::InstanceState::Alive { hide_from_list },
    })?;
    println!("{}", alive);

    let peers = get_peers(logger, &client, host, &software).await?;
    info!(logger, "{} has {} peers", host, peers.len());
    for instance in peers {
        let peer = serde_json::to_string(&ipc::CheckerResponse::Peer { peer: instance })?;
        println!("{}", peer);
    }

    Ok(())
}

async fn get_software(logger: &Logger, client: &HttpClient, host: &Host) -> anyhow::Result<String> {
    let nodeinfo = fetch_nodeinfo(logger, client, host).await?;
    json::parse(&nodeinfo)
        .map(|obj| obj["software"]["name"].to_string())
        .map_err(|err| {
            let msg = format!(
                "Failed to figure out the software name from the NodeInfo {}: {}",
                nodeinfo, err
            );
            error!(logger, "{}", &msg; "json_error" => err.to_string());
            anyhow!(msg)
        })
}

#[derive(Debug, Deserialize)]
struct NodeInfoPointer {
    links: Vec<NodeInfoPointerLink>,
}

#[derive(Debug, Deserialize)]
struct NodeInfoPointerLink {
    rel: String,
    href: String,
}

async fn fetch_nodeinfo(
    logger: &Logger,
    client: &HttpClient,
    host: &Host,
) -> anyhow::Result<String> {
    let pointer = fetch_nodeinfo_pointer(logger, client, host).await?;
    // TODO: add sanitization step that removes any links that point outside of the current host's
    // domain
    let url = pick_highest_supported_nodeinfo_version(&pointer)?;
    fetch_nodeinfo_document(logger, client, &url).await
}

async fn fetch_nodeinfo_pointer(
    logger: &Logger,
    client: &HttpClient,
    host: &Host,
) -> anyhow::Result<NodeInfoPointer> {
    let url = format!("https://{}/.well-known/nodeinfo", host);
    let response = client.get(&url).await?;
    response.error_for_status_ref().map_err(|err| {
        error!(
            logger, "Failed to fetch the well-known NodeInfo document: {}", err;
            "http_error" => err.to_string(), "url" => url);
        err
    })?;

    // TODO: replace this with a parser that only processes the first few KB of the input
    Ok(response.json::<NodeInfoPointer>().await?)
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
        .and_then(|u| Ok(Url::parse(u)?))
}

async fn fetch_nodeinfo_document(
    logger: &Logger,
    client: &HttpClient,
    url: &Url,
) -> anyhow::Result<String> {
    let response = client.get(url.clone()).await?;
    response.error_for_status_ref().map_err(|err| {
        error!(
            logger, "Failed to fetch NodeInfo: {}", err;
            "http_error" => err.to_string(), "url" => url.to_string());
        err
    })?;

    // TODO: replace this with a parser that only processes the first few KB of the input
    Ok(response.text().await?)
}

async fn get_peers(
    logger: &Logger,
    client: &HttpClient,
    host: &Host,
    software: &str,
) -> anyhow::Result<Vec<Host>> {
    match software {
        "mastodon" | "pleroma" | "misskey" | "bookwyrm" | "smithereen" => {
            get_peers_mastodonish(logger, client, host).await
        }
        _ => Ok(vec![]),
    }
}

async fn get_peers_mastodonish(
    logger: &Logger,
    client: &HttpClient,
    host: &Host,
) -> anyhow::Result<Vec<Host>> {
    let url = format!("https://{}/api/v1/instance/peers", host);
    let response = client.get(&url).await?;
    response.error_for_status_ref().map_err(|err| {
        error!(
            logger, "Failed to fetch Mastodon-ish peers: {}", err;
            "http_error" => err.to_string(), "url" => url);
        err
    })?;

    // TODO: replace this with a parser that only processes the first megabyte of the response
    Ok(response
        .json::<Vec<String>>()
        .await?
        .into_iter()
        .map(Host::Domain)
        .collect())
}

async fn is_instance_private(
    client: &HttpClient,
    host: &Host,
    software: &str,
) -> anyhow::Result<bool> {
    match software {
        "gnusocial" | "friendica" => {
            let config = get_statusnet_config(client, host)
                .await
                .context(with_loc!("Fetching StatusNet config"))?;
            let config =
                json::parse(&config).context(with_loc!("Parsing StatusNet config as JSON"))?;
            let is_private = config["site"]["private"].as_bool().unwrap_or(false);
            Ok(is_private)
        }

        "hubzilla" => {
            let siteinfo = get_siteinfo(client, host)
                .await
                .context(with_loc!("Fetching Siteinfo.json"))?;
            let siteinfo = json::parse(&siteinfo).context(with_loc!("Parsing Siteinfo as JSON"))?;
            let hide_in_statistics = siteinfo["hide_in_statistics"].as_bool().unwrap_or(false);
            Ok(hide_in_statistics)
        }

        _ => Ok(false),
    }
}

async fn get_statusnet_config(client: &HttpClient, host: &Host) -> anyhow::Result<String> {
    let url = format!("https://{}/api/statusnet/config.json", host);
    let response = client
        .get(&url)
        .await
        .context(with_loc!("Requesting StatusNet config.json"))?
        .text()
        .await
        .context(with_loc!("Getting a body of config.json response"))?;
    Ok(response)
}

async fn get_siteinfo(client: &HttpClient, host: &Host) -> anyhow::Result<String> {
    let url = format!("https://{}/siteinfo.json", host);
    let response = client
        .get(&url)
        .await
        .context(with_loc!("Requesting siteinfo.json"))?
        .text()
        .await
        .context(with_loc!("Getting a body of siteinfo.json response"))?;
    Ok(response)
}

#[cfg(test)]
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
}
