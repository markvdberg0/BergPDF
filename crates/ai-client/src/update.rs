//! Checking whether a newer BergPDF has been published.
//!
//! Only runs when the user allowed it (Preferences) or pressed "Check for updates". It asks GitHub's public API
//! for the latest *published* release (drafts and pre-releases are never returned) and sends nothing but the usual
//! HTTP request headers and the program name. It never downloads or starts an installer: the application shows a
//! link to the release page and the user takes it from there.

use std::io::Read;
use std::time::Duration;

/// Where the latest published release is described.
pub const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/markvdberg0/BergPDF/releases/latest";
/// Where releases can be downloaded.
pub const RELEASES_PAGE: &str = "https://github.com/markvdberg0/BergPDF/releases";

const MAX_BODY: u64 = 1024 * 1024;

/// A published release that is newer than the running program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Version without the leading `v`, e.g. `0.1.1`.
    pub version: String,
    /// Page of this release (built here from the tag, not taken from the response).
    pub url: String,
}

/// Why a check did not give an answer.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UpdateError {
    #[error("Could not reach the update server: {0}")]
    Network(String),
    #[error("The update server answered with status {0}.")]
    Status(u16),
    #[error("The update server's answer could not be understood.")]
    Parse,
}

/// `1.2.3`, `v1.2.3` or `1.2.3-rc1` as numbers; `None` for anything else.
fn parse_version(s: &str) -> Option<(u64, u64, u64, bool)> {
    let s = s.trim().trim_start_matches('v');
    let (core, pre) = match s.split_once(['-', '+']) {
        Some((c, _)) => (c, true),
        None => (s, false),
    };
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((a, b, c, pre))
}

/// Whether `latest` is a newer, final version than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(c), Some(l)) => !l.3 && (l.0, l.1, l.2) > (c.0, c.1, c.2),
        _ => false,
    }
}

/// Read the body of GitHub's `releases/latest` answer. `Ok(None)` when it is not newer than `current`.
pub fn parse_latest(json: &str, current: &str) -> Result<Option<Release>, UpdateError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|_| UpdateError::Parse)?;
    let tag = v
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or(UpdateError::Parse)?;
    // The tag ends up in a link, so only accept what a version tag looks like.
    if tag.is_empty()
        || tag.len() > 40
        || !tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        return Err(UpdateError::Parse);
    }
    if v.get("draft").and_then(|d| d.as_bool()) == Some(true)
        || v.get("prerelease").and_then(|d| d.as_bool()) == Some(true)
        || !is_newer(current, tag)
    {
        return Ok(None);
    }
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: format!("{RELEASES_PAGE}/tag/{tag}"),
    }))
}

fn loopback(url: &str) -> bool {
    url.starts_with("http://127.0.0.1") || url.starts_with("http://localhost")
}

/// Ask `api_url` (normally [`LATEST_RELEASE_API`]) for the latest release. Blocks; call it from a worker thread.
pub fn check(api_url: &str, current: &str) -> Result<Option<Release>, UpdateError> {
    if !(api_url.starts_with("https://") || loopback(api_url)) {
        return Err(UpdateError::Network("the address must use https".into()));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .https_only(!loopback(api_url))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent(format!("BergPDF/{current}"))
        .build()
        .into();
    let mut resp = agent
        .get(api_url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    let code = resp.status().as_u16();
    if !(200..300).contains(&code) {
        return Err(UpdateError::Status(code));
    }
    let mut body = String::new();
    resp.body_mut()
        .as_reader()
        .take(MAX_BODY)
        .read_to_string(&mut body)
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    parse_latest(&body, current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    #[test]
    fn versions_compare_numerically_and_ignore_pre_releases() {
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("0.1.9", "1.0.0"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.2.0", "v0.1.9"));
        assert!(!is_newer("0.1.0", "0.2.0-rc1"));
        assert!(!is_newer("0.1.0", "nightly"));
        assert!(!is_newer("0.1.0", "1.2"));
    }

    #[test]
    fn newer_release_is_reported_with_a_link_built_from_the_tag() {
        let json = r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false,
                       "html_url":"https://evil.example/phish"}"#;
        let r = parse_latest(json, "0.1.0").unwrap().unwrap();
        assert_eq!(r.version, "0.2.0");
        assert_eq!(
            r.url,
            "https://github.com/markvdberg0/BergPDF/releases/tag/v0.2.0"
        );
        assert_eq!(parse_latest(json, "0.2.0").unwrap(), None);
    }

    #[test]
    fn drafts_prereleases_and_odd_tags_are_not_offered() {
        let draft = r#"{"tag_name":"v9.0.0","draft":true}"#;
        let pre = r#"{"tag_name":"v9.0.0","prerelease":true}"#;
        assert_eq!(parse_latest(draft, "0.1.0").unwrap(), None);
        assert_eq!(parse_latest(pre, "0.1.0").unwrap(), None);
        let odd = r#"{"tag_name":"v9.0.0/../x"}"#;
        assert_eq!(parse_latest(odd, "0.1.0"), Err(UpdateError::Parse));
        assert_eq!(parse_latest("not json", "0.1.0"), Err(UpdateError::Parse));
        assert_eq!(parse_latest("{}", "0.1.0"), Err(UpdateError::Parse));
    }

    fn serve_once(status: &str, body: &str) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let reply = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply.as_bytes());
            }
        });
        format!("http://{addr}/latest")
    }

    #[test]
    fn check_talks_to_a_server_and_reports_status_errors() {
        let url = serve_once("200 OK", r#"{"tag_name":"v0.3.0"}"#);
        let r = check(&url, "0.1.0").unwrap().unwrap();
        assert_eq!(r.version, "0.3.0");
        let url = serve_once("404 Not Found", "{}");
        assert_eq!(check(&url, "0.1.0"), Err(UpdateError::Status(404)));
        assert!(matches!(
            check("http://example.com/x", "0.1.0"),
            Err(UpdateError::Network(_))
        ));
    }
}
