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
        let mut request = agent.get(current_url.as_str());
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

        redirects_left = redirects_left.saturating_sub(1);
        if redirects_left == 0 {
            break;
        }

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
    fn get_follows_redirect_same_origin_all_codes() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const FINAL_URL: &str = "/final";
        const STATUS_FINAL: u16 = 200;

        // Test all redirect codes (301, 302, 303, 307, 308)
        for (redirect_status, body) in [
            (301, "Redirected with 301."),
            (302, "Redirected with 302."),
            (303, "Redirected with 303."),
            (307, "Redirected with 307."),
            (308, "Redirected with 308."),
        ] {
            let server = MockServer::start();

            let mock_redirect = server.mock(|when, then| {
                when.method("GET").path(INITIAL_URL);
                then.status(redirect_status)
                    .header("Location", server.url(FINAL_URL));
            });

            let mock_final = server.mock(|when, then| {
                when.method("GET").path(FINAL_URL);
                then.status(STATUS_FINAL).body(body);
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
            assert_eq!(response.into_string().unwrap(), body);
        }
    }

    #[test]
    fn get_returns_moving_error_on_temporary_redirect_different_origin_all_codes() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        // Test all temporary redirect codes (302, 303, 307)
        for redirect_status in [302, 303, 307] {
            let server1 = MockServer::start();
            let server2 = MockServer::start();

            let mock_redirect = server1.mock(|when, then| {
                when.method("GET").path(INITIAL_URL);
                then.status(redirect_status)
                    .header("Location", server2.url(TARGET_URL));
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
    }

    #[test]
    fn get_returns_moved_error_on_permanent_redirect_different_origin_all_codes() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        // Test all permanent redirect codes (301, 308)
        for redirect_status in [301, 308] {
            let server1 = MockServer::start();
            let server2 = MockServer::start();

            let mock_redirect = server1.mock(|when, then| {
                when.method("GET").path(INITIAL_URL);
                then.status(redirect_status)
                    .header("Location", server2.url(TARGET_URL));
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
    }

    #[test]
    fn get_stops_on_redirect_to_different_schema() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        // Test all redirect codes (301, 302, 303, 307, 308)
        for redirect_status in [301, 302, 303, 307, 308] {
            let server1 = MockServer::start();
            let server2 = MockServer::start();

            let server2_url = server2.url(TARGET_URL);
            let http_location = server2_url.replace("https://", "http://");

            let mock_redirect = server1.mock(|when, then| {
                when.method("GET").path(INITIAL_URL);
                then.status(redirect_status)
                    .header("Location", &http_location);
            });

            let logger = slog::Logger::root(slog::Discard, slog::o!());

            let fetcher = HttpFetcher::new(logger);

            let url = server1.url(INITIAL_URL);
            let url = url::Url::parse(&url).unwrap();

            let result = fetcher.get(&url, None);

            mock_redirect.assert_calls(1);

            if is_temporary_redirect(redirect_status) {
                assert!(matches!(result, Err(HttpFetcherError::Moving(_))));
                if let Err(HttpFetcherError::Moving(redir)) = result {
                    assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
                    assert_eq!(redir.to.as_str(), http_location);
                }
            } else if is_permanent_redirect(redirect_status) {
                assert!(matches!(result, Err(HttpFetcherError::Moved(_))));
                if let Err(HttpFetcherError::Moved(redir)) = result {
                    assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
                    assert_eq!(redir.to.as_str(), http_location);
                }
            }
        }
    }

    #[test]
    fn get_stops_on_redirect_to_different_port() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const TARGET_URL: &str = "/target";

        // Test all redirect codes (301, 302, 303, 307, 308)
        for redirect_status in [301, 302, 303, 307, 308] {
            let server1 = MockServer::start();
            let server2 = MockServer::start();

            let mock_redirect = server1.mock(|when, then| {
                when.method("GET").path(INITIAL_URL);
                then.status(redirect_status)
                    .header("Location", server2.url(TARGET_URL));
            });

            let logger = slog::Logger::root(slog::Discard, slog::o!());

            let fetcher = HttpFetcher::new(logger);

            let url = server1.url(INITIAL_URL);
            let url = url::Url::parse(&url).unwrap();

            let result = fetcher.get(&url, None);

            mock_redirect.assert_calls(1);

            if is_temporary_redirect(redirect_status) {
                assert!(matches!(result, Err(HttpFetcherError::Moving(_))));
                if let Err(HttpFetcherError::Moving(redir)) = result {
                    assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
                    assert_eq!(redir.to.as_str(), server2.url(TARGET_URL));
                }
            } else if is_permanent_redirect(redirect_status) {
                assert!(matches!(result, Err(HttpFetcherError::Moved(_))));
                if let Err(HttpFetcherError::Moved(redir)) = result {
                    assert_eq!(redir.from.as_str(), server1.url(INITIAL_URL));
                    assert_eq!(redir.to.as_str(), server2.url(TARGET_URL));
                }
            }
        }
    }

    #[test]
    fn get_returns_no_location_header_error() {
        use httpmock::prelude::*;

        const URL: &str = "/redirect-no-location";

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("GET").path(URL);
            then.status(302);
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server.url(URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock.assert();

        assert!(matches!(result, Err(HttpFetcherError::NoLocationHeader(_))));
        if let Err(HttpFetcherError::NoLocationHeader(from)) = result {
            assert_eq!(from.as_str(), server.url(URL));
        }
    }

    #[test]
    fn get_handles_invalid_location_url() {
        use httpmock::prelude::*;

        const URL: &str = "/redirect-invalid-location";

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("GET").path(URL);
            then.status(302).header("Location", "not a valid url");
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());

        let fetcher = HttpFetcher::new(logger);

        let url = server.url(URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock.assert();

        assert!(matches!(result, Err(HttpFetcherError::NoLocationHeader(_))));
        if let Err(HttpFetcherError::NoLocationHeader(from)) = result {
            assert_eq!(from.as_str(), server.url(URL));
        }
    }

    #[test]
    fn get_follows_redirect_chain_up_to_limit() {
        use httpmock::prelude::*;

        const FINAL_URL: &str = "/final";
        const STATUS_FINAL: u16 = 200;
        const BODY: &str = "End of chain.";
        const REDIRECT_COUNT: usize = 5;

        let server = MockServer::start();

        let mut mocks = Vec::new();
        for i in 1..REDIRECT_COUNT {
            let from = format!("/r{}", i);
            let to = format!("/r{}", i + 1);
            let mock = server.mock(|when, then| {
                when.method("GET").path(&from);
                then.status(302).header("Location", server.url(&to));
            });
            mocks.push(mock);
        }

        let last_redirect_mock = server.mock(|when, then| {
            when.method("GET").path(format!("/r{}", REDIRECT_COUNT));
            then.status(302).header("Location", server.url(FINAL_URL));
        });
        mocks.push(last_redirect_mock);

        let mock_final = server.mock(|when, then| {
            when.method("GET").path(FINAL_URL);
            then.status(STATUS_FINAL).body(BODY);
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let fetcher = HttpFetcher::new(logger);

        let url = server.url("/r1");
        let url = url::Url::parse(&url).unwrap();

        let response = fetcher.get(&url, None).unwrap();

        for mock in mocks {
            mock.assert();
        }
        mock_final.assert();

        assert_eq!(response.get_url(), server.url(FINAL_URL));
        assert_eq!(response.status(), STATUS_FINAL);
        assert_eq!(response.into_string().unwrap(), BODY);
    }

    #[test]
    fn get_stops_after_10_redirects() {
        use httpmock::prelude::*;

        let server = MockServer::start();

        let mut mocks = Vec::new();
        for i in 1..=10 {
            let from = format!("/r{}", i);
            let to = format!("/r{}", i + 1);
            let mock = server.mock(|when, then| {
                when.method("GET").path(&from);
                then.status(302).header("Location", server.url(&to));
            });
            mocks.push(mock);
        }

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let fetcher = HttpFetcher::new(logger);

        let url = server.url("/r1");
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        for mock in mocks {
            mock.assert();
        }

        assert!(matches!(result, Err(HttpFetcherError::Moving(_))));
        if let Err(HttpFetcherError::Moving(redir)) = result {
            assert_eq!(redir.from.as_str(), server.url("/r1"));
            assert_eq!(redir.to.as_str(), server.url("/r11"));
        }
    }

    #[test]
    fn get_returns_404_as_ordinary_response_not_an_error() {
        use httpmock::prelude::*;

        const URL: &str = "/missing";
        const STATUS: u16 = 404;
        const BODY: &str = "Not Found";

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
    fn get_preserves_accept_header_across_redirects() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const FINAL_URL: &str = "/final";
        const STATUS_FINAL: u16 = 200;
        const BODY: &str = "Redirected successfully.";
        const ACCEPT_HEADER_VALUE: &str = "application/json";

        let server = MockServer::start();

        let mock_redirect = server.mock(|when, then| {
            when.method("GET")
                .path(INITIAL_URL)
                .header("Accept", ACCEPT_HEADER_VALUE);
            then.status(302).header("Location", server.url(FINAL_URL));
        });

        let mock_final = server.mock(|when, then| {
            when.method("GET")
                .path(FINAL_URL)
                .header("Accept", ACCEPT_HEADER_VALUE);
            then.status(STATUS_FINAL).body(BODY);
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let fetcher = HttpFetcher::new(logger);

        let url = server.url(INITIAL_URL);
        let url = url::Url::parse(&url).unwrap();

        let response = fetcher.get(&url, Some(ACCEPT_HEADER_VALUE)).unwrap();

        mock_redirect.assert();
        mock_final.assert();

        assert_eq!(response.get_url(), server.url(FINAL_URL));
        assert_eq!(response.status(), STATUS_FINAL);
        assert_eq!(response.into_string().unwrap(), BODY);
    }

    #[test]
    fn get_times_out_on_slow_response() {
        use httpmock::prelude::*;

        const URL: &str = "/slow";
        const DELAY_SECS: u64 = 35;
        const STATUS: u16 = 200;
        const BODY: &str = "This should never arrive.";

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("GET").path(URL);
            then.status(STATUS)
                .body(BODY)
                .delay(Duration::from_secs(DELAY_SECS));
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let fetcher = HttpFetcher::new(logger);

        let url = server.url(URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock.assert();

        assert!(matches!(result, Err(HttpFetcherError::UreqError(_))));
        if let Err(HttpFetcherError::UreqError(err)) = result {
            let err_str = err.to_string();
            assert!(
                err_str.contains("timeout") || err_str.contains("timed out"),
                "Expected timeout error, got: {}",
                err_str
            );
        }
    }

    #[test]
    fn get_fails_on_connection_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let fetcher = HttpFetcher::new(logger);

        let url = Url::parse(&format!("http://127.0.0.1:{port}/connection-timeout")).unwrap();

        let start = std::time::Instant::now();
        let result = fetcher.get(&url, None);
        let elapsed = start.elapsed();

        drop(listener);

        assert!(matches!(result, Err(HttpFetcherError::UreqError(_))));
        if let Err(HttpFetcherError::UreqError(err)) = result {
            assert_eq!(err.kind(), ureq::ErrorKind::Io);
        }

        assert!(
            elapsed >= Duration::from_secs(29),
            "Expected connection timeout to take at least 29s, but only took {:?}",
            elapsed
        );
    }

    #[test]
    fn get_times_out_during_redirect_chain() {
        use httpmock::prelude::*;

        const INITIAL_URL: &str = "/initial";
        const SLOW_URL: &str = "/slow";
        const DELAY_SECS: u64 = 35;
        const STATUS_FINAL: u16 = 200;
        const BODY: &str = "This should never arrive.";

        let server = MockServer::start();

        let mock_redirect = server.mock(|when, then| {
            when.method("GET").path(INITIAL_URL);
            then.status(302).header("Location", server.url(SLOW_URL));
        });

        let mock_slow = server.mock(|when, then| {
            when.method("GET").path(SLOW_URL);
            then.status(STATUS_FINAL)
                .body(BODY)
                .delay(Duration::from_secs(DELAY_SECS));
        });

        let logger = slog::Logger::root(slog::Discard, slog::o!());
        let fetcher = HttpFetcher::new(logger);

        let url = server.url(INITIAL_URL);
        let url = url::Url::parse(&url).unwrap();

        let result = fetcher.get(&url, None);

        mock_redirect.assert();
        mock_slow.assert();

        assert!(matches!(result, Err(HttpFetcherError::UreqError(_))));
        if let Err(HttpFetcherError::UreqError(err)) = result {
            let err_str = err.to_string();
            assert!(
                err_str.contains("timeout") || err_str.contains("timed out"),
                "Expected timeout error, got: {}",
                err_str
            );
        }
    }
}
