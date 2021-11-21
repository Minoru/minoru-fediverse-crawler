//! HTTP client that automatically checks requests against robots.txt.
use futures::future::{self, TryFutureExt};
use reqwest::Client;
use slog::{error, info, Logger};
use std::future::Future;
use std::time::Duration;
use url::{Host, Url};

/// The string to be matched against "User-agent" in robots.txt
const USER_AGENT_TOKEN: &str = "MinoruFediverseCrawler";

/// The string to be sent with each HTTP request.
const USER_AGENT_FULL: &str = "Minoru's Fediverse Crawler (+https://nodes.fediverse.party)";

#[derive(Debug)]
pub enum HttpClientError {
    /// The URL couldn't be accessed because the access is forbidden by robots.txt.
    ForbiddenByRobotsTxt(Url),

    /// The URL is temporarily redirected to another.
    Moving { from: Url, to: Url },

    /// The URL is permanently redirected to another.
    Moved { from: Url, to: Url },

    /// The URL is redirected, but we don't know where (response lacked a `Location` header).
    NoLocationHeader(Url),

    /// Error returned by the reqwest crate.
    ReqwestError(reqwest::Error),
}

impl std::fmt::Display for HttpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpClientError::ForbiddenByRobotsTxt(url) => {
                write!(f, "robots.txt forbids access to {}", url)
            }
            HttpClientError::Moving { from, to } => {
                write!(f, "{} is temporarily redirected to {}", from, to)
            }
            HttpClientError::Moved { from, to } => {
                write!(f, "{} is permanently redirected to {}", from, to)
            }
            HttpClientError::NoLocationHeader(from) => {
                write!(f, "{} is redirected, but we don't know where as `Location` header was missing or invalid", from)
            }
            HttpClientError::ReqwestError(err) => write!(f, "reqwest's crate error: {}", err),
        }
    }
}

impl std::error::Error for HttpClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HttpClientError::ForbiddenByRobotsTxt(_) => None,
            HttpClientError::Moving { .. } => None,
            HttpClientError::Moved { .. } => None,
            HttpClientError::NoLocationHeader(_) => None,
            HttpClientError::ReqwestError(err) => err.source(),
        }
    }
}

pub struct HttpClient {
    inner: Client,
    robots_txt: String,
}

impl HttpClient {
    pub async fn new(logger: Logger, host: &Host) -> Result<Self, HttpClientError> {
        // Our redirect policy is:
        // - follow redirects as long as they point to the same hostname:port, and schema didn't
        //   change
        // - stop after 10 redirects
        let redirect_policy = {
            let logger = logger.clone();
            reqwest::redirect::Policy::custom(move |attempt| {
                // This can't panic because in order to get redirected, we had to request some URL. So
                // there's at least one previously-visited URL in the array.
                #[allow(clippy::indexing_slicing)]
                let previous: &Url = &attempt.previous()[0];

                if attempt.previous().len() > 10 {
                    error!(logger, "Too many redirects: {:?}", attempt.previous());
                    attempt.error("too many redirects")
                } else if !is_same_origin(attempt.url(), previous) {
                    error!(
                        logger,
                        "Redirect points to {} which is of different origin that {}; stopping here",
                        attempt.url(),
                        previous
                    );
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            })
        };
        let inner = reqwest::ClientBuilder::new()
            .redirect(redirect_policy)
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT_FULL)
            .build()
            .map_err(HttpClientError::ReqwestError)?;
        let robots_txt = {
            let url = format!("https://{}/robots.txt", host);
            info!(logger, "Fetching robots.txt");
            let response = inner
                .get(url)
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .map_err(HttpClientError::ReqwestError)?;
            redirect_into_error(&response)?;
            response
                .text()
                .await
                .map_err(HttpClientError::ReqwestError)?
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
            .and_then(|response| async {
                redirect_into_error(&response)?;
                Ok(response)
            })
    }

    fn allowed_by_robots_txt(&self, url: &str) -> bool {
        use robotstxt::DefaultMatcher;
        let mut matcher = DefaultMatcher::default();
        matcher.one_agent_allowed_by_robots(&self.robots_txt, USER_AGENT_TOKEN, url)
    }
}

fn redirect_into_error(response: &reqwest::Response) -> Result<(), HttpClientError> {
    use reqwest::{header::LOCATION, StatusCode};

    match response.status() {
        StatusCode::FOUND | StatusCode::SEE_OTHER | StatusCode::TEMPORARY_REDIRECT => {
            let from = response.url().clone();

            let to = response
                .headers()
                .get(LOCATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| Url::parse(h).ok())
                .ok_or_else(|| HttpClientError::NoLocationHeader(from.clone()))?;

            return Err(HttpClientError::Moving { from, to });
        }

        StatusCode::MOVED_PERMANENTLY | StatusCode::PERMANENT_REDIRECT => {
            let from = response.url().clone();

            let to = response
                .headers()
                .get(LOCATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| Url::parse(h).ok())
                .ok_or_else(|| HttpClientError::NoLocationHeader(from.clone()))?;

            return Err(HttpClientError::Moved { from, to });
        }

        _ => {}
    }
    Ok(())
}

/// Returns `true` if the URLs have the same schema, domain, and port.
fn is_same_origin(lhs: &Url, rhs: &Url) -> bool {
    lhs.origin() == rhs.origin()
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
        assert!(!is_same_origin(&https_example_com, &https_foo_example_com));
        assert!(!is_same_origin(&https_foo_example_com, &https_example_com));

        assert!(is_same_origin(&https_example_com, &https_example_com));
        assert!(is_same_origin(&https_example_com, &https_example_com_443));
    }
}
