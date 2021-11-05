//! HTTP client that automatically checks requests against robots.txt.
use futures::future::{self, TryFutureExt};
use reqwest::Client;
use std::future::Future;
use std::time::Duration;
use url::{Host, Url};

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
    pub async fn new(host: &Host) -> Result<Self, HttpClientError> {
        let inner = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(HttpClientError::ReqwestError)?;
        let robots_txt = {
            let url = format!("https://{}/robots.txt", host);
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
        const USER_AGENT: &str = "";

        use robotstxt::DefaultMatcher;
        let mut matcher = DefaultMatcher::default();
        matcher.one_agent_allowed_by_robots(&self.robots_txt, USER_AGENT, url)
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
