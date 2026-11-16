//! Asking a channel's own source for the newest cohors, cheaply.
//!
//! Every source answers a conditional request (`If-None-Match` with the ETag it
//! gave last time) with a bodiless `304 Not Modified` while nothing changed, so
//! looking on every launch and every few minutes costs one tiny round trip.
//! Launches closer together than [`super::CHECK_FLOOR`] share one answer
//! through the memo.
//!
//! The test seam: `COHORS_UPDATE_SOURCE` (or the older `COHORS_UPDATE_URL`)
//! points every lookup at one local file or URL instead, read in any of the
//! sources' shapes or as a bare version (`0.7.8`).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use cohors_core::update::{VersionSource, latest_in_any};

use super::{CHECK_FLOOR, Memo};

/// Where to ask, and how to read the answer.
#[derive(Debug, Clone)]
pub(crate) struct Lookup {
    /// A URL, or (the test seam) a local file.
    pub location: String,
    /// How to read the answer; `None` reads any shape (the test seam).
    pub source: Option<VersionSource>,
}

impl Lookup {
    /// The lookup for `source`, unless the test seam redirects every lookup.
    pub fn for_source(source: VersionSource) -> Lookup {
        match seam() {
            Some(location) => Lookup {
                location,
                source: None,
            },
            None => Lookup {
                location: source.url().to_string(),
                source: Some(source),
            },
        }
    }

    /// How the place asked is named to a person.
    pub fn label(&self) -> String {
        match self.source {
            Some(source) => source.label().to_string(),
            None => self.location.clone(),
        }
    }

    fn read(&self, body: &str) -> Option<String> {
        match self.source {
            Some(source) => source.latest_in(body),
            None => latest_in_any(body),
        }
    }
}

/// `COHORS_UPDATE_SOURCE`, or the older `COHORS_UPDATE_URL`, when set.
fn seam() -> Option<String> {
    ["COHORS_UPDATE_SOURCE", "COHORS_UPDATE_URL"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
}

/// A location that is a local file: `file://…`, or anything without a scheme.
pub(crate) fn local_path(location: &str) -> Option<PathBuf> {
    if let Some(path) = location.strip_prefix("file://") {
        return Some(PathBuf::from(path));
    }
    (!location.contains("://")).then(|| PathBuf::from(location))
}

/// The User-Agent on every request: cohors' version and where it comes from,
/// nothing about the person or their repositories.
pub(crate) fn user_agent() -> String {
    format!(
        "cohors/{} (+https://github.com/{})",
        super::CURRENT,
        super::REPO
    )
}

/// What one look found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Answer {
    /// The newest version known: just now, or from the last answer when this
    /// one could not be had.
    pub latest: Option<String>,
    /// Did the source answer this time (a body or a 304), or was the last
    /// answer still fresh enough to use?
    pub reached: bool,
    /// Why the source could not be asked, when it could not.
    pub error: Option<String>,
}

/// The newest version `lookup` lists, through the memo's record for it. Asks
/// only when the record is older than the floor (or `force`), conditionally.
pub(crate) fn latest(lookup: &Lookup, memo: &mut Memo, now: i64, force: bool) -> Answer {
    let seen = memo.sources.entry(lookup.location.clone()).or_default();
    if !force && seen.checked_at > 0 && now - seen.checked_at < CHECK_FLOOR {
        return Answer {
            latest: seen.latest.clone(),
            reached: seen.error.is_none(),
            error: seen.error.clone(),
        };
    }
    seen.checked_at = now;
    let outcome = fetch(lookup, seen.etag.as_deref()).and_then(|fetched| match fetched {
        Fetched::NotModified => Ok(seen.latest.clone()),
        Fetched::Body { text, etag } => {
            let latest = lookup
                .read(&text)
                .ok_or_else(|| anyhow!("{} gave an answer cohors cannot read", lookup.label()))?;
            // The ETag is kept only with an answer that was read, so a bad
            // answer is never pinned by 304s.
            seen.etag = etag;
            Ok(Some(latest))
        }
    });
    match outcome {
        Ok(latest) => {
            seen.latest = latest.clone();
            seen.error = None;
            Answer {
                latest,
                reached: true,
                error: None,
            }
        }
        Err(err) => {
            let error = format!("{err:#}");
            tracing::debug!("update lookup at {} failed: {error}", lookup.location);
            seen.error = Some(error.clone());
            Answer {
                latest: seen.latest.clone(),
                reached: false,
                error: Some(error),
            }
        }
    }
}

enum Fetched {
    NotModified,
    Body { text: String, etag: Option<String> },
}

fn fetch(lookup: &Lookup, etag: Option<&str>) -> Result<Fetched> {
    if let Some(path) = local_path(&lookup.location) {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        return Ok(Fetched::Body { text, etag: None });
    }
    // Short: a slow network must never hold up anything that waits on this.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .build();
    let mut request = agent.get(&lookup.location).set("User-Agent", &user_agent());
    if let Some(source) = lookup.source {
        request = request.set("Accept", source.accept());
    }
    if let Some(etag) = etag {
        request = request.set("If-None-Match", etag);
    }
    let response = request
        .call()
        .with_context(|| format!("asking {}", lookup.label()))?;
    if response.status() == 304 {
        return Ok(Fetched::NotModified);
    }
    let etag = response.header("ETag").map(str::to_string);
    let text = response
        .into_string()
        .with_context(|| format!("reading {}'s answer", lookup.label()))?;
    Ok(Fetched::Body { text, etag })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A one-file web server on loopback that answers with an ETag and a 304
    /// when it is sent back, counting what it was asked.
    struct Server {
        url: String,
        asked: Arc<Mutex<Vec<Option<String>>>>,
        body: Arc<Mutex<String>>,
    }

    fn serve(body: &str) -> Server {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}/latest", server.server_addr().to_ip().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let body = Arc::new(Mutex::new(body.to_string()));
        let (log, text) = (asked.clone(), body.clone());
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let sent = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("If-None-Match"))
                    .map(|h| h.value.to_string());
                log.lock().unwrap().push(sent.clone());
                let text = text.lock().unwrap().clone();
                let etag = format!("\"{}\"", text.len());
                let header = tiny_http::Header::from_bytes("ETag", etag.as_bytes()).unwrap();
                let response = if sent.as_deref() == Some(etag.as_str()) {
                    tiny_http::Response::from_string("").with_status_code(304)
                } else {
                    tiny_http::Response::from_string(text)
                };
                let _ = request.respond(response.with_header(header));
            }
        });
        Server { url, asked, body }
    }

    #[test]
    fn a_second_look_costs_a_304_and_a_new_release_shows_at_once() {
        let server = serve(r#"{"tag_name": "v0.7.7"}"#);
        let lookup = Lookup {
            location: server.url.clone(),
            source: Some(VersionSource::GithubRelease),
        };
        let mut memo = Memo::default();
        let first = latest(&lookup, &mut memo, 1_000, false);
        assert_eq!(first.latest.as_deref(), Some("0.7.7"));
        assert!(first.reached);
        // Within the floor: the memo answers, nothing is asked.
        latest(&lookup, &mut memo, 1_000 + 10, false);
        assert_eq!(server.asked.lock().unwrap().len(), 1);
        // After it: asked again with the ETag, and the 304 keeps the answer.
        let again = latest(&lookup, &mut memo, 1_000 + CHECK_FLOOR, false);
        assert_eq!(again.latest.as_deref(), Some("0.7.7"));
        assert_eq!(
            server.asked.lock().unwrap().last().cloned().flatten(),
            Some(format!("\"{}\"", r#"{"tag_name": "v0.7.7"}"#.len()))
        );
        // A release changes the answer (and its ETag): the next look sees it.
        *server.body.lock().unwrap() = r#"{"tag_name": "v0.7.8", "x": 1}"#.into();
        let moved = latest(&lookup, &mut memo, 1_000 + 2 * CHECK_FLOOR, false);
        assert_eq!(moved.latest.as_deref(), Some("0.7.8"));
        // `force` asks even inside the floor.
        latest(&lookup, &mut memo, 1_000 + 2 * CHECK_FLOOR + 1, true);
        assert_eq!(server.asked.lock().unwrap().len(), 4);
    }

    #[test]
    fn an_unreachable_source_keeps_the_last_answer_and_says_why() {
        let mut memo = Memo::default();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "0.7.8\n").unwrap();
        let lookup = Lookup {
            location: file.path().display().to_string(),
            source: None,
        };
        assert_eq!(
            latest(&lookup, &mut memo, 1_000, false).latest.as_deref(),
            Some("0.7.8")
        );
        let gone = Lookup {
            location: "/nonexistent/cohors-latest".into(),
            ..lookup.clone()
        };
        memo.sources.insert(
            gone.location.clone(),
            memo.sources[&lookup.location].clone(),
        );
        let answer = latest(&gone, &mut memo, 1_000 + CHECK_FLOOR, false);
        assert_eq!(answer.latest.as_deref(), Some("0.7.8"));
        assert!(!answer.reached);
        assert!(answer.error.unwrap().contains("reading"));
    }

    #[test]
    fn locations_without_a_scheme_are_files() {
        assert_eq!(local_path("/tmp/x"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(local_path("file:///tmp/x"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(local_path("https://pypi.org/pypi/cohors/json"), None);
    }
}
