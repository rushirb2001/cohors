//! The `Selector` — a serializable predicate over the fleet, resolved entirely
//! in the pure core and shared by every surface (ADR-024).
//!
//! A selector is the headless generalization of the TUI's target-set: instead
//! of "marked repos, else the cursor," it describes *which* repos by their
//! state. [`resolve`] turns one into an ordered `Vec<RepoId>` against the
//! current fleet snapshot, in the active sort order. Because resolution is a
//! pure function over `&[RepoSnapshot]`, `cohors scan --select behind` and the
//! MCP `list_repos({behind: true})` return identical sets.
//!
//! Set fields are **AND**-combined; an omitted field is simply no constraint.
//! Two safety rules matter for the agent surface:
//!
//! - The **empty selector resolves to nothing** (never "all"). A bulk action
//!   requires `{all: true}` or at least one explicit predicate, so an agent
//!   can't act across the whole fleet by forgetting an argument.
//! - Path matching is performed against the snapshot's canonical path. Any `~`
//!   expansion is the caller's job (the CLI/config layer that knows `$HOME`),
//!   keeping this module free of environment access and WASM-safe.

use serde::{Deserialize, Serialize};

use crate::attention::{self, Severity};
use crate::model::{Branch, CiStatus, RepoId, RepoSnapshot};
use crate::sort::{SortMode, sort_indices};

/// Minimum attention severity a repo must reach to match `attention`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionLevel {
    /// Anything the attention layer flags (severity ≥ notice) — same threshold
    /// as [`crate::Assessment::needs_attention`].
    Any,
    Notice,
    Warn,
    Risk,
}

impl AttentionLevel {
    /// The minimum [`Severity`] a repo must reach to satisfy this level.
    fn threshold(self) -> Severity {
        match self {
            AttentionLevel::Any | AttentionLevel::Notice => Severity::Notice,
            AttentionLevel::Warn => Severity::Warn,
            AttentionLevel::Risk => Severity::Risk,
        }
    }
}

/// A predicate over the fleet. Set fields AND together; omitted fields impose no
/// constraint. Build one in code, or deserialize it from the CLI/MCP.
///
/// Unknown JSON fields are ignored (rather than rejected) so that selectors
/// written against a future schema — e.g. `group` or `unreleased`, which need
/// data the snapshot doesn't carry yet — degrade gracefully instead of erroring.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selector {
    // ── identity / scope ──
    /// Explicit "the whole fleet." A no-op on its own beyond making the selector
    /// non-empty (so `{all: true}` matches everything, while `{}` matches nothing).
    pub all: bool,
    /// Exact repo ids.
    pub ids: Vec<String>,
    /// Glob over the repo's alias / directory name (case-insensitive), e.g. `pay*`.
    pub name: Option<String>,
    /// Glob over the canonical path, e.g. `~/work/**` (expand `~` before resolving).
    pub path_glob: Option<String>,
    /// Limit to repos under one root directory (a canonical path prefix).
    pub root: Option<String>,
    /// Belongs to this config-defined group (matched against the repo's stamped
    /// `groups`). Lets "act on the whole payments cluster" be one argument.
    pub group: Option<String>,

    // ── local state ──
    /// Working tree has uncommitted changes.
    pub dirty: bool,
    /// Local commits not on the upstream (`ahead > 0`).
    #[serde(alias = "unpushed")]
    pub ahead: bool,
    /// Upstream commits not pulled (`behind > 0`).
    pub behind: bool,
    /// Both ahead and behind.
    pub diverged: bool,
    /// No tracking branch configured.
    pub no_upstream: bool,
    /// At least one stash.
    pub has_stash: bool,
    /// Detached `HEAD`.
    pub detached: bool,
    /// The repo failed to read.
    pub error: bool,
    /// Current branch equals this exact name.
    pub branch: Option<String>,
    /// Minimum attention severity (clock-dependent — see [`resolve`]).
    pub attention: Option<AttentionLevel>,

    // ── remote (needs GitHub enrichment; absent ⇒ no match) ──
    /// CI status of the default branch.
    pub ci: Option<CiStatus>,
    /// At least this many open pull requests.
    pub min_prs: Option<u32>,

    // ── combinators ──
    /// Match if **any** sub-selector matches (OR).
    pub any_of: Vec<Selector>,
    /// Match if the sub-selector does **not** match (negation).
    pub not: Option<Box<Selector>>,
}

impl Selector {
    /// A selector with no constraint at all — it deliberately resolves to
    /// nothing (the safety rule above).
    pub fn is_empty(&self) -> bool {
        !self.all
            && self.ids.is_empty()
            && self.name.is_none()
            && self.path_glob.is_none()
            && self.root.is_none()
            && self.group.is_none()
            && !self.dirty
            && !self.ahead
            && !self.behind
            && !self.diverged
            && !self.no_upstream
            && !self.has_stash
            && !self.detached
            && !self.error
            && self.branch.is_none()
            && self.attention.is_none()
            && self.ci.is_none()
            && self.min_prs.is_none()
            && self.any_of.is_empty()
            && self.not.is_none()
    }

    /// Whether a single repo satisfies every set constraint. `now` (Unix
    /// seconds) feeds the clock-dependent `attention` check.
    fn matches(&self, repo: &RepoSnapshot, now: i64) -> bool {
        if !self.ids.is_empty() && !self.ids.iter().any(|id| id == &repo.id.0) {
            return false;
        }
        if let Some(pat) = &self.name
            && !glob_ci(pat, &repo.name)
        {
            return false;
        }
        if let Some(pat) = &self.path_glob {
            match &repo.path {
                Some(p) if glob_match(pat, p.as_str()) => {}
                _ => return false,
            }
        }
        if let Some(root) = &self.root {
            match &repo.path {
                Some(p) if path_under(p.as_str(), root) => {}
                _ => return false,
            }
        }
        if let Some(group) = &self.group
            && !repo.groups.iter().any(|g| g == group)
        {
            return false;
        }
        if self.dirty && !repo.is_dirty() {
            return false;
        }
        if self.ahead && repo.ahead() == 0 {
            return false;
        }
        if self.behind && repo.behind() == 0 {
            return false;
        }
        if self.diverged && !(repo.ahead() > 0 && repo.behind() > 0) {
            return false;
        }
        if self.no_upstream && repo.upstream.is_some() {
            return false;
        }
        if self.has_stash && repo.stash_count == 0 {
            return false;
        }
        if self.detached && !matches!(repo.branch, Branch::Detached(_)) {
            return false;
        }
        if self.error && !repo.has_error() {
            return false;
        }
        if let Some(want) = &self.branch {
            match &repo.branch {
                Branch::Named(name) if name == want => {}
                _ => return false,
            }
        }
        if let Some(level) = self.attention
            && attention::assess(repo, now).severity < level.threshold()
        {
            return false;
        }
        if let Some(want) = self.ci {
            match &repo.remote {
                Some(r) if r.ci == want => {}
                _ => return false,
            }
        }
        if let Some(min) = self.min_prs {
            match &repo.remote {
                Some(r) if r.open_prs >= min => {}
                _ => return false,
            }
        }
        if let Some(inner) = &self.not
            && inner.matches(repo, now)
        {
            return false;
        }
        if !self.any_of.is_empty() && !self.any_of.iter().any(|s| s.matches(repo, now)) {
            return false;
        }
        true
    }
}

/// Resolve a selector against the fleet into an ordered `Vec<RepoId>`, in the
/// given sort order. The empty selector resolves to nothing. `now` (Unix
/// seconds) is injected so the `attention` predicate is deterministic.
///
/// These are *read* results: error and path-less repos are included if they
/// match. Callers performing an action should drop those, exactly as the TUI's
/// `action_targets` does (ADR-019).
pub fn resolve(
    repos: &[RepoSnapshot],
    selector: &Selector,
    sort: SortMode,
    now: i64,
) -> Vec<RepoId> {
    if selector.is_empty() {
        return Vec::new();
    }
    let mut indices: Vec<usize> = (0..repos.len())
        .filter(|&i| selector.matches(&repos[i], now))
        .collect();
    sort_indices(repos, &mut indices, sort);
    indices.into_iter().map(|i| repos[i].id.clone()).collect()
}

/// Case-insensitive glob over a name.
fn glob_ci(pattern: &str, text: &str) -> bool {
    glob_match(&pattern.to_lowercase(), &text.to_lowercase())
}

/// Case-insensitive `*`/`?` glob match, exposed so adapters (e.g. group
/// stamping) classify repo names with the *same* matcher selectors use.
pub fn glob_name(pattern: &str, text: &str) -> bool {
    glob_ci(pattern, text)
}

/// A tiny dependency-free glob matcher: `*` matches any sequence (including
/// path separators, so `**` works as a plain "match anything") and `?` matches
/// exactly one character. Linear time with backtracking on `*`.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // The most recent `*` in the pattern and where in `text` we last tried to
    // start matching after it — used to backtrack and consume one more char.
    let mut star: Option<usize> = None;
    let mut resume = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            resume = ti;
            pi += 1;
        } else if let Some(s) = star {
            // Mismatch: let the last `*` swallow one more character of `text`.
            pi = s + 1;
            resume += 1;
            ti = resume;
        } else {
            return false;
        }
    }
    // Any trailing pattern must be all `*` to match the empty remainder.
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Whether `path` is `root` itself or lives beneath it.
fn path_under(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root || path.starts_with(&format!("{root}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Branch, RemoteInfo, RepoSnapshot, Upstream, sample};

    const NOW: i64 = 1_000_000;

    /// A small fleet covering the common states.
    fn fleet() -> Vec<RepoSnapshot> {
        let mut behind = sample("behind", "behind", false, 0, 0, Some(NOW));
        behind.upstream = Some(Upstream {
            name: "origin/main".into(),
            ahead: 0,
            behind: 3,
        });

        let mut diverged = sample("diverged", "diverged", false, 2, 0, Some(NOW));
        diverged.upstream = Some(Upstream {
            name: "origin/main".into(),
            ahead: 2,
            behind: 1,
        });

        let mut detached = sample("detached", "detached", false, 0, 0, Some(NOW));
        detached.branch = Branch::Detached("a1b2c3d".into());

        let errored = RepoSnapshot::errored(
            crate::model::RepoId("broken".into()),
            "broken",
            Some("/repos/broken".into()),
            "unreadable",
        );

        vec![
            sample("clean", "clean", false, 0, 0, Some(NOW)),
            sample("dirty", "payments-api", true, 0, 0, Some(NOW)),
            sample("ahead", "ahead", false, 4, 0, Some(NOW)),
            behind,
            diverged,
            detached,
            errored,
        ]
    }

    fn names(ids: &[RepoId], repos: &[RepoSnapshot]) -> Vec<String> {
        ids.iter()
            .map(|id| {
                repos
                    .iter()
                    .find(|r| &r.id == id)
                    .map(|r| r.id.0.clone())
                    .unwrap()
            })
            .collect()
    }

    fn run(sel: &Selector) -> Vec<String> {
        let repos = fleet();
        let ids = resolve(&repos, sel, SortMode::Name, NOW);
        names(&ids, &repos)
    }

    #[test]
    fn empty_selector_resolves_to_nothing() {
        assert!(Selector::default().is_empty());
        assert_eq!(run(&Selector::default()), Vec::<String>::new());
    }

    #[test]
    fn group_selector_matches_stamped_groups() {
        let mut pay = sample("pay", "payments-api", false, 0, 0, Some(NOW));
        pay.groups = vec!["payments".into()];
        let mut bill = sample("bill", "billing", false, 0, 0, Some(NOW));
        bill.groups = vec!["payments".into()];
        let infra = sample("infra", "infra", false, 0, 0, Some(NOW)); // ungrouped
        let repos = vec![pay, bill, infra];

        let sel = Selector {
            group: Some("payments".into()),
            ..Default::default()
        };
        let mut got: Vec<String> = resolve(&repos, &sel, SortMode::Name, NOW)
            .iter()
            .map(|id| id.0.clone())
            .collect();
        got.sort();
        assert_eq!(got, ["bill", "pay"]);
        // `group` is a real constraint — a group selector isn't "empty".
        assert!(!sel.is_empty());
    }

    #[test]
    fn all_matches_the_whole_fleet() {
        let sel = Selector {
            all: true,
            ..Default::default()
        };
        assert_eq!(run(&sel).len(), fleet().len());
    }

    #[test]
    fn state_predicates() {
        assert_eq!(
            run(&Selector {
                dirty: true,
                ..Default::default()
            }),
            ["dirty"]
        );
        assert_eq!(
            run(&Selector {
                ahead: true,
                ..Default::default()
            }),
            ["ahead", "diverged"]
        );
        assert_eq!(
            run(&Selector {
                behind: true,
                ..Default::default()
            }),
            ["behind", "diverged"]
        );
        assert_eq!(
            run(&Selector {
                diverged: true,
                ..Default::default()
            }),
            ["diverged"]
        );
        assert_eq!(
            run(&Selector {
                detached: true,
                ..Default::default()
            }),
            ["detached"]
        );
        assert_eq!(
            run(&Selector {
                error: true,
                ..Default::default()
            }),
            ["broken"]
        );
    }

    #[test]
    fn unpushed_is_an_alias_for_ahead() {
        let sel: Selector = serde_json::from_str(r#"{"unpushed": true}"#).unwrap();
        assert!(sel.ahead);
    }

    #[test]
    fn name_glob_is_case_insensitive() {
        assert_eq!(
            run(&Selector {
                name: Some("PAY*".into()),
                ..Default::default()
            }),
            ["dirty"] // its name is "payments-api"
        );
    }

    #[test]
    fn path_glob_and_root() {
        assert_eq!(
            run(&Selector {
                path_glob: Some("/repos/**".into()),
                ..Default::default()
            })
            .len(),
            fleet().len()
        );
        assert_eq!(
            run(&Selector {
                root: Some("/repos/clean".into()),
                ..Default::default()
            }),
            ["clean"]
        );
    }

    #[test]
    fn fields_and_together() {
        // dirty AND name → only the dirty repo whose name matches.
        assert_eq!(
            run(&Selector {
                dirty: true,
                name: Some("pay*".into()),
                ..Default::default()
            }),
            ["dirty"]
        );
        // dirty AND behind → nothing in this fleet.
        assert!(
            run(&Selector {
                dirty: true,
                behind: true,
                ..Default::default()
            })
            .is_empty()
        );
    }

    #[test]
    fn combinators_or_and_not() {
        let mut got = run(&Selector {
            any_of: vec![
                Selector {
                    dirty: true,
                    ..Default::default()
                },
                Selector {
                    error: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        got.sort();
        assert_eq!(got, ["broken", "dirty"]);

        // Everything that is NOT ahead.
        let not_ahead = run(&Selector {
            not: Some(Box::new(Selector {
                ahead: true,
                ..Default::default()
            })),
            ..Default::default()
        });
        assert!(!not_ahead.contains(&"ahead".to_string()));
        assert!(!not_ahead.contains(&"diverged".to_string()));
        assert!(not_ahead.contains(&"clean".to_string()));
    }

    #[test]
    fn remote_predicates_need_enrichment() {
        let mut repos = fleet();
        repos[0].remote = Some(RemoteInfo {
            host: "github.com".into(),
            owner: "o".into(),
            repo: "clean".into(),
            default_branch: "main".into(),
            open_prs: 3,
            prs_awaiting_review: 0,
            ci: CiStatus::Failing,
            description: None,
            topics: Vec::new(),
            stars: 0,
            forks: 0,
            watchers: 0,
        });
        let failing = resolve(
            &repos,
            &Selector {
                ci: Some(CiStatus::Failing),
                ..Default::default()
            },
            SortMode::Name,
            NOW,
        );
        assert_eq!(names(&failing, &repos), ["clean"]);

        let prs = resolve(
            &repos,
            &Selector {
                min_prs: Some(1),
                ..Default::default()
            },
            SortMode::Name,
            NOW,
        );
        assert_eq!(names(&prs, &repos), ["clean"]);
    }

    #[test]
    fn output_is_in_sort_order() {
        let repos = fleet();
        let ids = resolve(
            &repos,
            &Selector {
                all: true,
                ..Default::default()
            },
            SortMode::Name,
            NOW,
        );
        // Expected order: every repo, by its name field (the active sort).
        let mut expected: Vec<&RepoSnapshot> = repos.iter().collect();
        expected.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.0.cmp(&b.id.0))
        });
        let expected_ids: Vec<String> = expected.iter().map(|r| r.id.0.clone()).collect();
        assert_eq!(names(&ids, &repos), expected_ids);
    }

    #[test]
    fn glob_matcher_basics() {
        assert!(glob_match("pay*", "payments"));
        assert!(glob_match("*api", "payments-api"));
        assert!(glob_match("p?y", "pay"));
        assert!(glob_match("~/work/**", "~/work/a/b/c"));
        assert!(!glob_match("pay*", "billing"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exactly"));
    }
}

// ── Shorthand parsing ────────────────────────────────────────────────────────
// The human-typed form of the selector language (`dirty,behind`, `name:pay*`),
// shared by every surface so `--select`, an MCP argument, or a web query all
// mean the same thing. JSON selectors need no parser here — `Selector` already
// derives `Deserialize`, and core deliberately carries no serde_json dep.

/// A shorthand token (or value) the parser didn't recognize.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseSelectorError(pub String);

impl std::fmt::Display for ParseSelectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ParseSelectorError {}

/// Parse comma-separated shorthand tokens into a [`Selector`] (tokens AND
/// together). Bare flags (`dirty`, `behind`, …) set booleans; `key:value`
/// tokens (`name:pay*`, `ci:failing`, `prs:1`, `branch:main`, `root:~/work`)
/// set the scoped predicates; a few named views (`clean`, `attention`) expand
/// to their selectors.
pub fn parse_shorthand(query: &str) -> Result<Selector, ParseSelectorError> {
    let mut sel = Selector::default();
    for raw in query.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if let Some((key, value)) = token.split_once(':') {
            let value = value.trim().to_string();
            match key.trim() {
                "name" => sel.name = Some(value),
                "branch" => sel.branch = Some(value),
                "root" => sel.root = Some(value),
                "path" => sel.path_glob = Some(value),
                "group" => sel.group = Some(value),
                "ci" => sel.ci = Some(parse_ci(&value)?),
                "prs" | "min-prs" => {
                    sel.min_prs = Some(value.parse().map_err(|_| {
                        ParseSelectorError(format!("`prs:` expects a number, got `{value}`"))
                    })?)
                }
                "attention" => sel.attention = Some(parse_attention(&value)?),
                other => {
                    return Err(ParseSelectorError(format!(
                        "unknown selector key `{other}:`"
                    )));
                }
            }
            continue;
        }
        match token {
            "all" => sel.all = true,
            "dirty" => sel.dirty = true,
            "ahead" | "unpushed" => sel.ahead = true,
            "behind" => sel.behind = true,
            "diverged" => sel.diverged = true,
            "no-upstream" => sel.no_upstream = true,
            "stash" | "has-stash" => sel.has_stash = true,
            "detached" => sel.detached = true,
            "error" | "errors" => sel.error = true,
            "attention" | "needs-attention" => sel.attention = Some(AttentionLevel::Any),
            "prs-open" => sel.min_prs = Some(1),
            "red-ci" => sel.ci = Some(CiStatus::Failing),
            // "clean" = nothing the attention layer would flag and readable.
            "clean" => {
                sel.not = Some(Box::new(Selector {
                    any_of: vec![
                        Selector {
                            dirty: true,
                            ..Default::default()
                        },
                        Selector {
                            ahead: true,
                            ..Default::default()
                        },
                        Selector {
                            behind: true,
                            ..Default::default()
                        },
                        Selector {
                            has_stash: true,
                            ..Default::default()
                        },
                        Selector {
                            error: true,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }))
            }
            other => {
                return Err(ParseSelectorError(format!(
                    "unknown selector `{other}` (try: dirty, behind, ahead, attention, clean, name:<glob>)"
                )));
            }
        }
    }
    Ok(sel)
}

fn parse_ci(value: &str) -> Result<CiStatus, ParseSelectorError> {
    Ok(match value {
        "passing" => CiStatus::Passing,
        "failing" => CiStatus::Failing,
        "pending" => CiStatus::Pending,
        other => {
            return Err(ParseSelectorError(format!(
                "unknown ci status `{other}` (passing | failing | pending)"
            )));
        }
    })
}

fn parse_attention(value: &str) -> Result<AttentionLevel, ParseSelectorError> {
    Ok(match value {
        "any" => AttentionLevel::Any,
        "notice" => AttentionLevel::Notice,
        "warn" => AttentionLevel::Warn,
        "risk" => AttentionLevel::Risk,
        other => {
            return Err(ParseSelectorError(format!(
                "unknown attention level `{other}` (any | notice | warn | risk)"
            )));
        }
    })
}

/// Expand a leading `~` in path predicates against `home`, recursing into
/// combinators. `home` is passed in (read from `$HOME` by the caller) so core
/// stays free of environment access.
pub fn expand_tilde(sel: &mut Selector, home: Option<&str>) {
    if let Some(home) = home.filter(|h| !h.is_empty()) {
        expand_one(&mut sel.root, home);
        expand_one(&mut sel.path_glob, home);
    }
    for inner in &mut sel.any_of {
        expand_tilde(inner, home);
    }
    if let Some(inner) = &mut sel.not {
        expand_tilde(inner, home);
    }
}

fn expand_one(field: &mut Option<String>, home: &str) {
    if let Some(value) = field {
        if let Some(rest) = value.strip_prefix("~/") {
            *value = format!("{home}/{rest}");
        } else if value == "~" {
            *value = home.to_string();
        }
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn shorthand_flags_and_combine() {
        let sel = parse_shorthand("dirty,behind").unwrap();
        assert!(sel.dirty && sel.behind);
        assert!(!sel.ahead);
    }

    #[test]
    fn shorthand_key_values() {
        let sel = parse_shorthand("name:pay*,ci:failing,prs:2,branch:main").unwrap();
        assert_eq!(sel.name.as_deref(), Some("pay*"));
        assert_eq!(sel.ci, Some(CiStatus::Failing));
        assert_eq!(sel.min_prs, Some(2));
        assert_eq!(sel.branch.as_deref(), Some("main"));
    }

    #[test]
    fn unpushed_aliases_ahead() {
        assert!(parse_shorthand("unpushed").unwrap().ahead);
    }

    #[test]
    fn clean_is_negation_of_attention_states() {
        let sel = parse_shorthand("clean").unwrap();
        let not = sel.not.expect("clean sets `not`");
        assert_eq!(not.any_of.len(), 5);
    }

    #[test]
    fn tilde_expands_against_home() {
        let mut sel = Selector {
            root: Some("~/work".into()),
            path_glob: Some("~/oss/**".into()),
            ..Default::default()
        };
        expand_tilde(&mut sel, Some("/home/test"));
        assert_eq!(sel.root.as_deref(), Some("/home/test/work"));
        assert_eq!(sel.path_glob.as_deref(), Some("/home/test/oss/**"));
    }

    #[test]
    fn unknown_token_errors() {
        assert!(parse_shorthand("frobnicate").is_err());
        assert!(parse_shorthand("ci:sideways").is_err());
    }
}
