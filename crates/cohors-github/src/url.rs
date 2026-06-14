//! Pure parsing of a git remote URL into a GitHub `(owner, repo)` pair.
//!
//! Kept free of I/O so it can be unit-tested without a network. Only
//! `github.com` is recognised; anything else (other hosts, malformed input)
//! resolves to `None` and is skipped by the enricher.

/// Parse a remote URL into `(owner, repo)` if (and only if) it points at
/// `github.com`. A trailing `.git` is stripped. Returns `None` for non-GitHub
/// hosts or anything that doesn't have exactly an owner and a repo segment.
///
/// Recognised shapes:
/// - `https://github.com/owner/repo` (with optional `.git`, optional trailing `/`)
/// - `git@github.com:owner/repo.git` (scp-like SSH)
/// - `ssh://git@github.com/owner/repo.git`
pub fn parse_repo(remote_url: &str) -> Option<(String, String)> {
    let url = remote_url.trim();

    // Pull the "host + path" portion out of the various URL shapes, normalising
    // everything to a `github.com/owner/repo` style string we can split.
    let rest = if let Some(scp) = url.strip_prefix("git@") {
        // scp-like: `git@github.com:owner/repo(.git)` — the host/path separator
        // is a colon, so turn it into a slash to match the others.
        scp.replacen(':', "/", 1)
    } else if let Some(after) = url.strip_prefix("ssh://") {
        // `ssh://git@github.com/owner/repo` — drop a `user@` if present.
        strip_userinfo(after).to_string()
    } else if let Some(after) = url.strip_prefix("https://") {
        strip_userinfo(after).to_string()
    } else if let Some(after) = url.strip_prefix("http://") {
        strip_userinfo(after).to_string()
    } else {
        return None;
    };

    // `rest` is now `host[:port]/owner/repo...`. Split host from path.
    let (host, path) = rest.split_once('/')?;

    // Drop a possible `:port` on the host before comparing.
    let host = host.split(':').next().unwrap_or(host);
    if !host.eq_ignore_ascii_case("github.com") {
        return None;
    }

    // The path must be exactly `owner/repo` (ignoring a trailing slash and the
    // optional `.git` suffix). Extra path segments mean it's not a repo root.
    let path = path.trim_end_matches('/');
    let (owner, repo) = path.split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);

    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }

    Some((owner.to_string(), repo.to_string()))
}

/// Strip a leading `user@` (URL userinfo) from a `host/path` string, if present.
/// Only the segment before the first `/` is considered, so a `@` inside the path
/// is left untouched.
fn strip_userinfo(s: &str) -> &str {
    match s.split_once('/') {
        Some((authority, path)) => {
            if let Some((_user, host)) = authority.split_once('@') {
                // Rebuild via the original slice so we keep the `/path` part.
                // SAFETY of indexing: `host` is a suffix of `authority`, which is
                // a prefix of `s`; recompute the offset into `s`.
                let host_start = s.len() - path.len() - 1 - host.len();
                &s[host_start..]
            } else {
                s
            }
        }
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_basic() {
        assert_eq!(
            parse_repo("https://github.com/owner/repo"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn https_with_dot_git() {
        assert_eq!(
            parse_repo("https://github.com/owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn https_trailing_slash() {
        assert_eq!(
            parse_repo("https://github.com/owner/repo/"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn ssh_scp_like_with_dot_git() {
        assert_eq!(
            parse_repo("git@github.com:owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn ssh_scp_like_without_dot_git() {
        assert_eq!(
            parse_repo("git@github.com:owner/repo"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn ssh_protocol_url() {
        assert_eq!(
            parse_repo("ssh://git@github.com/owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn https_with_userinfo() {
        // Tokens sometimes get embedded as `https://x-access-token@github.com/...`.
        assert_eq!(
            parse_repo("https://user@github.com/owner/repo.git"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn host_is_case_insensitive() {
        assert_eq!(
            parse_repo("https://GitHub.com/owner/repo"),
            Some(("owner".into(), "repo".into()))
        );
    }

    #[test]
    fn non_github_host_is_none() {
        assert_eq!(parse_repo("https://gitlab.com/owner/repo"), None);
        assert_eq!(parse_repo("git@bitbucket.org:owner/repo.git"), None);
        assert_eq!(
            parse_repo("https://example.com/github.com/owner/repo"),
            None
        );
    }

    #[test]
    fn missing_repo_is_none() {
        assert_eq!(parse_repo("https://github.com/owner"), None);
        assert_eq!(parse_repo("https://github.com/owner/"), None);
        assert_eq!(parse_repo("https://github.com/"), None);
    }

    #[test]
    fn extra_path_segments_are_none() {
        // A tree/blob URL is not a repo root.
        assert_eq!(parse_repo("https://github.com/owner/repo/tree/main"), None);
    }

    #[test]
    fn empty_and_garbage_are_none() {
        assert_eq!(parse_repo(""), None);
        assert_eq!(parse_repo("not a url"), None);
        assert_eq!(parse_repo("github.com/owner/repo"), None); // no scheme/ssh form
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(
            parse_repo("  https://github.com/owner/repo.git \n"),
            Some(("owner".into(), "repo".into()))
        );
    }
}
