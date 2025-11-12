//! HTTP client that automatically checks requests against robots.txt.
use slog::{Logger, error, info};
use std::time::Duration;
use ureq::Agent;
use url::{Host, Url};

/// The string to be matched against "User-agent" in robots.txt
const USER_AGENT_TOKEN: &str = "MinoruFediverseCrawler";

/// The string to be sent with each HTTP request.
const USER_AGENT_FULL: &str = "Minoru's Fediverse Crawler (+https://nodes.fediverse.party)";

/// A redirection from one URL to another.
#[derive(Debug)]
pub struct Redirection {
    /// The URL from which we were redirected.
    pub from: Url,

    /// The URL to which we were redirected.
    pub to: Url,
}

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

pub struct HttpClient {
    logger: Logger,
    inner: Agent,
    robots_txt: String,
}

impl HttpClient {
    pub fn new(logger: Logger, host: Host) -> Result<Self, HttpClientError> {
        let inner = ureq::AgentBuilder::new()
            // We'll handle redirects ourselves
            .redirects(0)
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT_FULL)
            .build();
        let robots_txt = {
            let url = format!("https://{host}/robots.txt");
            let url = Url::parse(&url).map_err(HttpClientError::UrlParseError)?;
            info!(logger, "Fetching robots.txt");
            get_with_type_ignoring_404(&logger, &inner, &url, None)?
                .into_string()
                .map_err(HttpClientError::UreqStdError)?
        };
        Ok(Self {
            logger,
            inner,
            robots_txt,
        })
    }

    pub fn get(&self, url: &Url) -> Result<ureq::Response, HttpClientError> {
        if !self.allowed_by_robots_txt(url.as_str()) {
            return Err(HttpClientError::ForbiddenByRobotsTxt(url.to_owned()));
        }

        match get_with_type_ignoring_404(&self.logger, &self.inner, url, Some("application/json")) {
            Ok(r) if r.status() == 404 => {
                let ureq_err = ureq::Error::Status(404, r);
                Err(HttpClientError::UreqError(Box::new(ureq_err)))
            }
            x => x,
        }
    }

    fn allowed_by_robots_txt(&self, url: &str) -> bool {
        use robotstxt::DefaultMatcher;
        let mut matcher = DefaultMatcher::default();
        matcher.one_agent_allowed_by_robots(&self.robots_txt, USER_AGENT_TOKEN, url)
    }
}

fn get_with_type_ignoring_404(
    logger: &Logger,
    agent: &Agent,
    url: &Url,
    acceptable_type: Option<&str>,
) -> Result<ureq::Response, HttpClientError> {
    // Our redirect policy is:
    // - follow redirects as long as they point to the same hostname:port, and schema didn't
    //   change
    // - stop after 10 redirects
    const REDIRECTS_LIMIT: u8 = 10;
    let mut redirects_left = REDIRECTS_LIMIT;
    let mut current_url = url.to_owned();
    let mut response;
    loop {
        let mut request = agent
            .get(current_url.as_str())
            .timeout(Duration::from_secs(10));
        if let Some(t) = acceptable_type {
            request = request.set("Accept", t);
        }

        match request.call() {
            Ok(r) => response = r,
            Err(ureq::Error::Status(404, r)) => response = r,
            Err(e) => return Err(HttpClientError::UreqError(Box::new(e))),
        }
        if !is_redirect(response.status()) {
            break;
        }

        // invariant: response's status is a redirect code

        let to = response
            .header("location")
            .and_then(|h| Url::parse(h).ok())
            .ok_or_else(|| HttpClientError::NoLocationHeader(current_url.clone()))?;

        if !is_same_origin(&to, &current_url) {
            error!(
                logger,
                "Redirect points to {} which is of different origin that {}; stopping here",
                to,
                current_url
            );
            break;
        }

        current_url = to;

        redirects_left = redirects_left.saturating_sub(1);
        if redirects_left == 0 {
            break;
        }
    }
    redirect_into_error(url, &response)?;
    Ok(response)
}

fn is_temporary_redirect(status: u16) -> bool {
    const FOUND: u16 = 302;
    const SEE_OTHER: u16 = 303;
    const TEMPORARY_REDIRECT: u16 = 307;

    status == FOUND || status == SEE_OTHER || status == TEMPORARY_REDIRECT
}

fn is_permanent_redirect(status: u16) -> bool {
    const MOVED_PERMANENTLY: u16 = 301;
    const PERMANENT_REDIRECT: u16 = 308;

    status == MOVED_PERMANENTLY || status == PERMANENT_REDIRECT
}

fn is_redirect(status: u16) -> bool {
    is_temporary_redirect(status) || is_permanent_redirect(status)
}

fn redirect_into_error(from: &Url, response: &ureq::Response) -> Result<(), HttpClientError> {
    if !is_redirect(response.status()) {
        return Ok(());
    }

    // invariant: `response` is a redirect

    let from = from.to_owned();
    let to = response
        .header("location")
        .and_then(|h| Url::parse(h).ok())
        .ok_or_else(|| HttpClientError::NoLocationHeader(from.clone()))?;

    if is_temporary_redirect(response.status()) {
        return Err(HttpClientError::Moving(Box::new(Redirection { from, to })));
    } else if is_permanent_redirect(response.status()) {
        return Err(HttpClientError::Moved(Box::new(Redirection { from, to })));
    }

    unreachable!(
        "The redirect is neither temporary not permanent: status code {}",
        response.status()
    )
}

/// Returns `true` if the URLs have the same schema, domain, and port.
fn is_same_origin(lhs: &Url, rhs: &Url) -> bool {
    lhs.origin() == rhs.origin()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
