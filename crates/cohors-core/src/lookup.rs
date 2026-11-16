//! Finding the one repository someone means from what they typed: an exact
//! name, alias, id or path; a unique prefix; a glob. When nothing matches,
//! [`close_names`] offers the names that come close ("did you mean").
//!
//! The CLI (`cohors path`, `cohors show`) and the MCP server (`get_repo`,
//! `repo_path`, …) both resolve names here, so a person and an agent who type
//! the same thing get the same repository.
//!
//! Pure: resolving a path on disk (`.`, a relative path, a symlink) is the
//! caller's job; it passes the result in as `canonical`.

use crate::model::RepoSnapshot;
use crate::select::glob_name;

/// What a typed repo name turned out to be.
#[derive(Debug)]
pub enum Resolved<'a> {
    /// Exactly one repository.
    One(&'a RepoSnapshot),
    /// Several, equally good: the name is ambiguous.
    Many(Vec<&'a RepoSnapshot>),
    /// None at all.
    Missing,
}

/// The repo's directory name: what it is called on disk, whatever alias the
/// config gives it.
fn dir_name(snap: &RepoSnapshot) -> Option<&str> {
    snap.path.as_ref().and_then(|p| p.file_name())
}

/// The names a repo answers to: the name it is shown under (its alias, when
/// one is set) and its directory name.
fn names(snap: &RepoSnapshot) -> impl Iterator<Item = &str> {
    std::iter::once(snap.name.as_str()).chain(dir_name(snap))
}

/// One repo becomes [`Resolved::One`], several [`Resolved::Many`], none `None`
/// (so the caller tries its next rule).
fn pick(found: Vec<&RepoSnapshot>) -> Option<Resolved<'_>> {
    match found.len() {
        0 => None,
        1 => Some(Resolved::One(found[0])),
        _ => Some(Resolved::Many(found)),
    }
}

/// Only the exact rule of [`resolve_repo`]: an id, a name or alias, a
/// directory name, or a path (`canonical` is `key` resolved on disk). For a
/// caller that must not guess, such as a tool that replaces a file.
pub fn exact<'a>(snaps: &'a [RepoSnapshot], key: &str, canonical: Option<&str>) -> Resolved<'a> {
    let key = key.trim();
    if key.is_empty() {
        return Resolved::Missing;
    }
    let found: Vec<&RepoSnapshot> = snaps
        .iter()
        .filter(|s| {
            s.id.0 == key
                || names(s).any(|n| n == key)
                || s.path
                    .as_ref()
                    .is_some_and(|p| p.as_str() == key || Some(p.as_str()) == canonical)
        })
        .collect();
    pick(found).unwrap_or(Resolved::Missing)
}

/// Resolve `key` to repos, in steps, stopping at the first that finds
/// anything:
///
/// 1. exactly what was typed (see [`exact`]); an exact match always wins;
/// 2. a folder inside a repo names that repo (the innermost one, for a repo
///    nested in another), when `canonical` is such a folder;
/// 3. a prefix of a name, in any case (`Pay` finds `payments`); when several
///    share the prefix, one whose whole name matches in another case wins;
/// 4. a glob over the names (`'pay*'`).
pub fn resolve_repo<'a>(
    snaps: &'a [RepoSnapshot],
    key: &str,
    canonical: Option<&str>,
) -> Resolved<'a> {
    let key = key.trim();
    if key.is_empty() {
        return Resolved::Missing;
    }
    if let found @ (Resolved::One(_) | Resolved::Many(_)) = exact(snaps, key, canonical) {
        return found;
    }

    if let Some(canonical) = canonical {
        let within = snaps
            .iter()
            .filter_map(|s| {
                let path = s.path.as_ref()?.as_str();
                let rest = canonical.strip_prefix(path)?;
                rest.starts_with('/').then_some((path.len(), s))
            })
            .max_by_key(|(len, _)| *len);
        if let Some((_, snap)) = within {
            return Resolved::One(snap);
        }
    }

    let lower = key.to_lowercase();
    let prefixed: Vec<&RepoSnapshot> = snaps
        .iter()
        .filter(|s| names(s).any(|n| n.to_lowercase().starts_with(&lower)))
        .collect();
    if prefixed.len() > 1 {
        let whole: Vec<&RepoSnapshot> = prefixed
            .iter()
            .copied()
            .filter(|s| names(s).any(|n| n.to_lowercase() == lower))
            .collect();
        if whole.len() == 1 {
            return Resolved::One(whole[0]);
        }
    }
    if let Some(found) = pick(prefixed) {
        return found;
    }

    if key.contains(['*', '?']) {
        let globbed: Vec<&RepoSnapshot> = snaps
            .iter()
            .filter(|s| names(s).any(|n| glob_name(key, n)))
            .collect();
        if let Some(found) = pick(globbed) {
            return found;
        }
    }
    Resolved::Missing
}

/// Names close to `key`, for a "did you mean": names that contain it (or that
/// it contains), then names a few typos away from the whole name or from one
/// of its words (`servce` is close to `auth-service`), nearest first. At most
/// five, as the repos are named (a caller showing them on a terminal filters
/// control characters first).
pub fn close_names(snaps: &[RepoSnapshot], key: &str) -> Vec<String> {
    let key = key.trim().to_lowercase();
    if key.is_empty() {
        return Vec::new();
    }
    // A third of the word may be wrong, but always allow a couple of slips.
    let budget = (key.chars().count() / 3).max(2);
    let mut scored: Vec<(usize, &str)> = snaps
        .iter()
        .filter_map(|s| {
            let distance = names(s)
                .map(|n| {
                    let n = n.to_lowercase();
                    if n.contains(&key) || (n.chars().count() >= 3 && key.contains(&n)) {
                        return 0;
                    }
                    // The whole name, and each word of three letters or more.
                    std::iter::once(n.as_str())
                        .chain(
                            n.split(['-', '_', '.'])
                                .filter(|word| word.chars().count() >= 3),
                        )
                        .map(|candidate| edit_distance(&key, candidate))
                        .min()
                        .unwrap_or(usize::MAX)
                })
                .min()?;
            (distance <= budget).then_some((distance, s.name.as_str()))
        })
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.1 == b.1);
    scored
        .into_iter()
        .take(5)
        .map(|(_, n)| n.to_string())
        .collect()
}

/// How many single-character edits turn `a` into `b` (Levenshtein), counted
/// in characters so a non-ASCII name can never split a byte sequence.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    // Only two rows of the table are kept: the previous one and this one.
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let swap = prev[j] + usize::from(ca != *cb);
            cur[j + 1] = swap.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Branch, RepoId};

    /// A readable repo on `main` at `path`, named `name`.
    fn snap(name: &str, path: &str) -> RepoSnapshot {
        let mut s = RepoSnapshot::errored(RepoId(path.into()), name, Some(path.into()), "");
        s.error = None;
        s.branch = Branch::Named("main".into());
        s
    }

    fn fleet() -> Vec<RepoSnapshot> {
        vec![
            snap("payments", "/code/payments"),
            snap("payroll", "/code/payroll"),
            snap("web", "/code/web"),
            // Shown under an alias; its directory is `legacy-api`.
            snap("api", "/code/legacy-api"),
        ]
    }

    fn name_of(r: Resolved<'_>) -> String {
        match r {
            Resolved::One(s) => s.name.clone(),
            Resolved::Many(v) => format!("many:{}", v.len()),
            Resolved::Missing => "missing".to_string(),
        }
    }

    #[test]
    fn a_repo_resolves_by_exact_name_alias_directory_id_and_path() {
        let repos = fleet();
        let r = |key: &str| name_of(resolve_repo(&repos, key, None));
        assert_eq!(r("payments"), "payments");
        assert_eq!(r("api"), "api", "the alias");
        assert_eq!(r("legacy-api"), "api", "the directory behind the alias");
        assert_eq!(r("/code/web"), "web", "the id or path");
        // `.` (or a folder inside a repo), once the caller resolved it on disk.
        assert_eq!(
            name_of(resolve_repo(&repos, ".", Some("/code/payroll"))),
            "payroll"
        );
        assert_eq!(
            name_of(resolve_repo(&repos, "src", Some("/code/payroll/src/lib"))),
            "payroll"
        );
    }

    #[test]
    fn a_unique_prefix_in_any_case_resolves_and_a_shared_one_does_not() {
        let repos = fleet();
        let r = |key: &str| name_of(resolve_repo(&repos, key, None));
        assert_eq!(r("PAYM"), "payments");
        assert_eq!(r("we"), "web");
        assert_eq!(r("pay"), "many:2", "payments and payroll share it");
        // A whole name in another case settles a shared prefix.
        let mut repos = fleet();
        repos.push(snap("pay", "/code/pay"));
        assert_eq!(name_of(resolve_repo(&repos, "PAY", None)), "pay");
    }

    #[test]
    fn an_exact_match_beats_a_longer_name_it_prefixes() {
        let mut repos = fleet();
        repos.push(snap("web-admin", "/code/web-admin"));
        assert_eq!(name_of(resolve_repo(&repos, "web", None)), "web");
        // `exact` alone never guesses from a prefix.
        assert_eq!(name_of(exact(&repos, "web-adm", None)), "missing");
        assert_eq!(name_of(resolve_repo(&repos, "web-adm", None)), "web-admin");
    }

    #[test]
    fn a_glob_resolves_when_nothing_else_does() {
        let repos = fleet();
        let r = |key: &str| name_of(resolve_repo(&repos, key, None));
        assert_eq!(r("*eb"), "web");
        assert_eq!(r("pay*"), "many:2", "a prefix glob shared by two");
        assert_eq!(r("*roll"), "payroll");
        assert_eq!(r("nothing"), "missing");
        assert_eq!(r("  "), "missing");
    }

    #[test]
    fn duplicate_names_are_ambiguous() {
        let mut repos = fleet();
        repos.push(snap("web", "/other/web"));
        assert_eq!(name_of(resolve_repo(&repos, "web", None)), "many:2");
        assert_eq!(name_of(exact(&repos, "web", None)), "many:2");
        // Its path still names exactly one.
        assert_eq!(name_of(resolve_repo(&repos, "/other/web", None)), "web");
    }

    #[test]
    fn a_miss_suggests_close_names_and_nothing_unrelated() {
        let repos = fleet();
        assert_eq!(close_names(&repos, "paymnets"), ["payments"]);
        assert_eq!(
            close_names(&repos, "ments"),
            ["payments"],
            "a part of a name"
        );
        assert_eq!(
            close_names(&repos, "legacy-apu"),
            ["api"],
            "the directory name"
        );
        assert_eq!(close_names(&repos, "legcy"), ["api"], "one word of a name");
        assert!(close_names(&repos, "zzzzzzzz").is_empty());
    }

    #[test]
    fn edit_distance_counts_characters() {
        assert_eq!(edit_distance("", ""), 0);
        assert_eq!(edit_distance("web", "web"), 0);
        assert_eq!(edit_distance("wbe", "web"), 2);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("café", "cafe"), 1, "never splits a char");
    }
}
