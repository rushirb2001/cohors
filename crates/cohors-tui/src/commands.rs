//! Implementations of the CLI subcommands.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use camino::Utf8PathBuf;
use cohors_core::{AttentionLevel, CiStatus, Selector, SortMode};

use crate::cli::Cli;
use crate::scan::Scanner;

/// `cohors init` — write a starter config and print its path. Auto-detects where
/// the user keeps repos and seeds `roots` with them, so the first `cohors` run
/// shows a populated fleet instead of an empty one.
pub fn init(cli: &Cli, force: bool) -> Result<()> {
    let path: Utf8PathBuf = match &cli.config {
        Some(p) => Utf8PathBuf::from(p),
        None => cohors_config::paths::config_file().context("resolving the config path")?,
    };
    let home = cohors_config::paths::home_dir().ok();
    let roots = crate::detect::detect_roots(home.as_deref());
    cohors_config::write_starter(&path, force, &roots)
        .with_context(|| format!("writing config to {path}"))?;
    println!("Wrote starter config to {path}");
    if roots.is_empty() {
        println!("No repos auto-detected — edit `roots` to point at your code, then run `cohors`.");
    } else {
        println!("Detected roots: {}", roots.join(", "));
        println!("Run `cohors` to see your fleet (edit `roots` to adjust).");
    }
    Ok(())
}

/// `cohors scan` — discover + snapshot all repos and print JSON to stdout.
///
/// With `--select`, only the matching repos are printed, in dirty-first order,
/// using the same [`cohors_core::resolve`] the dashboard and (later) the MCP
/// server use — so `scan --select behind` and `list_repos({behind:true})` agree.
pub fn scan(cli: &Cli, select: Option<&str>) -> Result<()> {
    let scanner = Scanner::from_cli(cli)?;
    let mut snapshots = scanner.scan();

    if let Some(query) = select {
        let selector = parse_selector(query)?;
        let order = cohors_core::resolve(&snapshots, &selector, SortMode::DirtyFirst, now_secs());
        // Reorder/filter the snapshots to the resolved set, preserving order.
        let by_id: HashMap<&str, usize> = snapshots
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id.0.as_str(), i))
            .collect();
        let picked: Vec<usize> = order
            .iter()
            .filter_map(|id| by_id.get(id.0.as_str()).copied())
            .collect();
        snapshots = picked.into_iter().map(|i| snapshots[i].clone()).collect();
    }

    let json = serde_json::to_string_pretty(&snapshots).context("serializing snapshots")?;
    println!("{json}");
    Ok(())
}

/// Current Unix time in seconds (for the clock-dependent `attention` predicate).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a `--select` value into a [`Selector`]. A value starting with `{` is
/// JSON; anything else is shorthand. `~` in path predicates is expanded against
/// `$HOME` here (a CLI concern), keeping `cohors-core` free of environment access.
fn parse_selector(query: &str) -> Result<Selector> {
    let query = query.trim();
    let mut selector = if query.starts_with('{') {
        serde_json::from_str::<Selector>(query).context("parsing --select JSON")?
    } else {
        parse_shorthand(query)?
    };
    expand_tilde(&mut selector, std::env::var("HOME").ok().as_deref());
    Ok(selector)
}

/// Parse comma-separated shorthand tokens into a [`Selector`] (tokens AND
/// together). Bare flags (`dirty`, `behind`, …) set booleans; `key:value`
/// tokens (`name:pay*`, `ci:failing`, `prs:1`, `branch:main`, `root:~/work`)
/// set the scoped predicates; a few named views (`clean`, `attention`) expand
/// to their selectors.
fn parse_shorthand(query: &str) -> Result<Selector> {
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
                "ci" => sel.ci = Some(parse_ci(&value)?),
                "prs" | "min-prs" => {
                    sel.min_prs = Some(value.parse().context("--select prs expects a number")?)
                }
                "attention" => sel.attention = Some(parse_attention(&value)?),
                other => bail!("unknown selector key `{other}:`"),
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
            other => bail!(
                "unknown selector `{other}` (try: dirty, behind, ahead, attention, clean, name:<glob>)"
            ),
        }
    }
    Ok(sel)
}

fn parse_ci(value: &str) -> Result<CiStatus> {
    Ok(match value {
        "passing" => CiStatus::Passing,
        "failing" => CiStatus::Failing,
        "pending" => CiStatus::Pending,
        other => bail!("unknown ci status `{other}` (passing | failing | pending)"),
    })
}

fn parse_attention(value: &str) -> Result<AttentionLevel> {
    Ok(match value {
        "any" => AttentionLevel::Any,
        "notice" => AttentionLevel::Notice,
        "warn" => AttentionLevel::Warn,
        "risk" => AttentionLevel::Risk,
        other => bail!("unknown attention level `{other}` (any | notice | warn | risk)"),
    })
}

/// Expand a leading `~` in path predicates against `home`, recursing into
/// combinators. `home` is passed in (read from `$HOME` by the caller) so this
/// stays a pure, directly-testable function.
fn expand_tilde(sel: &mut Selector, home: Option<&str>) {
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

/// Bare `cohors` — launch the interactive dashboard.
pub fn run_tui(cli: &Cli) -> Result<()> {
    let scanner = Arc::new(Scanner::from_cli(cli)?);
    crate::tui::run(scanner, !cli.no_cache, cli.watch)
}

/// `cohors demo` — launch the dashboard on a built-in sample fleet. No config,
/// no scanning, no disk access; a zero-setup way to try the tool.
pub fn run_demo() -> Result<()> {
    crate::tui::run_demo()
}

/// `cohors mcp` — speak the Model Context Protocol over stdio. Read-only unless
/// the matching `--allow-*` flags are passed (none have an effect yet — only the
/// read tools are implemented).
pub fn run_mcp(cli: &Cli, allow_writes: bool, allow_run: bool, allow_open: bool) -> Result<()> {
    let scanner = Scanner::from_cli(cli)?;
    let caps = crate::mcp::Caps {
        allow_writes,
        allow_run,
        allow_open,
    };
    let roots = scanner.roots();
    let config_path = scanner.config_path();
    let token = scanner.github_token();
    let mcp_config = scanner.mcp_config();
    let scan = || scanner.scan();
    crate::mcp::run(
        &scan,
        token.as_deref(),
        &roots,
        &config_path,
        caps,
        &mcp_config.run_allowlist,
        mcp_config.max_action_targets,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorthand_flags_and_combine() {
        let sel = parse_selector("dirty,behind").unwrap();
        assert!(sel.dirty && sel.behind);
        assert!(!sel.ahead);
    }

    #[test]
    fn shorthand_key_values() {
        let sel = parse_selector("name:pay*,ci:failing,prs:2,branch:main").unwrap();
        assert_eq!(sel.name.as_deref(), Some("pay*"));
        assert_eq!(sel.ci, Some(CiStatus::Failing));
        assert_eq!(sel.min_prs, Some(2));
        assert_eq!(sel.branch.as_deref(), Some("main"));
    }

    #[test]
    fn unpushed_aliases_ahead() {
        assert!(parse_selector("unpushed").unwrap().ahead);
    }

    #[test]
    fn clean_is_negation_of_attention_states() {
        let sel = parse_selector("clean").unwrap();
        let not = sel.not.expect("clean sets `not`");
        assert_eq!(not.any_of.len(), 5);
    }

    #[test]
    fn json_passthrough() {
        let sel = parse_selector(r#"{"behind": true, "name": "api"}"#).unwrap();
        assert!(sel.behind);
        assert_eq!(sel.name.as_deref(), Some("api"));
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
        assert!(parse_selector("frobnicate").is_err());
        assert!(parse_selector("ci:sideways").is_err());
    }
}
