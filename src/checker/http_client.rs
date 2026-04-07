//! HTTP client that automatically checks requests against robots.txt.
use crate::checker::http_fetcher::{HttpFetcher, HttpFetcherError, IHttpFetcher, Redirection};
use slog::{Logger, info};
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

pub struct HttpClient<FetcherT: IHttpFetcher = HttpFetcher> {
    fetcher: FetcherT,
    robots_txt: String,
}

impl<FetcherT: IHttpFetcher> HttpClient<FetcherT> {
    pub fn new(logger: Logger, host: Host) -> Result<Self, HttpClientError> {
        let fetcher = FetcherT::new(logger.clone());
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
