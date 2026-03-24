use crate::checker::http_client::{HttpClientError, Redirection};
use slog::{Logger, error};
use std::time::Duration;
use ureq::Agent;
use url::Url;

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
    ) -> Result<ureq::Response, HttpClientError> {
        get_with_type_ignoring_404(&self.logger, &self.inner, url, accept_header)
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
