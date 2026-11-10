//! The overlays: help, the standup, the full repository drill-in, the command
//! run results, and the small pickers and confirmations.

use super::cards::severity_style;
use super::*;

/// Truncate `s` to at most `max` characters, adding an ellipsis when cut.
pub(crate) fn ellipsize(s: &str, max: usize) -> String {
    text::clip(s, max, '…')
}

/// The help overlay: what the marks mean, then every key, grouped by the card
/// it belongs to. This is the one place the whole keymap is written down, so
/// it must be changed together with the key handling in `app.rs`.
pub(crate) fn render_help(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let g = &theme.glyphs;
    // The legend shows the severity blocks exactly as the rows draw them.
    let mark = |sev: Severity| super::cards::severity_block(sev, theme);

    // One shared two-column grid for the whole overlay: a 2-space-indented left
    // cell (a colored glyph example, or a key) is padded so *every* description
    // starts in the same column — so the legend and the keymap read as one
    // aligned table instead of a wrapping paragraph.
    let desc_col = 19usize;
    let row = |left: Vec<Span<'static>>, desc: &str| -> Line<'static> {
        let used: usize = left.iter().map(|sp| sp.content.chars().count()).sum();
        let pad = desc_col.saturating_sub(used).max(2);
        let mut spans = vec![Span::raw("  ")];
        spans.extend(left);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(desc.to_string(), theme.dim()));
        Line::from(spans)
    };
    let head = |t: &str| Line::from(Span::styled(t.to_string(), bold));
    let key = |t: &str| vec![Span::styled(t.to_string(), bold)];

    let enter = g.key_enter();
    let lines = vec![
        // The legend first: it answers "what am I looking at?", which is the
        // question a new user has before they care about keybindings.
        head("Marks"),
        row(
            vec![mark(Severity::Risk), Span::raw(" "), mark(Severity::Warn)],
            "work at risk: it exists only on this machine",
        ),
        row(
            vec![mark(Severity::Notice), Span::raw(" "), mark(Severity::Info)],
            "worth a look; a row with no block is clean",
        ),
        row(
            vec![Span::styled(g.marked(), theme.ahead())],
            "marked for a bulk action",
        ),
        Line::from(""),
        head("Everywhere"),
        row(key("1-5"), "focus a card"),
        row(key("tab / shift-tab"), "next / previous card"),
        row(
            key(&format!("{} j k", g.key_updown())),
            "move within the focused card",
        ),
        row(key(enter), "open the highlighted row"),
        row(
            key(":"),
            "command line (:fetch :pull :push :commit :sort …)",
        ),
        row(key("!"), "run a shell command in the marked repositories"),
        row(key("/"), "filter the repository table by name"),
        row(key("s  t"), "the standup / the launchpad (tasks and flows)"),
        row(
            key("r  ?  q"),
            "rescan / this help / quit (Ctrl-C also quits)",
        ),
        Line::from(""),
        head("1 Do next and 2 Work at risk"),
        row(
            key("a step's key"),
            "do that step for the repositories it lists",
        ),
        row(key(enter), "do the highlighted step / open the repository"),
        Line::from(""),
        head("3 Open tasks and 4 Commits"),
        row(
            key(enter),
            "that repository's tasks / its commits in the standup",
        ),
        row(key("y"), "on 4 Commits: copy the standup as markdown"),
        Line::from(""),
        head("5 Repositories, and the detail view"),
        row(
            key(&format!("{}  *", g.key_space())),
            "mark it / mark all listed (again to clear)",
        ),
        row(
            key("f  F  p  P"),
            "fetch / fetch all / pull (ff only) / push",
        ),
        row(
            key("c  S"),
            "commit / stash; both ask before changing anything",
        ),
        row(key("o  L  y"), "open with… / lazygit / copy the path"),
        row(
            key("a"),
            "ask your coding agent to set the repo up (asks first)",
        ),
        row(key(">"), "change the sort order"),
        row(key(enter), "the detail view; there, the full drill-in"),
        row(
            key("esc"),
            "clear marks, a narrowing, the filter; or go back",
        ),
        Line::from(""),
        Line::from(Span::styled(
            format!("cohors v{}   config: {}", version_str(), app.config_path),
            theme.dim(),
        )),
    ];

    // Collapse the box to its content (capped to the screen) so the help doesn't
    // sit in a tall, mostly-empty modal. + padding (2) + border (2) + the in-box
    // header (2) + the in-box hint footer (2).
    let want_h = lines.len() as u16 + 8;
    let h = want_h.min(full.height.saturating_sub(2).max(10));
    // Wide enough that no description wraps, when the screen allows it.
    let w = (full.width * 70 / 100)
        .max(88)
        .min(full.width.saturating_sub(4));
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let block = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_header(frame, area, inner, theme, Line::from("Help").bold());
    let inner = render_modal_hints(frame, area, inner, app, theme);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

/// Count commits by their conventional-commit type (the word before the first
/// `:`, scope stripped: `feat(ui): …` → `feat`), most-common first. Powers the
/// standup's at-a-glance "what you did" description.
pub(crate) fn commit_type_counts(commits: &[StandupCommit]) -> Vec<(String, usize)> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for c in commits {
        if let Some(kind) = commit_kind(&c.summary) {
            *counts.entry(kind).or_default() += 1;
        }
    }
    let mut v: Vec<(String, usize)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v
}

/// A stable colour per commit type, so the standup reads at a glance. Unknown
/// types fall back to dim. Respects `NO_COLOR` via `Theme::fg`.
pub(crate) fn commit_type_style(kind: &str, theme: &Theme) -> Style {
    let color = match kind {
        "feat" => Color::Green,
        "fix" => Color::Red,
        "design" | "style" => Color::Magenta,
        "chore" | "build" | "ci" | "deps" => Color::Blue,
        "docs" | "content" => Color::Cyan,
        "refactor" | "perf" => Color::Yellow,
        "test" => Color::LightGreen,
        _ => return theme.dim(),
    };
    theme.fg(color)
}

/// The commit type (`feat`, `fix`, scope stripped) at the start of a summary, if
/// it reads like a conventional-commit prefix.
pub(crate) fn commit_kind(summary: &str) -> Option<String> {
    let (prefix, _) = summary.split_once(':')?;
    let kind = prefix.split('(').next().unwrap_or(prefix).trim();
    (!kind.is_empty() && kind.len() <= 12 && !kind.contains(' ')).then(|| kind.to_lowercase())
}

/// A natural-language clause for a commit type, as `(lead, noun)` — e.g.
/// `("shipping 68 ", "features")` — so the standup can read like a sentence
/// ("…, shipping 68 features, fixing 3 bugs, …"). The noun is coloured by kind
/// by the caller; the lead stays plain. Unknown kinds get a generic phrasing.
pub(crate) fn commit_phrase(kind: &str, n: usize) -> (String, String) {
    let known = match kind {
        "feat" => Some(("shipping", "feature", "features")),
        "fix" => Some(("fixing", "bug", "bugs")),
        "design" => Some(("polishing", "design change", "design changes")),
        "style" => Some(("tidying", "style change", "style changes")),
        "chore" => Some(("clearing", "chore", "chores")),
        "docs" => Some(("writing", "doc update", "doc updates")),
        "content" => Some(("writing", "content update", "content updates")),
        "refactor" => Some(("refactoring in", "commit", "commits")),
        "perf" => Some(("tuning", "perf change", "perf changes")),
        "test" => Some(("adding", "test", "tests")),
        "build" => Some(("updating", "build change", "build changes")),
        "ci" => Some(("tweaking", "CI change", "CI changes")),
        _ => None,
    };
    match known {
        Some((verb, singular, plural)) => {
            let noun = if n == 1 { singular } else { plural };
            (format!("{verb} {n} "), noun.to_string())
        }
        // Unknown type → keep the literal kind: "3 wip commits".
        None => (
            format!("{n} "),
            format!("{kind} commit{}", if n == 1 { "" } else { "s" }),
        ),
    }
}

/// A commit summary as styled spans: the type prefix coloured by kind, the rest
/// plain.
pub(crate) fn commit_summary_spans(summary: &str, theme: &Theme) -> Vec<Span<'static>> {
    if let Some(kind) = commit_kind(summary)
        && let Some((prefix, rest)) = summary.split_once(':')
    {
        return vec![
            Span::styled(format!("{prefix}:"), commit_type_style(&kind, theme)),
            Span::raw(rest.to_string()),
        ];
    }
    vec![Span::raw(summary.to_string())]
}

pub(crate) fn render_standup(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    // Size the modal to its content (capped) so it doesn't dominate the screen:
    // the panes are as tall as the busiest repo's commit list, up to a cap, and
    // anything past that scrolls.
    let body = app
        .standup
        .as_ref()
        .map(|v| {
            let max_c = v.groups.iter().map(|(_, c)| c.len()).max().unwrap_or(1);
            v.groups.len().max(max_c)
        })
        .unwrap_or(3)
        .clamp(6, 16) as u16;
    // body + title(1) + header rule(1) + description(3) + mid rule(1) +
    // column header(1) + hints rule(1) + hints(1) + outer border(2).
    let h = (body + 11).min(full.height.saturating_sub(2)).max(12);
    let w = (full.width as u32 * 84 / 100) as u16;
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let window = app.standup_window.label();
    // The outer frame. Title + hints ride inside the box (on their own rules), and
    // the title sits directly under the top border — no blank pad line above it.
    let outer = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::new(1, 1, 0, 0));

    // Still walking the commits: a placeholder.
    let Some(view) = &app.standup else {
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let inner = render_modal_header(
            frame,
            area,
            inner,
            theme,
            Line::from(format!("Standup · {window}")).bold(),
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("Collecting your commits for {window}…"),
                theme.dim(),
            )),
            inner,
        );
        return;
    };

    let total = view.commits.len();
    let repos = view.groups.len();
    // The header carries the title and the count, inside the box.
    let header = Line::from(vec![
        Span::styled(format!("Standup · {window}"), Style::new().bold()),
        Span::styled(
            format!(
                "   ·   {total} commit{} · {repos} repo{}",
                if total == 1 { "" } else { "s" },
                if repos == 1 { "" } else { "s" },
            ),
            theme.dim(),
        ),
    ]);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let inner = render_modal_header(frame, area, inner, theme, header);
    let inner = render_modal_hints(frame, area, inner, app, theme);

    if view.groups.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(format!("No commits {window}."), theme.dim())),
            inner,
        );
        return;
    }

    // A dynamic, one-line description above the panes so the view explains
    // itself: which window, and what each column is.
    let repo_name = view
        .groups
        .get(view.focus)
        .map(|(r, _)| r.as_str())
        .unwrap_or("");
    // The description (up to 3 wrapped lines), a solid rule, then the two columns.
    let [desc_area, mid_rule_area, panes_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    // A glance of *what you did*, as one flowing, wrapping sentence:
    // "You authored 130 commits this week across 5 repos, shipping 68 features,
    //  fixing 14 bugs, polishing 16 design changes, and writing 8 doc updates."
    // (the type nouns are coloured by kind; the rest is prose.)
    let mut spans = vec![
        Span::styled("You authored ", theme.dim()),
        Span::styled(
            total.to_string(),
            theme.ahead().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " commit{} {window} across {repos} repo{}",
                if total == 1 { "" } else { "s" },
                if repos == 1 { "" } else { "s" }
            ),
            theme.dim(),
        ),
    ];
    // Each commit type becomes a clause ("shipping 68 features"); join them with
    // commas and a final "and".
    let counts = commit_type_counts(&view.commits);
    let mut clauses: Vec<Vec<Span>> = counts
        .iter()
        .take(5)
        .map(|(kind, n)| {
            let (lead, noun) = commit_phrase(kind, *n);
            vec![
                Span::styled(lead, theme.dim()),
                Span::styled(noun, commit_type_style(kind, theme)),
            ]
        })
        .collect();
    if counts.len() > 5 {
        let rest: usize = counts.iter().skip(5).map(|(_, n)| *n).sum();
        clauses.push(vec![Span::styled(
            format!("and {rest} more across other kinds"),
            theme.dim(),
        )]);
    }
    if !clauses.is_empty() {
        spans.push(Span::styled(", ", theme.dim()));
        let last = clauses.len() - 1;
        let has_more = counts.len() > 5; // the final clause already starts with "and"
        for (i, clause) in clauses.into_iter().enumerate() {
            if i > 0 {
                let sep = if i == last && !has_more {
                    if last > 1 { ", and " } else { " and " }
                } else {
                    ", "
                };
                spans.push(Span::styled(sep, theme.dim()));
            }
            spans.extend(clause);
        }
    }
    spans.push(Span::styled(".", theme.dim()));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true }),
        desc_area,
    );

    // Two columns split by a vertical divider (no inner boxes): the repos on the
    // left, the focused repo's commits on the right. The active column is marked by
    // a bold header; the other is dim.
    let active = |on: bool| {
        if on {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            theme.dim()
        }
    };

    // repos column | 1-col divider | commits column, with a space either side of
    // the divider (`.spacing(1)`).
    let [repos_col, divider_col, commits_col] = Layout::horizontal([
        Constraint::Length(22),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .spacing(1)
    .areas(panes_area);

    // A solid rule between the description and the columns, plus a solid vertical
    // divider that ties into it: the two read as one framed table. The vertical line
    // joins the rule above with `┬` and the footer rule below with `┴`.
    modal_rule(frame, area, mid_rule_area.y, theme);
    let cell = |ch: &'static str, x: u16, y: u16| {
        (
            Paragraph::new(Span::raw(ch)),
            Rect {
                x,
                y,
                width: 1,
                height: 1,
            },
        )
    };
    for y in divider_col.y..divider_col.y + divider_col.height {
        let (p, r) = cell("│", divider_col.x, y);
        frame.render_widget(p, r);
    }
    let (p, r) = cell("┬", divider_col.x, mid_rule_area.y);
    frame.render_widget(p, r);
    let (p, r) = cell("┴", divider_col.x, panes_area.y + panes_area.height);
    frame.render_widget(p, r);

    // Each column is a header row, then its list below.
    let [repos_hdr, repos_list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(repos_col);
    let [commits_hdr, commits_list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(commits_col);

    // Left column: a "Repos" header, then each repo with its commit count right-
    // aligned so the numbers line up. The focused repo gets the soft selection wash
    // (never a hard reverse, which buried the dim count and made the row unreadable).
    frame.render_widget(
        Paragraph::new(Span::styled("Repos", active(!view.commits_focused))),
        repos_hdr,
    );
    let col_w = repos_list_area.width as usize;
    let items: Vec<ListItem> = view
        .groups
        .iter()
        .map(|(repo, commits)| {
            let count = commits.len().to_string();
            let name = ellipsize(repo, col_w.saturating_sub(count.len() + 1));
            let pad = col_w.saturating_sub(name.chars().count() + count.len());
            ListItem::new(Line::from(vec![
                Span::raw(name),
                Span::styled(format!("{}{count}", " ".repeat(pad)), theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection());
    let mut list_state = ListState::default();
    list_state.select(Some(view.focus));
    frame.render_stateful_widget(list, repos_list_area, &mut list_state);

    // Right column: a "{repo} · N commits" header, then the commit list with a
    // scrollbar. The cursor shows only when this column has focus.
    let n = view.focused_len();
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(
                "{repo_name}  ·  {n} commit{}",
                if n == 1 { "" } else { "s" }
            ),
            active(view.commits_focused),
        )),
        commits_hdr,
    );

    let items: Vec<ListItem> = view
        .groups
        .get(view.focus)
        .map(|(_, commits)| {
            commits
                .iter()
                .map(|c| {
                    let mut spans = vec![
                        Span::styled(c.short_id.clone(), theme.ahead()),
                        Span::raw("  "),
                    ];
                    spans.extend(commit_summary_spans(&c.summary, theme));
                    ListItem::new(Line::from(spans))
                })
                .collect()
        })
        .unwrap_or_default();

    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(commits_list_area);

    // Keep the highlighted commit visible: nudge the scroll offset only as far as
    // needed, then clamp. This drives both the list and the scrollbar.
    let total = items.len() as u16;
    let viewport = text_area.height;
    let cursor = view.commit_cursor as u16;
    let mut offset = view.offset();
    if cursor < offset {
        offset = cursor;
    } else if viewport > 0 && cursor >= offset + viewport {
        offset = cursor + 1 - viewport;
    }
    offset = offset.min(total.saturating_sub(viewport));
    view.set_offset(offset);

    let list = List::new(items).highlight_style(theme.selection());
    let mut list_state = ListState::default();
    // Only show the cursor when the commits pane has focus.
    if view.commits_focused {
        list_state.select(Some(view.commit_cursor));
    }
    *list_state.offset_mut() = offset as usize;
    frame.render_stateful_widget(list, text_area, &mut list_state);

    if total > viewport {
        let mut sb = ScrollbarState::new(total as usize)
            .position(offset as usize)
            .viewport_content_length(viewport as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            bar_area,
            &mut sb,
        );
    }
}

/// A section header line inside the detail pane (blank spacer + bold title).
pub(crate) fn detail_section(lines: &mut Vec<Line<'static>>, title: String) {
    if !lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(
        title,
        Style::new().add_modifier(Modifier::BOLD),
    )));
}

/// Colour for a changed file: conflicts red, untracked dim, staged green,
/// unstaged yellow. Uses the typed porcelain interpretation from `cohors-core`
/// rather than matching status characters here.
pub(crate) fn changed_file_style(f: &cohors_core::ChangedFile, theme: &Theme) -> Style {
    if f.is_conflicted() {
        theme.risk()
    } else if f.is_untracked() {
        theme.dim()
    } else if f.is_staged() {
        theme.staged()
    } else {
        theme.modified()
    }
}

/// The per-repo drill-in detail pane: recent commits, working-tree changes,
/// branches, and stashes for the current repo, scrollable. Shows a "loading"
/// placeholder until the background read finishes.
pub(crate) fn render_detail(frame: &mut Frame, full: Rect, app: &App, now: i64, theme: &Theme) {
    let Some(dv) = &app.detail else {
        return;
    };
    let branch = dv.detail.as_ref().and_then(|d| d.current_branch.clone());
    let title = match &branch {
        Some(b) => format!(" {}  ·  {b} ", dv.repo_name),
        None => format!(" {} ", dv.repo_name),
    };

    // Still loading: a small centred box, not a full-size empty one.
    let Some(d) = &dv.detail else {
        let area = detail_rect(full, 5); // 1 content row + 2 header + 2 hint footer
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_type(FRAME)
            .padding(Padding::new(1, 0, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let inner =
            render_modal_header(frame, area, inner, theme, Line::from(title.clone()).bold());
        let inner = render_modal_hints(frame, area, inner, app, theme);
        frame.render_widget(
            Paragraph::new(Span::styled("Reading repo…", theme.dim())),
            inner,
        );
        return;
    };

    // Build both panes' content up front so the modal can size to it (and so the
    // panes can scroll). Right pane: recent commits.
    let mut clines: Vec<Line> = Vec::new();
    for c in &d.recent_commits {
        let mut spans = vec![
            Span::styled(format!("{}  ", c.short_id), theme.ahead()),
            Span::styled(
                format!("{:>4}  ", time::relative(c.timestamp, now)),
                theme.dim(),
            ),
        ];
        spans.extend(commit_summary_spans(&c.summary, theme));
        clines.push(Line::from(spans));
    }

    // Left pane: attention · changes · branches · stashes · PRs · contributors.
    let mut lines: Vec<Line> = Vec::new();

    // Why this repo needs you, most urgent first — the same reasons the row's
    // Status column abbreviates, spelled out in one place.
    if let Some(snap) = app.repos.iter().find(|r| r.id == dv.repo_id) {
        let assessment = assess(snap, now);
        if !assessment.reasons.is_empty() {
            detail_section(&mut lines, "Attention".to_string());
            for reason in &assessment.reasons {
                lines.push(Line::from(Span::styled(
                    reason.label(),
                    severity_style(reason.severity(), theme),
                )));
            }
        }
        // CI and open pull requests, when the repository's host reported them.
        // The table has no columns for these, so this is where they are read.
        if let Some(remote) = &snap.remote {
            detail_section(&mut lines, "Checks".to_string());
            lines.push(checks_line(remote, theme));
        }
    }

    detail_section(&mut lines, format!("Changes ({})", d.changed_files.len()));
    if d.changed_files.is_empty() {
        lines.push(Line::from(Span::styled("clean", theme.dim())));
    }
    for f in &d.changed_files {
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", f.status), changed_file_style(f, theme)),
            Span::raw(f.path.clone()),
        ]));
    }
    detail_section(&mut lines, format!("Branches ({})", d.branches.len()));
    for b in &d.branches {
        let style = if b.is_current {
            theme.ahead().add_modifier(Modifier::BOLD)
        } else {
            Style::new()
        };
        // Per-branch sync against its own upstream: "↑2 ↓1", or "no upstream"
        // for a local-only branch (in-sync tracked branches show nothing).
        let sync = if !b.has_upstream {
            "  · no upstream".to_string()
        } else if b.ahead > 0 || b.behind > 0 {
            let mut s = String::from("  ");
            if b.ahead > 0 {
                s.push_str(&format!("↑{}", b.ahead));
            }
            if b.behind > 0 {
                if b.ahead > 0 {
                    s.push(' ');
                }
                s.push_str(&format!("↓{}", b.behind));
            }
            s
        } else {
            String::new()
        };
        lines.push(Line::from(vec![
            Span::styled(if b.is_current { "● " } else { "  " }, style),
            Span::styled(b.name.clone(), style),
            Span::styled(sync, theme.dim()),
        ]));
    }
    if !d.stashes.is_empty() {
        detail_section(&mut lines, format!("Stashes ({})", d.stashes.len()));
        for s in &d.stashes {
            lines.push(Line::from(Span::raw(s.clone())));
        }
    }
    render_remote_sections(&mut lines, dv, theme);

    // Collapse the modal to the taller pane's content (its rows + the pane's own
    // border) rather than a fixed 84% box, so short repos don't show acres of
    // whitespace. `detail_rect` caps it at 84% of the screen.
    // +1 column-header row + 1 (title/header rule and hint footer are absorbed by
    // detail_rect's own border allowance); no inner pane borders any more.
    let content_rows = clines.len().max(lines.len()) as u16;
    let area = detail_rect(full, content_rows + 3);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::new(1, 0, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_header(frame, area, inner, theme, Line::from(title).bold());
    let inner = render_modal_hints(frame, area, inner, app, theme);

    // Two columns split by a solid vertical divider that ties into the rules above
    // and below — no inner boxes, the outer frame is the only box — matching the
    // standup: repo state on the left, commits on the right.
    let header_rule_y = inner.y.saturating_sub(1);
    let footer_rule_y = inner.y + inner.height;
    let [left_col, divider_col, right_col] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .spacing(1)
    .areas(inner);

    // The solid divider joins the header rule (`┬`) and the footer rule (`┴`) so
    // the pane reads as one framed table.
    for y in divider_col.y..divider_col.y + divider_col.height {
        frame.render_widget(
            Paragraph::new(Span::raw("│")),
            Rect {
                x: divider_col.x,
                y,
                width: 1,
                height: 1,
            },
        );
    }
    for (ch, y) in [("┬", header_rule_y), ("┴", footer_rule_y)] {
        frame.render_widget(
            Paragraph::new(Span::raw(ch)),
            Rect {
                x: divider_col.x,
                y,
                width: 1,
                height: 1,
            },
        );
    }

    // Each column is a bold header row, then its scrolling body. Both bodies share
    // `dv.scroll`, each clamped to its own overflow — so a short column stops while
    // the longer keeps scrolling, and either shows a scrollbar only when it needs one.
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let [left_hdr, left_body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(left_col);
    let [right_hdr, right_body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(right_col);
    frame.render_widget(
        Paragraph::new(Span::styled("Changes · branches · PRs", bold)),
        left_hdr,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!("Commits ({})", d.recent_commits.len()),
            bold,
        )),
        right_hdr,
    );

    let c_max = (clines.len() as u16).saturating_sub(right_body.height);
    let l_max = (lines.len() as u16).saturating_sub(left_body.height);
    dv.set_max_scroll(c_max.max(l_max));
    render_scrolling_pane(frame, right_body, clines, dv.scroll.min(c_max));
    render_scrolling_pane(frame, left_body, lines, dv.scroll.min(l_max));
}

/// A centred detail-modal rect sized to `content_rows` of pane content (plus the
/// pane border + the outer border), at 84% width, and capped at 84% of the
/// screen height with a sensible floor.
pub(crate) fn detail_rect(full: Rect, content_rows: u16) -> Rect {
    let max_h = (full.height * 84 / 100).max(7);
    let h = (content_rows + 4).clamp(7, max_h); // +2 pane border, +2 outer border
    let w = (full.width * 84 / 100).max(20);
    Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    }
}

/// Render `lines` into `inner` scrolled by `offset`, reserving the rightmost
/// column for a scrollbar when the content overflows the pane.
pub(crate) fn render_scrolling_pane(
    frame: &mut Frame,
    inner: Rect,
    lines: Vec<Line<'static>>,
    offset: u16,
) {
    let total = lines.len() as u16;
    let viewport = inner.height;
    let overflow = total > viewport;
    let [text_area, bar_area] = if overflow {
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(inner)
    } else {
        [inner, Rect { width: 0, ..inner }]
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).scroll((offset, 0)),
        text_area,
    );
    if overflow {
        let mut sb = ScrollbarState::new(total as usize)
            .position(offset as usize)
            .viewport_content_length(viewport as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            bar_area,
            &mut sb,
        );
    }
}

/// Append the GitHub PR + contributor sections to the detail pane's left column,
/// with loading / no-data states so an empty section reads as "nothing" rather
/// than "broken."
pub(crate) fn render_remote_sections(
    lines: &mut Vec<Line<'static>>,
    dv: &crate::app::DetailView,
    theme: &Theme,
) {
    // A one-line GitHub summary (open issues + latest release) above the lists.
    if let Some(r) = &dv.remote {
        detail_section(lines, "GitHub".to_string());
        let release = r.latest_release.clone().unwrap_or_else(|| "—".to_string());
        lines.push(Line::from(vec![
            Span::raw(format!("{} open issues", r.open_issues)),
            Span::styled(format!("  ·  latest {release}"), theme.dim()),
        ]));
    }

    detail_section(
        lines,
        match dv.remote.as_ref().map(|r| r.prs.len()) {
            Some(n) => format!("Pull requests ({n})"),
            None => "Pull requests".to_string(),
        },
    );
    match (&dv.remote, dv.remote_pending) {
        (_, true) => lines.push(Line::from(Span::styled("loading…", theme.dim()))),
        (Some(r), _) if r.prs.is_empty() => {
            lines.push(Line::from(Span::styled("none open", theme.dim())));
        }
        (Some(r), _) => {
            for pr in &r.prs {
                let mut spans = vec![
                    Span::styled(format!("#{} ", pr.number), theme.ahead()),
                    Span::raw(pr.title.clone()),
                ];
                if !pr.author.is_empty() {
                    spans.push(Span::styled(format!("  @{}", pr.author), theme.dim()));
                }
                if pr.draft {
                    spans.push(Span::styled("  draft", theme.warn()));
                }
                lines.push(Line::from(spans));
            }
        }
        (None, _) => lines.push(Line::from(Span::styled(
            "needs a GitHub remote + token",
            theme.dim(),
        ))),
    }

    detail_section(
        lines,
        match dv.remote.as_ref().map(|r| r.contributors.len()) {
            Some(n) => format!("Contributors ({n})"),
            None => "Contributors".to_string(),
        },
    );
    match (&dv.remote, dv.remote_pending) {
        (_, true) => lines.push(Line::from(Span::styled("loading…", theme.dim()))),
        (Some(r), _) if r.contributors.is_empty() => {
            lines.push(Line::from(Span::styled("—", theme.dim())));
        }
        (Some(r), _) => {
            for c in &r.contributors {
                lines.push(Line::from(vec![
                    Span::raw(format!("@{}", c.login)),
                    Span::styled(format!("  ×{}", c.contributions), theme.dim()),
                ]));
            }
        }
        (None, _) => {}
    }
}

/// `CI failing  ·  2 open pull requests` — the state of the default branch's
/// checks in a word (so it reads without colour) and how many pull requests
/// are open.
fn checks_line(remote: &cohors_core::RemoteInfo, theme: &Theme) -> Line<'static> {
    let (ci, style) = match remote.ci {
        CiStatus::Passing => ("CI passing", theme.ok()),
        CiStatus::Failing => ("CI failing", theme.risk()),
        CiStatus::Pending => ("CI pending", theme.warn()),
        CiStatus::None => ("no CI runs", theme.dim()),
    };
    let prs = text::count(
        remote.open_prs as usize,
        "open pull request",
        "open pull requests",
    );
    Line::from(vec![
        Span::styled(ci, style),
        Span::styled(format!("  ·  {prs}"), theme.dim()),
    ])
}

/// The command-run overlay: a left list of repos (status glyph + name, the
/// focused one highlighted) and the focused repo's scrollable output on the
/// right, with a live combined pass/fail summary in the bottom border.
pub(crate) fn render_command_run(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let Some(run) = &app.run else {
        return;
    };
    let (ok, fail, running) = run.summary();
    let summary = if running > 0 {
        format!("{ok} ✓ · {fail} ✗ · {running} running")
    } else {
        format!("{ok} ✓ · {fail} ✗")
    };

    // Auto-collapse: size to the taller column (the repo list vs the busiest repo's
    // output) so a short command like `git status` doesn't fill an 85%-tall box.
    // Capped at 85% of the screen (then it scrolls) and floored so it never looks
    // cramped. Measured over *all* repos so the box stays put as you switch focus.
    let busiest_out = run
        .results
        .iter()
        .map(|r| match &r.state {
            RunState::Done { stdout, stderr, .. } => {
                2 + stdout.lines().count()
                    + if stderr.is_empty() {
                        0
                    } else {
                        1 + stderr.lines().count()
                    }
            }
            RunState::Running => 1,
        })
        .max()
        .unwrap_or(1);
    let content_rows = run.results.len().max(busiest_out) as u16;
    let w = (full.width * 88 / 100).max(24);
    // content + title(1) + header rule(1) + column header(1) + footer rule(1) +
    // hints(1) + border(2).
    let h = (content_rows + 7).clamp(10, (full.height * 85 / 100).max(10));
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let block = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::new(1, 1, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let header = Line::from(vec![
        Span::styled(format!("Run · {}", run.command), Style::new().bold()),
        Span::styled(format!("   ·  {summary}"), theme.dim()),
    ]);
    let inner = render_modal_header(frame, area, inner, theme, header);
    let inner = render_modal_hints(frame, area, inner, app, theme);

    // Two columns split by a solid divider joined into the rules (`┬`/`┴`), like the
    // standup: the repo list on the left, the focused repo's output on the right.
    let header_rule_y = inner.y.saturating_sub(1);
    let footer_rule_y = inner.y + inner.height;
    let [repos_col, divider_col, out_col] = Layout::horizontal([
        Constraint::Length(22),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .spacing(1)
    .areas(inner);
    for y in divider_col.y..divider_col.y + divider_col.height {
        frame.render_widget(
            Paragraph::new(Span::raw("│")),
            Rect {
                x: divider_col.x,
                y,
                width: 1,
                height: 1,
            },
        );
    }
    for (ch, y) in [("┬", header_rule_y), ("┴", footer_rule_y)] {
        frame.render_widget(
            Paragraph::new(Span::raw(ch)),
            Rect {
                x: divider_col.x,
                y,
                width: 1,
                height: 1,
            },
        );
    }

    // Each column: a bold header, then its body.
    let [repos_hdr, repos_list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(repos_col);
    let focused_name = run
        .results
        .get(run.focus)
        .map(|r| r.name.as_str())
        .unwrap_or("");
    let [out_hdr, out_body] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(out_col);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    frame.render_widget(Paragraph::new(Span::styled("Repos", bold)), repos_hdr);
    frame.render_widget(
        Paragraph::new(Span::styled(focused_name.to_string(), bold)),
        out_hdr,
    );

    // Left column: status glyph + name; the focused row takes the soft wash.
    let spin = theme.glyphs.spinner(app.spinner);
    let name_w = (repos_list_area.width as usize).saturating_sub(4);
    let items: Vec<ListItem> = run
        .results
        .iter()
        .map(|r| {
            let (glyph, style, note) = match &r.state {
                RunState::Running => (spin.to_string(), theme.ahead(), String::new()),
                RunState::Done { code: 0, .. } => ("✓".to_string(), theme.ok(), String::new()),
                RunState::Done { code, .. } => ("✗".to_string(), theme.risk(), format!(" {code}")),
            };
            ListItem::new(Line::from(vec![
                Span::styled(glyph, style),
                Span::raw(" "),
                Span::raw(ellipsize(&r.name, name_w)),
                Span::styled(note, theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection());
    let mut list_state = ListState::default();
    if !run.results.is_empty() {
        list_state.select(Some(run.focus));
    }
    frame.render_stateful_widget(list, repos_list_area, &mut list_state);

    // Right column: the focused repo's output (stdout, then a dim stderr divider),
    // scrollable.
    let out_lines = run_output_lines(run, theme);
    let total = out_lines.len() as u16;
    let viewport = out_body.height;
    let max_scroll = total.saturating_sub(viewport);
    run.set_max_scroll(max_scroll);
    let offset = run.scroll.min(max_scroll);
    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(out_body);
    frame.render_widget(
        Paragraph::new(Text::from(out_lines)).scroll((offset, 0)),
        text_area,
    );
    if max_scroll > 0 {
        let mut sb = ScrollbarState::new(total as usize)
            .position(offset as usize)
            .viewport_content_length(viewport as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            bar_area,
            &mut sb,
        );
    }
}

/// The lines for the focused repo's output pane.
pub(crate) fn run_output_lines(run: &crate::app::CommandRun, theme: &Theme) -> Vec<Line<'static>> {
    match run.results.get(run.focus).map(|r| &r.state) {
        Some(RunState::Done {
            code,
            stdout,
            stderr,
        }) => {
            let exit_style = if *code == 0 { theme.ok() } else { theme.risk() };
            let mut lines = vec![
                Line::from(Span::styled(format!("exit {code}"), exit_style)),
                Line::from(""),
            ];
            for l in stdout.lines() {
                lines.push(Line::from(l.to_string()));
            }
            if !stderr.is_empty() {
                lines.push(Line::from(Span::styled("── stderr ──", theme.dim())));
                for l in stderr.lines() {
                    lines.push(Line::from(Span::styled(l.to_string(), theme.warn())));
                }
            }
            lines
        }
        Some(RunState::Running) => vec![Line::from(Span::styled("running…", theme.dim()))],
        None => Vec::new(),
    }
}

/// The confirmation modal: the prompt, the affected repos, and y/N choices with
/// No as the default. Reached only via a deliberate action key (ADR-008/021).
pub(crate) fn render_confirm(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let Some(pending) = &app.confirm else {
        return;
    };
    let ids = match &pending.action {
        ConfirmAction::BulkStash(ids) => ids.as_slice(),
        ConfirmAction::BulkCommit { ids, .. } => ids.as_slice(),
        ConfirmAction::AgentSetup(id) => std::slice::from_ref(id),
    };
    let names: Vec<&str> = ids
        .iter()
        .filter_map(|id| {
            app.repos
                .iter()
                .find(|r| &r.id == id)
                .map(|r| r.name.as_str())
        })
        .collect();
    // Size to the content (prompt + affected repos) so the dialog never clips. The
    // y / n keys live in the footer hint bar (`y confirm · N / Esc cancel`), so the
    // body doesn't repeat them.
    let names_str = names.join(", ");
    let w = 60.min(full.width.saturating_sub(4)).max(24);
    let inner_w = w.saturating_sub(4).max(1); // borders(2) + h-padding(2)
    let wrapped = |s: &str| (s.chars().count() as u16).div_ceil(inner_w).max(1);
    let content_h = wrapped(&pending.prompt) + 1 + wrapped(&names_str); // prompt + gap + names
    let h = (content_h + 6) // +2 header, +2 footer, +2 border
        .min(full.height.saturating_sub(2))
        .max(7);
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_header(frame, area, inner, theme, Line::from("Confirm").bold());
    let inner = render_modal_hints(frame, area, inner, app, theme);
    let lines = vec![
        Line::from(Span::styled(
            pending.prompt.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(names_str, theme.dim())),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

/// The "Open with…" picker: a centered list of installed editors plus "Reveal
/// in folder" and lazygit, with the current default marked. `↑/↓` choose, `⏎`
/// opens, `d` sets the highlighted editor as the default.
pub(crate) fn render_open_with(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let Some(ow) = &app.open_with else {
        return;
    };

    let repo = app.selected_repo().map(|r| r.name.as_str()).unwrap_or("");
    let title = if repo.is_empty() {
        "Open with…".to_string()
    } else {
        format!("Open {repo} with…")
    };

    // When PATH has no editor CLI, the list is just Reveal/lazygit — explain why
    // and how to fix it, in a two-line note above the list.
    let no_editor = !ow
        .openers
        .iter()
        .any(|o| matches!(o, Opener::Editor { .. }));
    let note_h: u16 = if no_editor { 2 } else { 0 };

    // Size to the content: fixed-ish width, height = items + note + borders + 2
    // for the header (title + rule) + 2 for the hint footer (rule + keys).
    let w = 52.min(full.width.saturating_sub(2)).max(20);
    let h = (ow.openers.len() as u16 + note_h + 6)
        .min(full.height.saturating_sub(2))
        .max(3);
    let x = full.x + full.width.saturating_sub(w) / 2;
    let y = full.y + full.height.saturating_sub(h) / 2;
    let area = Rect {
        x,
        y,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let block = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_header(frame, area, inner, theme, Line::from(title).bold());
    let inner = render_modal_hints(frame, area, inner, app, theme);

    // Split off the note row(s) when there are no detected editors.
    let list_area = if no_editor {
        let [note, list] =
            Layout::vertical([Constraint::Length(note_h), Constraint::Min(0)]).areas(inner);
        let lines = vec![
            Line::from(Span::styled(
                "No editor CLI found on your PATH.",
                theme.warn(),
            )),
            Line::from(Span::styled(
                "Install its shell command to list it here.",
                theme.dim(),
            )),
        ];
        frame.render_widget(Paragraph::new(Text::from(lines)), note);
        list
    } else {
        inner
    };

    let items: Vec<ListItem> = ow
        .openers
        .iter()
        .map(|o| match o {
            Opener::Editor { command, label } => {
                let mut spans = vec![Span::raw(label.clone())];
                if ow.default_command.as_deref() == Some(command.as_str()) {
                    spans.push(Span::styled("  · default", theme.ahead()));
                }
                ListItem::new(Line::from(spans))
            }
            Opener::Reveal => ListItem::new(Span::styled("Reveal in file manager", theme.dim())),
            Opener::Lazygit => ListItem::new(Span::styled("Open in lazygit", theme.dim())),
        })
        .collect();
    // The soft selection wash (matching the fleet list and the other modals), never
    // a hard reverse that buries a dim row like "Reveal in file manager".
    let list = List::new(items).highlight_style(theme.selection());
    let mut state = ListState::default();
    state.select(Some(ow.cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
}

/// The "push which branch?" picker for a repo with several local branches: each
/// branch (current first, marked `●`) with its push state — `↑2` to push, `local`
/// for a never-pushed branch, "up to date" when synced. `↑/↓` choose, `⏎` pushes
/// the highlighted one. Reached from `P` when the target repo is multi-branch.
pub(crate) fn render_push_branch(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let Some(pick) = &app.push_pick else {
        return;
    };
    let title = format!("Push {} · which branch?", pick.name);

    // Size to the content: items + header (title + rule) + hint footer + border.
    let w = 56.min(full.width.saturating_sub(2)).max(24);
    let h = (pick.branches.len() as u16 + 6)
        .min(full.height.saturating_sub(2))
        .max(5);
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(FRAME)
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_header(frame, area, inner, theme, Line::from(title).bold());
    let inner = render_modal_hints(frame, area, inner, app, theme);

    let items: Vec<ListItem> = pick
        .branches
        .iter()
        .map(|b| {
            let style = if b.is_current {
                theme.ahead().add_modifier(Modifier::BOLD)
            } else {
                Style::new()
            };
            let mut spans = vec![
                Span::styled(if b.is_current { "● " } else { "  " }, style),
                Span::styled(b.name.clone(), style),
            ];
            let sync = if !b.has_upstream {
                "  local".to_string()
            } else {
                match (b.ahead, b.behind) {
                    (0, 0) => "  up to date".to_string(),
                    (a, 0) => format!("  ↑{a}"),
                    (0, d) => format!("  ↓{d}"),
                    (a, d) => format!("  ↑{a} ↓{d}"),
                }
            };
            spans.push(Span::styled(sync, theme.dim()));
            if b.is_current {
                spans.push(Span::styled("  · current", theme.dim()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    let list = List::new(items).highlight_style(theme.selection());
    let mut state = ListState::default();
    state.select(Some(pick.cursor));
    frame.render_stateful_widget(list, inner, &mut state);
}
