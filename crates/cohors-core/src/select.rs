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
    /// Glob over the canonical path, e.g. `~/work/**` (expand `~` before
    /// resolving). `*`/`?` stay within one path segment; `**` crosses `/`.
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
    /// Repos with unmerged (conflicted) files.
    pub conflicted: bool,
    /// Repos with a multi-step git operation (merge/rebase/…) in progress.
    pub in_operation: bool,
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
            && !self.conflicted
            && !self.in_operation
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
        if self.conflicted && repo.worktree.conflicted == 0 {
            return false;
        }
        if self.in_operation && repo.operation.is_none() {
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

/// Case-insensitive glob over a name. Uses Rust's locale-independent Unicode
/// lowercasing on both sides, so the result is the same on every machine
/// (no Turkish-İ style locale surprises).
fn glob_ci(pattern: &str, text: &str) -> bool {
    glob_match(&pattern.to_lowercase(), &text.to_lowercase())
}

/// Case-insensitive `*`/`?` glob match, exposed so adapters (e.g. group
/// stamping) classify repo names with the *same* matcher selectors use.
pub fn glob_name(pattern: &str, text: &str) -> bool {
    glob_ci(pattern, text)
}

/// One compiled glob token: a literal char, `?` (one char, not `/`), `*` (any
/// run within one path segment), or `**` (any run, crossing `/`).
enum GlobTok {
    Lit(char),
    One,
    Star,
    StarStar,
}

/// A tiny dependency-free glob matcher with path-aware semantics: `?` matches
/// exactly one character except `/`, `*` matches any run of characters within
/// one path segment (never crossing `/`), and `**` matches any run including
/// separators. For repo *names* (which carry no `/`) `*` and `**` behave the
/// same. Implemented as a token-by-token DP sweep — O(pattern × text) worst
/// case with no backtracking, so no pattern can blow it up.
fn glob_match(pattern: &str, text: &str) -> bool {
    // Compile the pattern, collapsing runs of `*` (three-or-more reads as `**`).
    let mut toks: Vec<GlobTok> = Vec::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                let mut stars = 1;
                while chars.peek() == Some(&'*') {
                    chars.next();
                    stars += 1;
                }
                toks.push(if stars >= 2 {
                    GlobTok::StarStar
                } else {
                    GlobTok::Star
                });
            }
            '?' => toks.push(GlobTok::One),
            c => toks.push(GlobTok::Lit(c)),
        }
    }

    let t: Vec<char> = text.chars().collect();
    // dp[j] = "the tokens consumed so far can match text[..j]".
    let mut dp = vec![false; t.len() + 1];
    dp[0] = true;
    for tok in &toks {
        let mut next = vec![false; t.len() + 1];
        match tok {
            GlobTok::StarStar => {
                // Matches any (possibly empty) suffix extension of every
                // reachable prefix.
                let mut reachable = false;
                for j in 0..=t.len() {
                    reachable |= dp[j];
                    next[j] = reachable;
                }
            }
            GlobTok::Star => {
                // Same, but the extension may not cross a `/`.
                for j in 0..=t.len() {
                    next[j] = dp[j] || (j > 0 && next[j - 1] && t[j - 1] != '/');
                }
            }
            GlobTok::One => {
                for j in 1..=t.len() {
                    next[j] = dp[j - 1] && t[j - 1] != '/';
                }
            }
            GlobTok::Lit(c) => {
                for j in 1..=t.len() {
                    next[j] = dp[j - 1] && t[j - 1] == *c;
                }
            }
        }
        dp = next;
    }
    dp[t.len()]
}

/// Whether `path` is `root` itself or lives beneath it. An empty root matches
/// nothing (a blank `root:` must not silently select the fleet); `/` matches
/// every absolute path. Trailing slashes on the root are ignored.
fn path_under(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return path.starts_with('/'); // the root was literally "/"
    }
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
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
            open_prs_by_base: Default::default(),
            prs_awaiting_review: 0,
            ci: CiStatus::Failing,
            ci_by_branch: Default::default(),
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

    #[test]
    fn glob_star_stays_within_one_path_segment() {
        // `*` and `?` never cross `/`; `**` does. Repo names carry no `/`,
        // so name globs are unaffected.
        assert!(glob_match("/repos/*", "/repos/api"));
        assert!(!glob_match("/repos/*", "/repos/api/sub"));
        assert!(glob_match("/repos/**", "/repos/api/sub"));
        assert!(glob_match("/repos/*/sub", "/repos/api/sub"));
        assert!(!glob_match("/repos/?pi", "/repos/api/x"));
        // Three-or-more stars collapse to `**`.
        assert!(glob_match("/repos/***", "/repos/a/b"));
    }

    #[test]
    fn glob_edge_cases_never_panic_or_overmatch() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "x"));
        assert!(glob_match("**", ""));
        assert!(glob_match("*", ""));
        assert!(!glob_match("?", ""));
        // A repeat-heavy pattern that blows up naive backtracking matchers
        // resolves instantly under the DP sweep.
        assert!(!glob_match("*a*a*a*a*a*a*a*a*a*b", &"a".repeat(64)));
        // Unicode: case-insensitive matching folds both sides identically on
        // every machine (locale-independent lowercasing).
        assert!(glob_name("CAFÉ*", "café-infra"));
        // ß lowercases to itself while SS lowercases to "ss" — simple folding,
        // not full case-equivalence, and the same answer on every machine.
        assert!(!glob_name("straße", "STRASSE"));
    }

    #[test]
    fn path_under_edges() {
        assert!(path_under("/repos/api", "/repos"));
        assert!(path_under("/repos/api", "/repos/"));
        assert!(path_under("/repos", "/repos"));
        // Prefix of a *component* is not containment.
        assert!(!path_under("/repos-archive/api", "/repos"));
        // "/" contains every absolute path; "" contains nothing.
        assert!(path_under("/anything", "/"));
        assert!(!path_under("/anything", ""));
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
/// tokens (`name:pay*`, `id:payments-api`, `ci:failing`, `prs:1`,
/// `branch:main`, `root:~/work`) set the scoped predicates; a few named views
/// (`clean`, `attention`) expand to their selectors. A `!` prefix negates any
/// token (`!dirty`, `!name:pay*`) — negations AND in, so `dirty,!behind` reads
/// "dirty and not behind".
pub fn parse_shorthand(query: &str) -> Result<Selector, ParseSelectorError> {
    let mut sel = Selector::default();
    // Negated tokens accumulate here and fold into `sel.not` at the end:
    // NOT(a) AND NOT(b) is NOT(a OR b), which the single `not` slot expresses.
    let mut negated: Vec<Selector> = Vec::new();
    for raw in query.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(rest) = token.strip_prefix('!') {
            let rest = rest.trim();
            if rest.is_empty() {
                return Err(ParseSelectorError(
                    "`!` needs a token to negate, e.g. `!dirty`".to_string(),
                ));
            }
            negated.push(parse_shorthand(rest)?);
            continue;
        }
        if let Some((key, value)) = token.split_once(':') {
            let value = value.trim().to_string();
            match key.trim() {
                "name" => sel.name = Some(value),
                "id" => sel.ids.push(value),
                "branch" => sel.branch = Some(value),
                "root" => sel.root = Some(value),
                "path" => sel.path_glob = Some(value),
                "group" => sel.group = Some(value),
                // `FromStr` on CiStatus is the same case-insensitive vocabulary
                // the wire uses, so `ci:none` (no CI configured) works too.
                "ci" => {
                    sel.ci = Some(value.parse().map_err(|_| {
                        ParseSelectorError(format!(
                            "unknown ci status `{value}` (passing | failing | pending | none)"
                        ))
                    })?)
                }
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
            "conflicted" | "conflicts" => sel.conflicted = true,
            "operation" | "in-progress" => sel.in_operation = true,
            "error" | "errors" => sel.error = true,
            "attention" | "needs-attention" => sel.attention = Some(AttentionLevel::Any),
            "prs-open" => sel.min_prs = Some(1),
            "red-ci" => sel.ci = Some(CiStatus::Failing),
            // "clean" = nothing the attention layer would flag and readable —
            // expressed as negations so it composes with explicit `!` tokens.
            "clean" => {
                for flagged in [
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
                ] {
                    negated.push(flagged);
                }
            }
            other => {
                return Err(ParseSelectorError(format!(
                    "unknown selector `{other}` (try: dirty, behind, ahead, conflicted, operation, attention, clean, name:<glob>, or !token to negate)"
                )));
            }
        }
    }
    if !negated.is_empty() {
        sel.not = Some(Box::new(if negated.len() == 1 {
            negated.pop().expect("len checked")
        } else {
            Selector {
                any_of: negated,
                ..Default::default()
            }
        }));
    }
    Ok(sel)
}

fn parse_attention(value: &str) -> Result<AttentionLevel, ParseSelectorError> {
    Ok(match value.to_ascii_lowercase().as_str() {
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
    fn conflicted_and_operation_predicates() {
        let sel = parse_shorthand("conflicted,operation").unwrap();
        assert!(sel.conflicted && sel.in_operation);
        let mut repos = fleet_for_new_predicates();
        repos[0].worktree.conflicted = 2;
        repos[1].operation = Some(crate::model::RepoOperation::Rebase);
        let hits = resolve(
            &repos,
            &Selector {
                conflicted: true,
                ..Default::default()
            },
            SortMode::Name,
            0,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, repos[0].id.0);
        let hits = resolve(
            &repos,
            &Selector {
                in_operation: true,
                ..Default::default()
            },
            SortMode::Name,
            0,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, repos[1].id.0);
    }

    fn fleet_for_new_predicates() -> Vec<RepoSnapshot> {
        vec![
            crate::model::sample("a", "a", false, 0, 0, Some(1)),
            crate::model::sample("b", "b", false, 0, 0, Some(1)),
        ]
    }

    #[test]
    fn unknown_token_errors() {
        assert!(parse_shorthand("frobnicate").is_err());
        assert!(parse_shorthand("ci:sideways").is_err());
        assert!(parse_shorthand("!").is_err());
    }

    #[test]
    fn bang_negates_flags_and_key_values() {
        // One negation lands directly in `not`.
        let sel = parse_shorthand("dirty,!behind").unwrap();
        assert!(sel.dirty);
        assert!(sel.not.as_ref().unwrap().behind);

        // Several negations OR inside `not` (NOT a AND NOT b == NOT (a OR b)).
        let sel = parse_shorthand("!dirty,!name:pay*").unwrap();
        let not = sel.not.unwrap();
        assert_eq!(not.any_of.len(), 2);
        assert!(not.any_of.iter().any(|s| s.dirty));
        assert!(not.any_of.iter().any(|s| s.name.as_deref() == Some("pay*")));
    }

    #[test]
    fn id_key_and_ci_none() {
        let sel = parse_shorthand("id:payments-api,id:billing").unwrap();
        assert_eq!(sel.ids, ["payments-api", "billing"]);
        // `ci:` speaks the full wire vocabulary, case-insensitively.
        assert_eq!(parse_shorthand("ci:none").unwrap().ci, Some(CiStatus::None));
        assert_eq!(
            parse_shorthand("ci:FAILING").unwrap().ci,
            Some(CiStatus::Failing)
        );
        assert_eq!(
            parse_shorthand("attention:WARN").unwrap().attention,
            Some(AttentionLevel::Warn)
        );
    }
}
