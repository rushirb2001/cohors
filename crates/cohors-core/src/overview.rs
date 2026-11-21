//! The fleet overview — one model that every front-end renders.
//!
//! The web dashboard, the terminal UI, the editor extension and the MCP server
//! all answer the same question: *what is the state of my repositories and what
//! should I do next?* Before this module each of them worked the answer out
//! separately, so the numbers could disagree. Now there is exactly one
//! computation, [`overview`], and each surface only decides how to lay it out.
//!
//! Like the rest of `cohors-core` it is pure: it takes snapshots, launchpad
//! entries and the current time as arguments and reads nothing itself, so it
//! compiles to WebAssembly and tests run without a disk.
//!
//! What it contains, in the order a person reads it:
//!
//! 1. [`Overview::actions`] — "Do next": the few things worth doing, most
//!    urgent first, each with the repositories it applies to.
//! 2. [`Overview::figures`] — the key numbers at a glance.
//! 3. [`Overview::repos`] — one [`RepoLine`] per repository, most urgent first.
//!    Lines with [`RepoLine::at_risk`] set are the "work at risk" table.
//! 4. [`Overview::work`] and [`Overview::commits`] — open tasks and the last
//!    seven days of commits, per repository.

use serde::{Deserialize, Serialize};

use crate::attention::{
    AttentionReason, FleetSummary, STALE_FETCH_SECS, Severity, assess, fleet_summary,
};
use crate::launchpad::{Board, FocusItem, LaunchpadEntry};
use crate::model::{CommitMeta, RepoSnapshot};

const DAY: i64 = 24 * 60 * 60;

/// Everything a front-end needs to draw the overview. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overview {
    /// The clock the judgments were made against (Unix seconds).
    pub now: i64,
    /// One sentence that sums the fleet up, e.g. `28 repositories. 7 have work
    /// at risk, 3 are clean.`
    pub headline: String,
    /// Counts by severity and by kind of trouble.
    pub summary: FleetSummary,
    /// The key numbers.
    pub figures: KeyFigures,
    /// "Do next", most urgent first. Empty when nothing needs doing.
    pub actions: Vec<NextAction>,
    /// Every repository, most urgent first, then by name.
    pub repos: Vec<RepoLine>,
    /// Repositories that have open tasks, most open tasks first.
    pub work: Vec<RepoWork>,
    /// Repositories with commits in the last seven days, most commits first.
    pub commits: Vec<RepoCommits>,
    /// Commits per week across the whole fleet, oldest week first; the last
    /// entry is the most recent seven days.
    pub activity: Vec<u32>,
    /// The cross-repository focus board.
    pub focus: Vec<FocusItem>,
}

impl Overview {
    /// The repositories where local work could be lost: the "work at risk" table.
    pub fn at_risk(&self) -> impl Iterator<Item = &RepoLine> {
        self.repos.iter().filter(|r| r.at_risk)
    }
}

/// The key numbers. Every count of "repositories" is a number of repositories;
/// counts of files, commits, stashes and tasks say so in their name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyFigures {
    pub repos: usize,
    /// Repositories with nothing to report at all.
    pub clean: usize,
    /// Repositories at warning level or above.
    pub at_risk: usize,
    /// Repositories whose current branch has a remote but was never pushed.
    pub unpublished: usize,
    /// Repositories with commits that are not on the remote (diverged
    /// included, and commits held only in a linked worktree).
    pub unpushed_repos: usize,
    /// How many commits those are.
    pub unpushed_commits: u32,
    /// `unpublished + unpushed_repos`: work that exists only on this machine.
    pub not_backed_up: usize,
    pub behind_repos: usize,
    pub behind_commits: u32,
    pub stashes: u32,
    pub stash_repos: usize,
    /// Of `stash_repos`, how many have a newest stash older than a week.
    pub stale_stash_repos: usize,
    pub uncommitted_files: u32,
    pub dirty_repos: usize,
    /// Repositories not fetched for longer than the stale-fetch threshold.
    pub stale_fetch_repos: usize,
    /// Days since the least recently fetched of those was fetched.
    pub oldest_fetch_days: i64,
    pub open_tasks: usize,
    pub high_tasks: usize,
    pub doing_tasks: usize,
    pub blocked_tasks: usize,
    /// Repositories that have at least one open task.
    pub task_repos: usize,
    /// Repositories with a launchpad file.
    pub configured_repos: usize,
    /// Commits in the last seven days.
    pub commits_week: u32,
    /// Commits in the seven days before that.
    pub commits_prev_week: u32,
    /// Commits across every week in `Overview::activity`.
    pub commits_total: u32,
    /// Repositories with at least one commit in the last seven days.
    pub active_repos: usize,
    /// Linked worktrees (`git worktree add`) across every repository.
    pub worktrees: usize,
    /// Of `worktrees`, how many hold uncommitted files or unpushed commits.
    pub worktrees_holding_work: usize,
    /// Uncommitted files in linked worktrees (not part of `uncommitted_files`,
    /// which counts the main checkouts).
    pub worktree_files: u32,
    /// Unpushed commits in linked worktrees. Part of `unpushed_commits`; a
    /// repository whose only unpushed work is in a worktree is part of
    /// `unpushed_repos`, and so of `not_backed_up`.
    pub worktree_commits: u32,
}

/// What kind of step a [`NextAction`] is. Front-ends use it to pick an icon and
/// a key; it never changes meaning between surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Unreadable,
    Resolve,
    Push,
    Reconcile,
    Publish,
    ReviewStash,
    Pull,
    Commit,
    Fetch,
    StartTask,
    /// Work sitting in linked worktrees: unpushed commits, uncommitted files.
    Worktrees,
}

/// One line of "Do next".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextAction {
    pub kind: ActionKind,
    pub severity: Severity,
    /// What to do: `Publish 7 branches that have no upstream`.
    pub title: String,
    /// Why it matters, and where: `Not backed up anywhere: a, b, c and 4 more.`
    pub why: String,
    /// The button text: `Publish 7`.
    pub label: String,
    /// The action-registry verb that carries it out (`push`, `pull`, `fetch`),
    /// or `None` when a person has to look first (review a stash, resolve a
    /// conflict). A surface with a verb can run it over `repos` directly.
    pub verb: Option<String>,
    /// The ids of the repositories this applies to.
    pub repos: Vec<String>,
}

/// The single most useful thing to do with one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestedAction {
    /// Registry verb, or `None` when the step is to open the repository.
    pub verb: Option<String>,
    pub label: String,
}

/// One repository as the overview shows it: state, judgment and next step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoLine {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    /// The branch label (`main`, `@a1b2c3d (detached)`, `(no commits)`).
    pub branch: String,
    /// The tracking branch, e.g. `origin/main`. `None` when there is none.
    pub upstream: Option<String>,
    /// Whether the repository has a remote at all.
    pub has_remote: bool,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
    pub conflicted: u32,
    pub stashes: u32,
    pub severity: Severity,
    /// Warning level or above: local work here could be lost.
    pub at_risk: bool,
    /// Every reason in plain words, most urgent first.
    pub reasons: Vec<String>,
    pub action: Option<SuggestedAction>,
    pub commit: Option<CommitMeta>,
    /// When the remote was last fetched (Unix seconds).
    pub fetched: Option<i64>,
    pub tasks_open: usize,
    pub tasks_high: usize,
    /// Whether the repository has a launchpad file.
    pub configured: bool,
    /// Commits per week, oldest first, as in [`RepoSnapshot::activity`].
    pub activity: Vec<u8>,
    pub groups: Vec<String>,
    /// Why the repository could not be read, when it could not.
    pub error: Option<String>,
}

impl RepoLine {
    /// Uncommitted files of every kind.
    pub fn changed(&self) -> u32 {
        self.staged + self.modified + self.untracked + self.conflicted
    }
}

/// Open tasks in one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoWork {
    pub id: String,
    pub name: String,
    pub open: usize,
    pub high: usize,
    pub doing: usize,
    pub blocked: usize,
    /// How many items on the focus board point at this repository.
    pub focus: usize,
}

/// Commits in the last seven days in one repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoCommits {
    pub id: String,
    pub name: String,
    pub count: u32,
}

/// The single most useful step for one reason, if there is one.
pub fn suggest(reason: &AttentionReason) -> Option<SuggestedAction> {
    let (verb, label) = match reason {
        AttentionReason::Unpushed { .. } => (Some("push"), "Push"),
        AttentionReason::Unpublished => (Some("push"), "Publish branch"),
        AttentionReason::Behind { .. } => (Some("pull"), "Pull"),
        AttentionReason::StaleRemote { .. } => (Some("fetch"), "Fetch"),
        AttentionReason::Conflicted { .. } => (None, "Resolve conflicts"),
        AttentionReason::Operation(_) => (None, "Finish operation"),
        AttentionReason::Diverged { .. } => (None, "Reconcile"),
        AttentionReason::Stash { .. } => (None, "Review stash"),
        AttentionReason::Uncommitted { .. } => (None, "Review changes"),
        AttentionReason::WorktreeUnpushed { .. } | AttentionReason::WorktreeUncommitted { .. } => {
            (None, "Review worktree")
        }
        AttentionReason::Unreadable
        | AttentionReason::Detached
        | AttentionReason::WorktreeMissing { .. } => return None,
    };
    Some(SuggestedAction {
        verb: verb.map(str::to_string),
        label: label.to_string(),
    })
}

/// Commits per week across the fleet, oldest first. Repositories can carry
/// histories of different lengths (an old cache, a repo read without history),
/// so they are lined up at the *end*: the last entry of every repository is its
/// most recent week.
pub fn fleet_activity(repos: &[RepoSnapshot]) -> Vec<u32> {
    let weeks = repos.iter().map(|r| r.activity.len()).max().unwrap_or(0);
    let mut total = vec![0u32; weeks];
    for repo in repos {
        let offset = weeks - repo.activity.len();
        for (i, count) in repo.activity.iter().enumerate() {
            total[offset + i] += u32::from(*count);
        }
    }
    total
}

/// `7 branches` / `1 branch`.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `a, b, c and 4 more` — at most three names, so a sentence stays a sentence.
fn names(list: &[&str]) -> String {
    const SHOWN: usize = 3;
    let shown = list.iter().take(SHOWN).copied().collect::<Vec<_>>();
    match list.len() {
        0 => String::new(),
        n if n <= SHOWN => match shown.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
            _ => shown.join(""),
        },
        n => format!("{} and {} more", shown.join(", "), n - SHOWN),
    }
}

/// What "Do next" needs to know about one repository while the list is built.
struct Seen<'a> {
    id: &'a str,
    name: &'a str,
    reasons: Vec<AttentionReason>,
}

/// Collects the repositories that share one kind of trouble.
#[derive(Default)]
struct Bucket<'a> {
    ids: Vec<String>,
    names: Vec<&'a str>,
}

impl<'a> Bucket<'a> {
    fn push(&mut self, seen: &Seen<'a>) {
        self.ids.push(seen.id.to_string());
        self.names.push(seen.name);
    }
    fn len(&self) -> usize {
        self.ids.len()
    }
    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// Build the overview of `repos` as of `now` (Unix seconds).
///
/// `entries` are the launchpad entries for those repositories (matched by id);
/// pass an empty slice when the launchpad was not read. `board` is the focus
/// board. Nothing here touches the disk or the clock.
pub fn overview(
    repos: &[RepoSnapshot],
    entries: &[LaunchpadEntry],
    board: &Board,
    now: i64,
) -> Overview {
    let entry_of = |id: &str| entries.iter().find(|e| e.id == id);
    let mut figures = KeyFigures {
        repos: repos.len(),
        ..Default::default()
    };

    // ── One pass over the repositories: lines, figures, and "Do next" buckets.
    let mut lines = Vec::with_capacity(repos.len());
    let mut seen = Vec::with_capacity(repos.len());
    for repo in repos {
        let assessment = assess(repo, now);
        let entry = entry_of(&repo.id.0);
        let open = entry.map(|e| e.open_tasks()).unwrap_or_default();
        let high = open
            .iter()
            .filter(|t| t.priority.as_deref() == Some("high"))
            .count();

        if assessment.reasons.is_empty() {
            figures.clean += 1;
        }
        if assessment.severity >= Severity::Warn {
            figures.at_risk += 1;
        }
        let week = repo.activity.last().copied().unwrap_or(0);
        if week > 0 {
            figures.active_repos += 1;
        }
        figures.worktrees += repo.worktrees.len();
        figures.worktrees_holding_work += repo.worktrees.iter().filter(|w| w.holds_work()).count();

        lines.push(RepoLine {
            id: repo.id.0.clone(),
            name: repo.name.clone(),
            path: repo.path.as_ref().map(|p| p.to_string()),
            branch: repo.branch.label(),
            upstream: repo.upstream.as_ref().map(|u| u.name.clone()),
            has_remote: repo.remote_url.is_some(),
            ahead: repo.ahead(),
            behind: repo.behind(),
            staged: repo.worktree.staged,
            modified: repo.worktree.modified,
            untracked: repo.worktree.untracked,
            conflicted: repo.worktree.conflicted,
            stashes: repo.stash_count,
            severity: assessment.severity,
            at_risk: assessment.severity >= Severity::Warn,
            reasons: assessment
                .reasons
                .iter()
                .map(AttentionReason::phrase)
                .collect(),
            action: assessment.reasons.iter().find_map(suggest),
            commit: repo.last_commit.clone(),
            fetched: repo.last_fetch,
            tasks_open: open.len(),
            tasks_high: high,
            configured: entry.is_some_and(|e| e.configured),
            activity: repo.activity.clone(),
            groups: repo.groups.clone(),
            error: repo.error.clone(),
        });
        seen.push(Seen {
            id: &repo.id.0,
            name: &repo.name,
            reasons: assessment.reasons,
        });
    }
    // Most urgent first; within a level, alphabetical without regard to case.
    lines.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    // ── "Do next" buckets, one per kind of trouble.
    let mut unreadable = Bucket::default();
    let mut blocked = Bucket::default();
    let mut unpushed = Bucket::default();
    let mut diverged = Bucket::default();
    let mut unpublished = Bucket::default();
    let mut stale_stash = Bucket::default();
    let mut behind = Bucket::default();
    let mut dirty = Bucket::default();
    let mut stale_fetch = Bucket::default();
    let mut worktree_work = Bucket::default();
    let mut unpushed_aging = false;
    let mut worktree_aging = false;
    // Repositories whose only unpushed work is in a linked worktree.
    let mut worktree_only_repos = 0usize;
    let mut largest: Option<(&str, u32)> = None;

    for repo in &seen {
        // A repository stopped mid-merge shows up once, however many reasons say so.
        let mut is_blocked = false;
        // Likewise one with work in its worktrees, whether commits, files or both.
        let mut has_worktree_work = false;
        for reason in &repo.reasons {
            match reason {
                AttentionReason::Unreadable => unreadable.push(repo),
                AttentionReason::Conflicted { .. } | AttentionReason::Operation(_) => {
                    is_blocked = true;
                }
                AttentionReason::Unpushed { commits, aging } => {
                    figures.unpushed_repos += 1;
                    figures.unpushed_commits += commits;
                    unpushed_aging |= aging;
                    unpushed.push(repo);
                }
                AttentionReason::Diverged { ahead, behind } => {
                    figures.unpushed_repos += 1;
                    figures.unpushed_commits += ahead;
                    figures.behind_repos += 1;
                    figures.behind_commits += behind;
                    diverged.push(repo);
                }
                AttentionReason::Behind { commits } => {
                    figures.behind_repos += 1;
                    figures.behind_commits += commits;
                    behind.push(repo);
                }
                AttentionReason::Unpublished => {
                    figures.unpublished += 1;
                    unpublished.push(repo);
                }
                AttentionReason::Uncommitted {
                    staged,
                    modified,
                    untracked,
                } => {
                    let files = staged + modified + untracked;
                    figures.dirty_repos += 1;
                    figures.uncommitted_files += files;
                    if largest.is_none_or(|(_, most)| files > most) {
                        largest = Some((repo.name, files));
                    }
                    dirty.push(repo);
                }
                AttentionReason::Stash { count, stale } => {
                    figures.stash_repos += 1;
                    figures.stashes += count;
                    if *stale {
                        figures.stale_stash_repos += 1;
                        stale_stash.push(repo);
                    }
                }
                AttentionReason::StaleRemote { days } => {
                    figures.stale_fetch_repos += 1;
                    figures.oldest_fetch_days = figures.oldest_fetch_days.max(*days);
                    stale_fetch.push(repo);
                }
                AttentionReason::Detached | AttentionReason::WorktreeMissing { .. } => {}
                AttentionReason::WorktreeUnpushed { commits, aging, .. } => {
                    figures.worktree_commits += commits;
                    worktree_aging |= aging;
                    has_worktree_work = true;
                }
                AttentionReason::WorktreeUncommitted { files, .. } => {
                    figures.worktree_files += files;
                    has_worktree_work = true;
                }
            }
        }
        if is_blocked {
            blocked.push(repo);
        }
        if has_worktree_work {
            // Not backed up, unless the repository is already counted for its
            // main checkout.
            let counted = repo.reasons.iter().any(|r| {
                matches!(
                    r,
                    AttentionReason::Unpushed { .. }
                        | AttentionReason::Diverged { .. }
                        | AttentionReason::Unpublished
                )
            });
            let unpushed_there = repo
                .reasons
                .iter()
                .any(|r| matches!(r, AttentionReason::WorktreeUnpushed { .. }));
            worktree_only_repos += usize::from(unpushed_there && !counted);
            worktree_work.push(repo);
        }
    }
    // A repository whose only unpushed commits sit in a worktree has commits
    // that are not on the remote like any other, so every surface's "not
    // backed up" wording covers it without a new case.
    figures.unpushed_repos += worktree_only_repos;
    figures.not_backed_up = figures.unpublished + figures.unpushed_repos;

    // ── Tasks.
    let mut work: Vec<RepoWork> = Vec::new();
    for entry in entries {
        figures.configured_repos += usize::from(entry.configured);
        let open = entry.open_tasks();
        if open.is_empty() {
            continue;
        }
        let with_status = |status: &str| open.iter().filter(|t| t.status() == status).count();
        let item = RepoWork {
            id: entry.id.clone(),
            name: entry.name.clone(),
            open: open.len(),
            high: open
                .iter()
                .filter(|t| t.priority.as_deref() == Some("high"))
                .count(),
            doing: with_status("doing"),
            blocked: with_status("blocked"),
            focus: board
                .focus
                .iter()
                .filter(|f| f.repo.as_deref() == Some(entry.name.as_str()))
                .count(),
        };
        figures.open_tasks += item.open;
        figures.high_tasks += item.high;
        figures.doing_tasks += item.doing;
        figures.blocked_tasks += item.blocked;
        figures.task_repos += 1;
        work.push(item);
    }
    work.sort_by(|a, b| b.open.cmp(&a.open).then_with(|| a.name.cmp(&b.name)));

    // ── Commits.
    let activity = fleet_activity(repos);
    figures.commits_total = activity.iter().sum();
    figures.commits_week = activity.last().copied().unwrap_or(0);
    figures.commits_prev_week = activity.iter().rev().nth(1).copied().unwrap_or(0);
    let mut commits: Vec<RepoCommits> = repos
        .iter()
        .filter_map(|r| {
            let count = u32::from(r.activity.last().copied().unwrap_or(0));
            (count > 0).then(|| RepoCommits {
                id: r.id.0.clone(),
                name: r.name.clone(),
                count,
            })
        })
        .collect();
    commits.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));

    // ── "Do next", written out. The order here is the order of urgency; the
    // stable sort below only lifts the highest severities to the top.
    let mut actions = Vec::new();
    let mut add =
        |kind, severity, title: String, why: String, label: String, verb, bucket: Bucket| {
            actions.push(NextAction {
                kind,
                severity,
                title,
                why,
                label,
                verb: Option::<&str>::map(verb, str::to_string),
                repos: bucket.ids,
            });
        };
    if !unreadable.is_empty() {
        let n = unreadable.len();
        add(
            ActionKind::Unreadable,
            Severity::Risk,
            format!(
                "Check {}",
                count(
                    n,
                    "repository that could not be read",
                    "repositories that could not be read"
                )
            ),
            format!("Nothing else is known about {}.", names(&unreadable.names)),
            "Open".to_string(),
            None,
            unreadable,
        );
    }
    if !blocked.is_empty() {
        let n = blocked.len();
        add(
            ActionKind::Resolve,
            Severity::Risk,
            format!(
                "Finish the merge or rebase paused in {}",
                count(n, "repository", "repositories")
            ),
            format!(
                "Nothing else can proceed there until it is resolved: {}.",
                names(&blocked.names)
            ),
            "Open".to_string(),
            None,
            blocked,
        );
    }
    if !unpushed.is_empty() {
        let n = unpushed.len();
        let commits = figures.unpushed_commits
            - seen
                .iter()
                .flat_map(|r| &r.reasons)
                .filter_map(|r| match r {
                    AttentionReason::Diverged { ahead, .. } => Some(*ahead),
                    _ => None,
                })
                .sum::<u32>();
        let age = if unpushed_aging {
            " Some are more than 2 days old."
        } else {
            ""
        };
        add(
            ActionKind::Push,
            if unpushed_aging {
                Severity::Risk
            } else {
                Severity::Warn
            },
            format!(
                "Push {} from {}",
                count(commits as usize, "commit", "commits"),
                count(n, "repository", "repositories")
            ),
            format!(
                "They exist only on this machine: {}.{age}",
                names(&unpushed.names)
            ),
            format!("Push {n}"),
            Some("push"),
            unpushed,
        );
    }
    if !diverged.is_empty() {
        let n = diverged.len();
        add(
            ActionKind::Reconcile,
            Severity::Warn,
            format!(
                "Reconcile {}",
                count(n, "diverged branch", "diverged branches")
            ),
            format!(
                "Local and remote both moved, so neither a push nor a fast-forward pull will work: {}.",
                names(&diverged.names)
            ),
            "Open".to_string(),
            None,
            diverged,
        );
    }
    if !unpublished.is_empty() {
        let n = unpublished.len();
        add(
            ActionKind::Publish,
            Severity::Warn,
            format!(
                "Publish {}",
                count(
                    n,
                    "branch that has no upstream",
                    "branches that have no upstream"
                )
            ),
            format!("Not backed up anywhere: {}.", names(&unpublished.names)),
            format!("Publish {n}"),
            Some("push"),
            unpublished,
        );
    }
    if !worktree_work.is_empty() {
        // One step for all of it, commits and files together: what to do is
        // the same (go and look in the worktree), and one line keeps "Do
        // next" short.
        let mut held = Vec::new();
        if figures.worktree_commits > 0 {
            held.push(count(
                figures.worktree_commits as usize,
                "commit",
                "commits",
            ));
        }
        if figures.worktree_files > 0 {
            held.push(count(
                figures.worktree_files as usize,
                "uncommitted file",
                "uncommitted files",
            ));
        }
        let age = if worktree_aging {
            " Some commits are more than 2 days old."
        } else {
            ""
        };
        add(
            ActionKind::Worktrees,
            if worktree_aging {
                Severity::Risk
            } else {
                Severity::Warn
            },
            format!(
                "Back up {} sitting in {}",
                held.join(" and "),
                count(figures.worktrees_holding_work, "worktree", "worktrees")
            ),
            format!(
                "A worktree is easy to forget, and cleaning it up can delete what was never pushed: {}.{age}",
                names(&worktree_work.names)
            ),
            "Open".to_string(),
            None,
            worktree_work,
        );
    }
    if !stale_stash.is_empty() {
        let n = stale_stash.len();
        add(
            ActionKind::ReviewStash,
            Severity::Warn,
            format!(
                "Review stashes in {}",
                count(n, "repository", "repositories")
            ),
            format!(
                "Parked for more than a week in {}.",
                names(&stale_stash.names)
            ),
            "Review stashes".to_string(),
            None,
            stale_stash,
        );
    }
    if !behind.is_empty() {
        let n = behind.len();
        add(
            ActionKind::Pull,
            Severity::Notice,
            format!(
                "Pull {}",
                count(
                    n,
                    "repository that is behind",
                    "repositories that are behind"
                )
            ),
            format!(
                "A fast-forward pull cannot lose work: {}.",
                names(&behind.names)
            ),
            format!("Pull {n}"),
            Some("pull"),
            behind,
        );
    }
    if !dirty.is_empty() {
        let n = dirty.len();
        let most = largest
            .map(|(name, files)| format!(" Largest: {name} with {files}."))
            .unwrap_or_default();
        add(
            ActionKind::Commit,
            Severity::Notice,
            format!(
                "Commit or discard {}",
                count(
                    figures.uncommitted_files as usize,
                    "uncommitted file",
                    "uncommitted files"
                )
            ),
            format!("In {}.{most}", count(n, "repository", "repositories")),
            "Review changes".to_string(),
            None,
            dirty,
        );
    }
    if !stale_fetch.is_empty() {
        let n = stale_fetch.len();
        add(
            ActionKind::Fetch,
            Severity::Info,
            format!("Fetch {}", count(n, "repository", "repositories")),
            format!(
                "Not fetched in over {} days, so their ahead and behind counts cannot be trusted. Oldest: {} days.",
                STALE_FETCH_SECS / DAY,
                figures.oldest_fetch_days
            ),
            format!("Fetch {n}"),
            Some("fetch"),
            stale_fetch,
        );
    }
    if figures.high_tasks > 0 && figures.doing_tasks == 0 {
        let with_high: Vec<&RepoWork> = work.iter().filter(|w| w.high > 0).collect();
        let where_: Vec<String> = with_high
            .iter()
            .take(3)
            .map(|w| format!("{} {}", w.name, w.high))
            .collect();
        add(
            ActionKind::StartTask,
            Severity::Info,
            "Start a high-priority task".to_string(),
            format!(
                "{} open and none is in progress: {}.",
                count(figures.high_tasks, "is", "are"),
                where_.join(", ")
            ),
            "Open tasks".to_string(),
            None,
            Bucket {
                ids: with_high.iter().map(|w| w.id.clone()).collect(),
                names: Vec::new(),
            },
        );
    }
    actions.sort_by_key(|a| std::cmp::Reverse(a.severity));
    // Commits in worktrees are not backed up either. Added only now, because
    // the "Push N commits" step above counts what a push of the main
    // checkouts would send, which is not these.
    figures.unpushed_commits += figures.worktree_commits;

    // Every repo lands in exactly one of three groups, and the headline names
    // all three, so the numbers add up to the total.
    let look = figures
        .repos
        .saturating_sub(figures.at_risk)
        .saturating_sub(figures.clean);
    let middle = if look > 0 {
        format!(", {} a look", count(look, "needs", "need"))
    } else {
        String::new()
    };
    let headline = match (figures.repos, figures.at_risk) {
        (0, _) => "No repositories found.".to_string(),
        (n, 0) => format!(
            "{}. No work is at risk{middle}, {} clean.",
            count(n, "repository", "repositories"),
            count(figures.clean, "is", "are")
        ),
        (n, risky) => format!(
            "{}. {} work at risk{middle}, {} clean.",
            count(n, "repository", "repositories"),
            count(risky, "has", "have"),
            count(figures.clean, "is", "are")
        ),
    };

    Overview {
        now,
        headline,
        summary: fleet_summary(repos, now),
        figures,
        actions,
        repos: lines,
        work,
        commits,
        activity,
        focus: board.focus.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launchpad::{Contribution, Task};
    use crate::model::{Upstream, WorktreeStatus, sample};

    const NOW: i64 = 1_700_000_000;

    /// A clean repository with a remote and an upstream, committed an hour ago.
    fn clean(id: &str) -> RepoSnapshot {
        let mut r = sample(id, id, false, 0, 0, Some(NOW - 3600));
        r.remote_url = Some(format!("git@github.com:me/{id}.git"));
        r.upstream = Some(Upstream {
            name: "origin/main".to_string(),
            ahead: 0,
            behind: 0,
        });
        r.last_fetch = Some(NOW - 3600);
        r
    }

    fn task(id: &str, status: &str, priority: &str) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            status: Some(status.to_string()),
            priority: Some(priority.to_string()),
            ..Default::default()
        }
    }

    fn entry(id: &str, tasks: Vec<Task>) -> LaunchpadEntry {
        LaunchpadEntry {
            id: id.to_string(),
            name: id.to_string(),
            configured: true,
            contribution: Some(Contribution {
                name: id.to_string(),
                tasks,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn kinds(o: &Overview) -> Vec<ActionKind> {
        o.actions.iter().map(|a| a.kind).collect()
    }

    #[test]
    fn an_empty_fleet_has_nothing_to_do() {
        let o = overview(&[], &[], &Board::default(), NOW);
        assert_eq!(o.headline, "No repositories found.");
        assert!(o.actions.is_empty() && o.repos.is_empty() && o.activity.is_empty());
    }

    #[test]
    fn a_clean_fleet_reports_no_actions() {
        let o = overview(&[clean("a"), clean("b")], &[], &Board::default(), NOW);
        assert!(o.actions.is_empty(), "{:?}", o.actions);
        assert_eq!(o.figures.clean, 2);
        assert_eq!(
            o.headline,
            "2 repositories. No work is at risk, 2 are clean."
        );
        assert_eq!(o.at_risk().count(), 0);
    }

    #[test]
    fn do_next_is_ordered_by_urgency_and_carries_its_targets() {
        // unpublished: has a remote, no upstream.
        let mut unpublished = clean("unpublished");
        unpublished.upstream = None;
        // behind by 4.
        let mut behind = clean("behind");
        behind.upstream.as_mut().unwrap().behind = 4;
        // 3 uncommitted files.
        let mut dirty = clean("dirty");
        dirty.worktree = WorktreeStatus {
            staged: 1,
            modified: 2,
            untracked: 0,
            conflicted: 0,
        };
        // not fetched for 40 days.
        let mut stale = clean("stale");
        stale.last_fetch = Some(NOW - 40 * DAY);
        // 2 unpushed commits, newest 5 days old → aging → risk.
        let mut ahead = clean("ahead");
        ahead.upstream.as_mut().unwrap().ahead = 2;
        ahead.last_commit.as_mut().unwrap().timestamp = NOW - 5 * DAY;

        let fleet = [unpublished, behind, dirty, stale, ahead];
        let o = overview(&fleet, &[], &Board::default(), NOW);

        assert_eq!(
            kinds(&o),
            [
                ActionKind::Push,
                ActionKind::Publish,
                ActionKind::Pull,
                ActionKind::Commit,
                ActionKind::Fetch
            ]
        );
        let push = &o.actions[0];
        assert_eq!(push.title, "Push 2 commits from 1 repository");
        assert_eq!(push.verb.as_deref(), Some("push"));
        assert_eq!(push.repos, ["ahead"]);
        assert_eq!(push.severity, Severity::Risk);
        assert_eq!(o.actions[1].title, "Publish 1 branch that has no upstream");
        assert_eq!(o.actions[1].label, "Publish 1");
        assert_eq!(o.actions[3].title, "Commit or discard 3 uncommitted files");
        assert!(o.actions[3].why.contains("Largest: dirty with 3."));
        assert!(o.actions[4].why.contains("Oldest: 40 days."));

        assert_eq!(o.figures.not_backed_up, 2);
        assert_eq!(o.figures.unpushed_commits, 2);
        assert_eq!(o.figures.behind_commits, 4);
        assert_eq!(o.figures.stale_fetch_repos, 1);
        assert_eq!(
            o.headline,
            "5 repositories. 2 have work at risk, 3 need a look, 0 are clean."
        );
    }

    #[test]
    fn lines_are_most_urgent_first_and_say_why_in_plain_words() {
        let mut ahead = clean("zeta");
        ahead.upstream.as_mut().unwrap().ahead = 1;
        let mut dirty = clean("Alpha");
        dirty.worktree.modified = 1;
        let o = overview(&[clean("beta"), dirty, ahead], &[], &Board::default(), NOW);

        let order: Vec<&str> = o.repos.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(order, ["zeta", "Alpha", "beta"]);
        assert_eq!(o.repos[0].reasons, ["1 commit not pushed"]);
        assert_eq!(
            o.repos[0].action,
            Some(SuggestedAction {
                verb: Some("push".to_string()),
                label: "Push".to_string()
            })
        );
        assert!(o.repos[0].at_risk && !o.repos[1].at_risk);
        assert_eq!(o.repos[1].reasons, ["1 uncommitted file"]);
        assert_eq!(o.repos[1].changed(), 1);
        assert!(o.repos[2].reasons.is_empty() && o.repos[2].action.is_none());
    }

    #[test]
    fn a_repository_stopped_mid_merge_is_listed_once() {
        let mut r = clean("mid");
        r.worktree.conflicted = 2;
        r.operation = Some(crate::model::RepoOperation::Merge);
        let o = overview(&[r], &[], &Board::default(), NOW);
        let resolve: Vec<&NextAction> = o
            .actions
            .iter()
            .filter(|a| a.kind == ActionKind::Resolve)
            .collect();
        assert_eq!(resolve.len(), 1);
        assert_eq!(resolve[0].repos, ["mid"]);
        assert_eq!(
            resolve[0].title,
            "Finish the merge or rebase paused in 1 repository"
        );
    }

    #[test]
    fn tasks_are_counted_per_repository_and_prompt_a_start() {
        let entries = [
            entry(
                "app",
                vec![
                    task("a", "todo", "high"),
                    task("b", "todo", "high"),
                    task("c", "blocked", "normal"),
                    task("d", "done", "high"),
                ],
            ),
            entry("lib", vec![task("e", "todo", "low")]),
            entry("idle", vec![]),
        ];
        let board = Board {
            focus: vec![FocusItem {
                id: "f".to_string(),
                title: "ship".to_string(),
                repo: Some("app".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let fleet = [clean("app"), clean("lib"), clean("idle")];
        let o = overview(&fleet, &entries, &board, NOW);

        assert_eq!(o.figures.open_tasks, 4);
        assert_eq!(o.figures.high_tasks, 2);
        assert_eq!(o.figures.blocked_tasks, 1);
        assert_eq!(o.figures.task_repos, 2);
        assert_eq!(o.figures.configured_repos, 3);
        assert_eq!(o.work.len(), 2);
        assert_eq!((o.work[0].name.as_str(), o.work[0].open), ("app", 3));
        assert_eq!(o.work[0].focus, 1);
        assert_eq!(o.focus.len(), 1);

        let start = o.actions.last().unwrap();
        assert_eq!(start.kind, ActionKind::StartTask);
        assert_eq!(start.why, "2 are open and none is in progress: app 2.");
        assert_eq!(start.repos, ["app"]);

        let app = o.repos.iter().find(|r| r.id == "app").unwrap();
        assert_eq!(
            (app.tasks_open, app.tasks_high, app.configured),
            (3, 2, true)
        );

        // Once something is in progress there is nothing to prompt.
        let busy = [entry("app", vec![task("a", "doing", "high")])];
        let o = overview(&fleet, &busy, &Board::default(), NOW);
        assert!(!kinds(&o).contains(&ActionKind::StartTask));
    }

    #[test]
    fn activity_lines_up_at_the_most_recent_week() {
        let mut long = clean("long");
        long.activity = vec![1, 2, 3, 4];
        let mut short = clean("short");
        short.activity = vec![10, 20];
        let quiet = clean("quiet");
        let fleet = [long, short, quiet];

        assert_eq!(fleet_activity(&fleet), [1, 2, 13, 24]);
        let o = overview(&fleet, &[], &Board::default(), NOW);
        assert_eq!(o.figures.commits_week, 24);
        assert_eq!(o.figures.commits_prev_week, 13);
        assert_eq!(o.figures.commits_total, 40);
        assert_eq!(o.figures.active_repos, 2);
        let ranked: Vec<(&str, u32)> = o
            .commits
            .iter()
            .map(|c| (c.name.as_str(), c.count))
            .collect();
        assert_eq!(ranked, [("short", 20), ("long", 4)]);
    }

    #[test]
    fn names_stay_a_sentence() {
        assert_eq!(names(&[]), "");
        assert_eq!(names(&["a"]), "a");
        assert_eq!(names(&["a", "b"]), "a and b");
        assert_eq!(names(&["a", "b", "c"]), "a, b and c");
        assert_eq!(names(&["a", "b", "c", "d", "e"]), "a, b, c and 2 more");
    }

    #[test]
    fn the_overview_round_trips_through_json() {
        let mut r = clean("a");
        r.worktree.modified = 2;
        let o = overview(&[r], &[], &Board::default(), NOW);
        let json = serde_json::to_string(&o).unwrap();
        let back: Overview = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    /// A linked worktree with `unpushed` commits, `files` untracked files and
    /// a newest commit `commit_age` seconds old.
    fn linked(
        name: &str,
        unpushed: u32,
        files: u32,
        commit_age: i64,
    ) -> crate::model::LinkedWorktree {
        crate::model::LinkedWorktree {
            name: name.to_string(),
            path: format!("/repos/x/.claude/worktrees/{name}").into(),
            branch: crate::model::Branch::Named(format!("worktree-{name}")),
            status: WorktreeStatus {
                untracked: files,
                ..Default::default()
            },
            unpushed,
            unpushed_unique: unpushed,
            last_commit: Some(crate::model::CommitMeta {
                short_id: "abc1234".to_string(),
                author: "agent".to_string(),
                timestamp: NOW - commit_age,
                summary: "wip".to_string(),
            }),
            touched: None,
            locked: false,
            missing: false,
        }
    }

    #[test]
    fn work_in_worktrees_is_one_step_and_counts_as_not_backed_up() {
        let mut a = clean("a");
        a.worktrees = vec![linked("agent-a", 5, 1, 9 * DAY)];
        let mut b = clean("b");
        b.worktrees = vec![linked("fix", 0, 2, 3600), linked("idle", 0, 0, 3600)];
        let o = overview(&[a, b, clean("c")], &[], &Board::default(), NOW);

        // One step for commits and files together, naming the repositories.
        let step = o
            .actions
            .iter()
            .find(|x| x.kind == ActionKind::Worktrees)
            .expect("a worktrees step");
        assert_eq!(
            step.title,
            "Back up 5 commits and 3 uncommitted files sitting in 2 worktrees"
        );
        assert_eq!(step.severity, Severity::Risk, "some commits are 9 days old");
        assert!(step.why.contains("a and b"), "{}", step.why);
        assert!(step.why.contains("more than 2 days old"), "{}", step.why);
        // No verb reaches into a worktree: a person has to look.
        assert_eq!(step.verb, None);
        assert_eq!(step.repos, ["a", "b"]);
        // And no "Push N commits" step: a push of the main checkouts would
        // send none of these.
        assert!(!o.actions.iter().any(|x| x.kind == ActionKind::Push));

        let f = o.figures;
        assert_eq!((f.worktrees, f.worktrees_holding_work), (3, 2));
        assert_eq!((f.worktree_commits, f.worktree_files), (5, 3));
        // `a` has commits on no remote, so it is not backed up; `b` only has
        // files, which is not what "backed up" counts.
        assert_eq!((f.unpushed_repos, f.not_backed_up), (1, 1));
        assert_eq!(f.unpushed_commits, 5);
        // The main checkouts are clean, and stay counted that way.
        assert_eq!((f.dirty_repos, f.uncommitted_files), (0, 0));
        assert_eq!(f.at_risk, 2);
        assert_eq!(
            o.headline,
            "3 repositories. 2 have work at risk, 1 is clean."
        );
    }

    #[test]
    fn a_repo_already_unpushed_is_not_counted_twice_for_its_worktree() {
        let mut a = clean("a");
        a.upstream = Some(Upstream {
            name: "origin/main".to_string(),
            ahead: 2,
            behind: 0,
        });
        a.worktrees = vec![linked("agent-a", 3, 0, 3600)];
        let o = overview(&[a], &[], &Board::default(), NOW);
        assert_eq!(o.figures.unpushed_repos, 1);
        assert_eq!(o.figures.not_backed_up, 1);
        // Both kinds of commit are not backed up; only the main checkout's
        // are what a push sends.
        assert_eq!(o.figures.unpushed_commits, 5);
        let push = o
            .actions
            .iter()
            .find(|x| x.kind == ActionKind::Push)
            .expect("a push step");
        assert_eq!(push.title, "Push 2 commits from 1 repository");
    }
}
