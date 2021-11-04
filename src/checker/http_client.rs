//! HTTP client that automatically checks requests against robots.txt.
use anyhow::Context;
use futures::future::{self, TryFutureExt};
use reqwest::Client;
use slog::{error, Logger};
use std::future::Future;
use std::time::Duration;
use url::{Host, Url};

#[derive(Debug)]
pub enum HttpClientError {
    /// The URL couldn't be accessed because the access is forbidden by robots.txt.
    ForbiddenByRobotsTxt(Url),

    /// Error returned by the reqwest crate.
    ReqwestError(reqwest::Error),
}

impl std::fmt::Display for HttpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpClientError::ForbiddenByRobotsTxt(url) => {
                write!(f, "robots.txt forbids access to {}", url)
            }
            HttpClientError::ReqwestError(err) => write!(f, "reqwest's crate error: {}", err),
        }
    }
}

impl std::error::Error for HttpClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HttpClientError::ForbiddenByRobotsTxt(_) => None,
            HttpClientError::ReqwestError(err) => err.source(),
        }
    }
}

pub struct HttpClient {
    inner: Client,
    robots_txt: String,
}

impl HttpClient {
    pub async fn new(logger: &Logger, host: &Host) -> anyhow::Result<Self> {
        // Accept no more than 5 redirects, and only allow redirects to current domain and its
        // subdomains
        let redirect_policy = {
            let logger = logger.clone();
            reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() > 5 {
                    error!(logger, "Too many redirects: {:?}", attempt.previous());
                    attempt.error("too many redirects")
                } else if !is_same_origin(attempt.url(), &attempt.previous()[0]) {
                    error!(
                        logger,
                        "Redirect points to {} which is of different origin than {}",
                        attempt.url(),
                        &attempt.previous()[0]
                    );
                    attempt.error("redirected outside of current origin")
                } else {
                    attempt.follow()
                }
            })
        };
        let inner = reqwest::ClientBuilder::new()
            // TODO: set a User Agent with a URL that describes the bot
            .redirect(redirect_policy)
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to prepare a reqwest client")?;
        let robots_txt = {
            let url = format!("https://{}/robots.txt", host);
            inner
                .get(url)
                .timeout(Duration::from_secs(10))
                .send()
                .await?
                .text()
                .await?
        };
        Ok(Self { inner, robots_txt })
    }

    pub fn get<'a, U: reqwest::IntoUrl + 'a>(
        &'a self,
        url: U,
    ) -> impl Future<Output = Result<reqwest::Response, HttpClientError>> + 'a {
        async { url.into_url().map_err(HttpClientError::ReqwestError) }
            .and_then(|url| {
                if !self.allowed_by_robots_txt(url.as_str()) {
                    future::err(HttpClientError::ForbiddenByRobotsTxt(url))
                } else {
                    future::ok(url)
                }
            })
            .and_then(|url| {
                self.inner
                    .get(url)
                    .header(
                        reqwest::header::ACCEPT,
                        reqwest::header::HeaderValue::from_static("application/json"),
                    )
                    .timeout(Duration::from_secs(10))
                    .send()
                    .map_err(HttpClientError::ReqwestError)
            })
    }

    fn allowed_by_robots_txt(&self, url: &str) -> bool {
        const USER_AGENT: &str = "";

        use robotstxt::DefaultMatcher;
        let mut matcher = DefaultMatcher::default();
        matcher.one_agent_allowed_by_robots(&self.robots_txt, USER_AGENT, url)
    }
}

/// Returns `true` if all of the following holds:
/// - the URLs have the same schema
/// - the URLs use the same domain, or one domain is a sub-domain of another
/// - the URLs use the same port (if any; port can be implied by the schema)
fn is_same_origin(lhs: &Url, rhs: &Url) -> bool {
    use url::{Host::*, Origin::*};

    match (lhs.origin(), rhs.origin()) {
        (Opaque(lhs), Opaque(rhs)) => lhs == rhs,
        (Opaque(_), _) => false,
        (_, Opaque(_)) => false,
        (Tuple(lhs_schema, lhs_host, lhs_port), Tuple(rhs_schema, rhs_host, rhs_port)) => {
            let same_host = match (lhs_host, rhs_host) {
                (Domain(lhs), Domain(rhs)) => lhs
                    .rsplit('.')
                    .zip(rhs.rsplit('.'))
                    .all(|(lhs, rhs)| lhs == rhs),
                (Ipv4(lhs), Ipv4(rhs)) => lhs == rhs,
                (Ipv6(lhs), Ipv6(rhs)) => lhs == rhs,
                _ => false,
            };

            lhs_schema == rhs_schema && same_host && lhs_port == rhs_port
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_origin() {
        let http_example_com = Url::parse("http://example.com").unwrap();
        let https_example_com = Url::parse("https://example.com").unwrap();
        let https_foo_example_com = Url::parse("https://foo.example.com").unwrap();
        let https_example_com_443 = Url::parse("https://example.com:443").unwrap();
        let https_example_com_444 = Url::parse("https://example.com:444").unwrap();
        let https_example_org = Url::parse("https://example.org").unwrap();

        assert!(!is_same_origin(&http_example_com, &https_example_com));
        assert!(!is_same_origin(&https_example_com, &http_example_com));
        assert!(!is_same_origin(&https_example_com, &https_example_org));
        assert!(!is_same_origin(&https_example_com, &https_example_com_444));

        assert!(is_same_origin(&https_example_com, &https_example_com));
        assert!(is_same_origin(&https_example_com, &https_foo_example_com));
        assert!(is_same_origin(&https_foo_example_com, &https_example_com));

        assert!(is_same_origin(&https_example_com, &https_example_com_443));
    }
}
