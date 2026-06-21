//! A whisper from the surface: check GitHub Releases for a newer version.
//!
//! The repository is public, so the GitHub REST API needs no token. The check
//! is a single blocking GET (run off the UI thread) with a short timeout, and
//! every failure is treated as "no update" — an offline user is never nagged.

use std::time::Duration;

/// `owner/repo` on GitHub. Releases here drive the in-app update prompt.
const REPO: &str = "4G0NYY/AbyssC";

/// A newer release than the one currently running.
#[derive(Clone, Debug)]
pub struct UpdateInfo {
    /// Version string, normalized without a leading `v` (e.g. `0.4.0`).
    pub version: String,
    /// The release's web page, where the installer can be downloaded.
    pub url: String,
}

/// Ask GitHub for the latest release. `Ok(Some(_))` only when it is newer than
/// this build; `Ok(None)` when up to date or no releases exist; `Err` on any
/// network/parse problem (callers should treat `Err` as silently "no update").
pub fn check() -> Result<Option<UpdateInfo>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(6)).build();

    let response = agent
        .get(&url)
        .set("User-Agent", concat!("AbyssC/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .call();

    let value: serde_json::Value = match response {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        // 404 = the repo has no published releases yet.
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };

    let tag = value.get("tag_name").and_then(|v| v.as_str()).unwrap_or_default();
    let html = value.get("html_url").and_then(|v| v.as_str()).unwrap_or_default();
    if tag.is_empty() {
        return Ok(None);
    }

    if is_newer(tag, env!("CARGO_PKG_VERSION")) {
        Ok(Some(UpdateInfo {
            version: tag.trim_start_matches(['v', 'V']).to_string(),
            url: html.to_string(),
        }))
    } else {
        Ok(None)
    }
}

/// Is `latest` a higher version than `current`? Tags may carry a `v` prefix and
/// a pre-release/build suffix, both of which are tolerated.
fn is_newer(latest: &str, current: &str) -> bool {
    parse(latest) > parse(current)
}

/// Extract `(major, minor, patch)` from a version string, defaulting to 0.
fn parse(s: &str) -> (u64, u64, u64) {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let mut parts = s.split(['.', '-', '+']);
    let num = |p: Option<&str>| -> u64 {
        p.map(|x| x.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .and_then(|d| d.parse().ok())
            .unwrap_or(0)
    };
    (num(parts.next()), num(parts.next()), num(parts.next()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_and_without_prefix() {
        assert_eq!(parse("v1.2.3"), (1, 2, 3));
        assert_eq!(parse("0.3.0"), (0, 3, 0));
        assert_eq!(parse("v2.0.0-beta.1"), (2, 0, 0));
        assert_eq!(parse("garbage"), (0, 0, 0));
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.3.1", "0.3.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.4.0", "0.3.9"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
    }
}
