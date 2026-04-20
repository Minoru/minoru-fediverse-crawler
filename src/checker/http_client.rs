//! HTTP client that automatically checks requests against robots.txt.
use crate::checker::http_fetcher::{HttpFetcher, HttpFetcherError, IHttpFetcher, Redirection};
use slog::{info, Logger};
use url::{Host, Url};

/// The string to be matched against "User-agent" in robots.txt
const USER_AGENT_TOKEN: &str = "MinoruFediverseCrawler";

#[derive(Debug)]
pub enum HttpClientError {
    /// The URL couldn't be accessed because the access is forbidden by robots.txt.
    ForbiddenByRobotsTxt(Url),

    /// The URL is temporarily redirected to another.
    // The fields are put into a box to avoid clippy::result_large_err warning.
    Moving(Box<Redirection>),

    /// The URL is permanently redirected to another.
    // The fields are put into a box to avoid clippy::result_large_err warning.
    Moved(Box<Redirection>),

    /// The URL is redirected, but we don't know where (response lacked a `Location` header).
    NoLocationHeader(Url),

    /// Error returned by the ureq crate.
    // The fields are put into a box to avoid clippy::result_large_err warning.
    UreqError(Box<ureq::Error>),

    /// Std error returned by the ureq crate.
    UreqStdError(std::io::Error),

    /// Error parsing a URL with the `url` crate.
    UrlParseError(url::ParseError),
}

impl std::fmt::Display for HttpClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpClientError::ForbiddenByRobotsTxt(url) => {
                write!(f, "robots.txt forbids access to {url}")
            }
            HttpClientError::Moving(redir) => {
                write!(
                    f,
                    "{} is temporarily redirected to {}",
                    redir.from, redir.to
                )
            }
            HttpClientError::Moved(redir) => {
                write!(
                    f,
                    "{} is permanently redirected to {}",
                    redir.from, redir.to
                )
            }
            HttpClientError::NoLocationHeader(from) => {
                write!(
                    f,
                    "{from} is redirected, but we don't know where as `Location` header was missing or invalid"
                )
            }
            HttpClientError::UreqError(err) => write!(f, "ureq's crate error: {err}"),
            HttpClientError::UreqStdError(err) => {
                write!(f, "ureq's crate produced an std error: {err}")
            }
            HttpClientError::UrlParseError(err) => {
                write!(f, "error parsing URL: {err}")
            }
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
            HttpClientError::UreqError(err) => err.source(),
            HttpClientError::UreqStdError(err) => err.source(),
            HttpClientError::UrlParseError(err) => err.source(),
        }
    }
}

impl From<HttpFetcherError> for HttpClientError {
    fn from(err: HttpFetcherError) -> Self {
        match err {
            HttpFetcherError::Moving(r) => Self::Moving(r),
            HttpFetcherError::Moved(r) => Self::Moved(r),
            HttpFetcherError::NoLocationHeader(u) => Self::NoLocationHeader(u),
            HttpFetcherError::UreqError(e) => Self::UreqError(e),
        }
    }
}

pub struct HttpClient {
    fetcher: Box<dyn IHttpFetcher>,
    robots_txt: String,
}

impl HttpClient {
    pub fn new(logger: Logger, host: Host) -> Result<Self, HttpClientError> {
        let fetcher = Box::new(HttpFetcher::new(logger.clone()));
        Self::with_fetcher(fetcher, logger, host)
    }

    fn with_fetcher(
        fetcher: Box<dyn IHttpFetcher>,
        logger: Logger,
        host: Host,
    ) -> Result<Self, HttpClientError> {
        let robots_txt = {
            let url = format!("https://{host}/robots.txt");
            let url = Url::parse(&url).map_err(HttpClientError::UrlParseError)?;
            info!(logger, "Fetching robots.txt");
            fetcher
                .get(&url, None)
                .map_err(HttpClientError::from)?
                .into_string()
                .map_err(HttpClientError::UreqStdError)?
        };
        Ok(Self {
            fetcher,
            robots_txt,
        })
    }

    pub fn get(&self, url: &Url) -> Result<ureq::Response, HttpClientError> {
        if !self.allowed_by_robots_txt(url.as_str()) {
            return Err(HttpClientError::ForbiddenByRobotsTxt(url.to_owned()));
        }

        match self.fetcher.get(url, Some("application/json")) {
            Ok(r) if r.status() == 404 => {
                let ureq_err = ureq::Error::Status(404, r);
                Err(HttpClientError::UreqError(Box::new(ureq_err)))
            }
            x => x.map_err(HttpClientError::from),
        }
    }

    fn allowed_by_robots_txt(&self, url: &str) -> bool {
        use robotstxt::DefaultMatcher;
        let mut matcher = DefaultMatcher::default();
        matcher.one_agent_allowed_by_robots(&self.robots_txt, USER_AGENT_TOKEN, url)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use crate::checker::http_fetcher::MockIHttpFetcher;

    #[test]
    fn constructor_requests_robots_txt() {
        let host = Host::parse("example.com").unwrap();
        let expected_url = Url::parse("https://example.com/robots.txt").unwrap();

        let mut fetcher = Box::new(MockIHttpFetcher::new());
        fetcher
            .expect_get()
            .with(
                mockall::predicate::eq(expected_url),
                mockall::predicate::always(),
            )
            .once()
            .returning(|_url, _accept_header| {
                let ureq_404 = Box::new(ureq::Error::Status(
                    404,
                    ureq::Response::new(404, "Not found", "").unwrap(),
                ));
                Err(HttpFetcherError::UreqError(ureq_404))
            });

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let _client = HttpClient::with_fetcher(fetcher, logger, host);
    }

    #[test]
    fn constructor_propagates_ureq_error_when_fetching_robots_txt() {
        use crate::checker::http_fetcher::{HttpFetcherError, MockIHttpFetcher};

        let host = Host::parse("example.com").unwrap();
        let expected_url = Url::parse("https://example.com/robots.txt").unwrap();

        let mut fetcher = Box::new(MockIHttpFetcher::new());
        fetcher
            .expect_get()
            .with(
                mockall::predicate::eq(expected_url),
                mockall::predicate::always(),
            )
            .once()
            .returning(|_url, _accept_header| {
                let ureq_err = Box::new(ureq::Error::Status(
                    403,
                    ureq::Response::new(403, "Forbidden", "").unwrap(),
                ));
                Err(HttpFetcherError::UreqError(ureq_err))
            });

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let result = HttpClient::with_fetcher(fetcher, logger, host);

        assert!(matches!(result, Err(HttpClientError::UreqError(_))));
    }

    #[test]
    fn empty_robots_txt_allows_any_request() {
        use crate::checker::http_fetcher::{HttpFetcherError, MockIHttpFetcher};

        let host = Host::parse("example.com").unwrap();
        let robots_url = Url::parse("https://example.com/robots.txt").unwrap();
        let target_url = Url::parse("https://example.com/feed.atom").unwrap();

        let mut fetcher = Box::new(MockIHttpFetcher::new());

        // First call: fetch robots.txt (returns empty string)
        fetcher
            .expect_get()
            .with(
                mockall::predicate::eq(robots_url),
                mockall::predicate::always(),
            )
            .once()
            .returning(|_url, _accept_header| Ok(ureq::Response::new(200, "OK", "").unwrap()));

        // Second call: fetch the target URL (returns 404)
        fetcher
            .expect_get()
            .with(
                mockall::predicate::eq(target_url.clone()),
                mockall::predicate::always(),
            )
            .once()
            .returning(|_url, _accept_header| {
                let ureq_404 = Box::new(ureq::Error::Status(
                    404,
                    ureq::Response::new(404, "Not found", "").unwrap(),
                ));
                Err(HttpFetcherError::UreqError(ureq_404))
            });

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let client = HttpClient::with_fetcher(fetcher, logger.clone(), host).unwrap();

        // The request should not be forbidden by robots.txt (error should be UreqError, not ForbiddenByRobotsTxt)
        let result = client.get(&target_url);
        assert!(matches!(result, Err(HttpClientError::UreqError(_))));
    }
}
