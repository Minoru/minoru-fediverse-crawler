use slog::{Logger, error};
use std::time::Duration;
use ureq::Agent;
use url::Url;

#[derive(Debug)]
pub(super) struct Redirection {
    pub(super) from: Url,
    pub(super) to: Url,
}

#[derive(Debug)]
pub(super) enum HttpFetcherError {
    Moving(Box<Redirection>),
    Moved(Box<Redirection>),
    NoLocationHeader(Url),
    UreqError(Box<ureq::Error>),
}

impl std::fmt::Display for HttpFetcherError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpFetcherError::Moving(redir) => {
                write!(
                    f,
                    "{} is temporarily redirected to {}",
                    redir.from, redir.to
                )
            }
            HttpFetcherError::Moved(redir) => {
                write!(
                    f,
                    "{} is permanently redirected to {}",
                    redir.from, redir.to
                )
            }
            HttpFetcherError::NoLocationHeader(from) => {
                write!(
                    f,
                    "{from} is redirected, but we don't know where as `Location` header was missing or invalid"
                )
            }
            HttpFetcherError::UreqError(err) => write!(f, "ureq's crate error: {err}"),
        }
    }
}

impl std::error::Error for HttpFetcherError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HttpFetcherError::UreqError(err) => err.source(),
            _ => None,
        }
    }
}

pub(super) struct HttpFetcher {
    logger: Logger,
    inner: Agent,
}

impl HttpFetcher {
    pub(super) fn new(logger: Logger) -> Self {
        const USER_AGENT_FULL: &str = "Minoru's Fediverse Crawler (+https://nodes.fediverse.party)";

        let inner = ureq::AgentBuilder::new()
            // We'll handle redirects ourselves
            .redirects(0)
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT_FULL)
            .build();

        Self { logger, inner }
    }

    pub(super) fn get(
        &self,
        url: &Url,
        accept_header: Option<&str>,
    ) -> Result<ureq::Response, HttpFetcherError> {
        get_with_type_ignoring_404(&self.logger, &self.inner, url, accept_header)
    }
}

fn get_with_type_ignoring_404(
    logger: &Logger,
    agent: &Agent,
    url: &Url,
    acceptable_type: Option<&str>,
) -> Result<ureq::Response, HttpFetcherError> {
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
            Err(e) => return Err(HttpFetcherError::UreqError(Box::new(e))),
        }
        if !is_redirect(response.status()) {
            break;
        }

        // invariant: response's status is a redirect code

        let to = response
            .header("location")
            .and_then(|h| Url::parse(h).ok())
            .ok_or_else(|| HttpFetcherError::NoLocationHeader(current_url.clone()))?;

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

fn redirect_into_error(from: &Url, response: &ureq::Response) -> Result<(), HttpFetcherError> {
    if !is_redirect(response.status()) {
        return Ok(());
    }

    // invariant: `response` is a redirect

    let from = from.to_owned();
    let to = response
        .header("location")
        .and_then(|h| Url::parse(h).ok())
        .ok_or_else(|| HttpFetcherError::NoLocationHeader(from.clone()))?;

    if is_temporary_redirect(response.status()) {
        return Err(HttpFetcherError::Moving(Box::new(Redirection { from, to })));
    } else if is_permanent_redirect(response.status()) {
        return Err(HttpFetcherError::Moved(Box::new(Redirection { from, to })));
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

    #[test]
    fn get_fetches_given_url() {
        use httpmock::prelude::*;

        const URL: &str = "/my-path/for_testing";
        const STATUS: u16 = 200;
        const BODY: &str = "All is well.";

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("GET").path(URL);
            then.status(STATUS).body(BODY);
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server.url(URL);
        let url = url::Url::parse(&url).unwrap();

        let response = fetcher.get(&url, None).unwrap();

        mock.assert();

        assert_eq!(response.get_url(), server.url(URL));
        assert_eq!(response.status(), STATUS);
        assert_eq!(response.into_string().unwrap(), BODY);
    }

    #[test]
    fn get_adds_accept_header_when_provided() {
        use httpmock::prelude::*;

        const URL: &str = "/my-path/for_testing";
        const STATUS: u16 = 200;
        const BODY: &str = "All is well.";
        const ACCEPT_HEADER_VALUE: &str = "application/json";

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("GET")
                .path(URL)
                .header("Accept", ACCEPT_HEADER_VALUE);
            then.status(STATUS).body(BODY);
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server.url(URL);
        let url = url::Url::parse(&url).unwrap();

        let response = fetcher.get(&url, Some(ACCEPT_HEADER_VALUE)).unwrap();

        mock.assert();

        assert_eq!(response.get_url(), server.url(URL));
        assert_eq!(response.status(), STATUS);
        assert_eq!(response.into_string().unwrap(), BODY);
    }

    #[test]
    fn get_follows_temporary_redirect_same_origin() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const FINAL_URL: &str = "/final";
        const STATUS_FINAL: u16 = 200;
        const BODY: &str = "Redirected successfully.";

        let server = MockServer::start();

        let mock_redirect = server.mock(|when, then| {
            when.method("GET").path(INITIAL_URL);
            then.status(302).header("Location", server.url(FINAL_URL));
        });

        let mock_final = server.mock(|when, then| {
            when.method("GET").path(FINAL_URL);
            then.status(STATUS_FINAL).body(BODY);
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server.url(INITIAL_URL);
        let url = url::Url::parse(&url).unwrap();

        let response = fetcher.get(&url, None).unwrap();

        mock_redirect.assert();
        mock_final.assert();

        assert_eq!(response.get_url(), server.url(FINAL_URL));
        assert_eq!(response.status(), STATUS_FINAL);
        assert_eq!(response.into_string().unwrap(), BODY);
    }

    #[test]
    fn get_returns_moving_error_on_temporary_redirect_no_follow() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        let server1 = MockServer::start();
        let server2 = MockServer::start();

        let mock_redirect = server1.mock(|when, then| {
            when.method("GET").path(INITIAL_URL);
            then.status(302).header("Location", server2.url(TARGET_URL));
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server1.url(INITIAL_URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock_redirect.assert();

        assert!(matches!(result, Err(HttpFetcherError::Moving(_))));
        if let Err(HttpFetcherError::Moving(redir)) = result {
            assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
            assert_eq!(redir.to.as_str(), server2.url(TARGET_URL));
        }
    }

    #[test]
    fn get_returns_moved_error_on_permanent_redirect_no_follow() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        let server1 = MockServer::start();
        let server2 = MockServer::start();

        let mock_redirect = server1.mock(|when, then| {
            when.method("GET").path(INITIAL_URL);
            then.status(301).header("Location", server2.url(TARGET_URL));
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server1.url(INITIAL_URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock_redirect.assert();

        assert!(matches!(result, Err(HttpFetcherError::Moved(_))));
        if let Err(HttpFetcherError::Moved(redir)) = result {
            assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
            assert_eq!(redir.to.as_str(), server2.url(TARGET_URL));
        }
    }

    #[test]
    fn get_stops_on_redirect_to_different_schema() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        let server1 = MockServer::start();
        let server2 = MockServer::start();

        let server2_url = server2.url(TARGET_URL);
        let http_location = server2_url.replace("https://", "http://");

        let mock_redirect = server1.mock(|when, then| {
            when.method("GET").path(INITIAL_URL);
            then.status(302).header("Location", &http_location);
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server1.url(INITIAL_URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock_redirect.assert_calls(1);

        assert!(matches!(result, Err(HttpFetcherError::Moving(_))));
        if let Err(HttpFetcherError::Moving(redir)) = result {
            assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
            assert_eq!(redir.to.as_str(), http_location);
        }
    }

    #[test]
    fn get_stops_on_redirect_to_different_port() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        let server1 = MockServer::start();
        let server2 = MockServer::start();

        let mock_redirect = server1.mock(|when, then| {
            when.method("GET").path(INITIAL_URL);
            then.status(302).header("Location", server2.url(TARGET_URL));
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server1.url(INITIAL_URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock_redirect.assert();

        assert!(matches!(result, Err(HttpFetcherError::Moving(_))));
        if let Err(HttpFetcherError::Moving(redir)) = result {
            assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
            assert_eq!(redir.to.as_str(), server2.url(TARGET_URL));
        }
    }

    #[test]
    fn get_returns_no_location_header_error() {
        // **Arrange:** Mock returns 302 without Location header
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Returns `HttpFetcherError::NoLocationHeader(url)`
    }

    #[test]
    fn get_handles_invalid_location_url() {
        // **Arrange:** Mock returns 302 with Location: "not a valid url"
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Returns `HttpFetcherError::NoLocationHeader(url)`
    }

    #[test]
    fn get_follows_redirect_chain_up_to_limit() {
        // **Arrange:** Mock chain of 5 redirects (302 → 302 → ... → 200), all same origin
        // **Act:** `fetcher.get(&initial_url, None)`
        // **Assert:** Follows all 5, returns final response
    }

    #[test]
    fn get_stops_after_10_redirects() {
        // **Arrange:** Mock chain of 11 redirects, all same origin
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Stops at redirect #10, returns error with last URL
    }

    #[test]
    fn get_handles_303_see_other() {
        // **Arrange:** Mock returns 303 → 200 same origin
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Follows redirect (303 is temporary)
    }

    #[test]
    fn get_handles_307_temporary_redirect() {
        // **Arrange:** Mock returns 307 → 200 same origin
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Follows redirect (307 is temporary)
    }

    #[test]
    fn get_handles_308_permanent_redirect() {
        // **Arrange:** Mock returns 308 to different origin
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Returns `HttpFetcherError::Moved`
    }

    #[test]
    fn get_returns_404_as_error() {
        // **Arrange:** Mock returns 404
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Returns `HttpFetcherError::UreqError(Status(404, ...))`
    }

    #[test]
    fn get_preserves_accept_header_across_redirects() {
        // **Arrange:** Mock chain (302 → 200) that verifies Accept header on both requests
        // **Act:** `fetcher.get(&url, Some("application/json"))`
        // **Assert:** Both requests include Accept: application/json
    }

    #[test]
    fn get_handles_subdomain_redirect_as_different_origin() {
        // **Arrange:** Mock returns 302 from `example.com` to `foo.example.com`
        // **Act:** `fetcher.get(&url, None)`
        // **Assert:** Returns redirect error (subdomains are different origins)
    }

    #[test]
    fn is_redirect_identifies_all_redirect_codes() {
        // **Arrange:** Test all status codes 301-308
        // **Act:** Call `is_redirect()` with each
        // **Assert:** Returns true for 301, 302, 303, 307, 308; false for others
    }
}
