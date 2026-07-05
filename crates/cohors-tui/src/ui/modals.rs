use super::*;

pub(crate) fn render_help(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let s = |t: &str, st: Style| Span::styled(t.to_string(), st);

    // One shared two-column grid for the whole overlay: a 2-space-indented left
    // cell (a colored glyph example, or a key) is padded so *every* description
    // starts in the same column — so the legend and the keymap read as one
    // aligned table instead of a wrapping paragraph.
    let desc_col = 16usize;
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

    let lines = vec![
        // The legend first: it answers "what am I looking at?", which is the
        // question a new user has before they care about keybindings.
        head("Legend — what each column shows"),
        row(vec![s("↑2", theme.ahead())], "commits ahead of upstream"),
        row(vec![s("↓5", theme.behind())], "commits behind upstream"),
        row(
            vec![s(theme.glyphs.synced(), theme.ok())],
            "in sync with upstream",
        ),
        row(vec![s("local", theme.dim())], "no upstream (local only)"),
        row(
            vec![
                s(theme.glyphs.ci_dot(CiStatus::Passing), theme.ok()),
                Span::raw(" "),
                s(theme.glyphs.ci_dot(CiStatus::Failing), theme.risk()),
                Span::raw(" "),
                s(theme.glyphs.ci_dot(CiStatus::Pending), theme.warn()),
            ],
            "CI: pass / fail / pending",
        ),
        row(vec![s("2pr", theme.dim())], "open pull requests"),
        row(vec![s("3", theme.staged())], "changed files, all staged"),
        row(
            vec![s("3", theme.modified())],
            "changed files, some unstaged",
        ),
        row(vec![s("s1", theme.dim())], "stash entries"),
        row(vec![s("name", theme.dim())], "row: clean"),
        row(vec![s("name", bold)], "row: needs attention"),
        row(vec![s("name", theme.error())], "row: unreadable (error)"),
        row(
            vec![s(theme.glyphs.marked(), theme.ahead())],
            "row: marked for a bulk action",
        ),
        Line::from(""),
        head("Navigation"),
        row(key("↑ / ↓"), "move cursor"),
        row(key("Home / End"), "top / bottom"),
        Line::from(""),
        head("Select & filter"),
        row(key("Space"), "mark / unmark the repo"),
        row(key("a"), "mark all (again to clear)"),
        row(key("Esc"), "clear the selection"),
        row(key("/"), "fuzzy filter (Esc clears)"),
        row(
            key(":"),
            "command mode (:fetch, :!cmd shell, :sort, :jump …)",
        ),
        row(key("d"), "toggle needs-attention"),
        row(key("s"), "cycle sort mode"),
        row(key("Tab"), "weekly standup"),
        Line::from(""),
        head("Act — on the marked repos, else the current one"),
        row(key("⏎"), "inspect repo — commits, changes, branches"),
        row(key("o"), "open with… (editors, reveal, lazygit)"),
        row(key("f / F"), "fetch selection / all"),
        row(key("p"), "pull (fast-forward only)"),
        row(key("P"), "push (current branch upstream)"),
        row(key("!"), "shell command (opens the : palette)"),
        row(key("S"), "stash (asks to confirm)"),
        row(key("L"), "open in lazygit"),
        row(key("y"), "copy path to clipboard"),
        Line::from(""),
        head("App"),
        row(key("r"), "refresh (re-scan)"),
        row(key("h"), "show / hide the hint bar"),
        row(key("?"), "toggle this help"),
        row(key("q / Ctrl-C"), "quit"),
        Line::from(""),
        Line::from(Span::styled(
            format!("cohors v{}", env!("CARGO_PKG_VERSION")),
            theme.dim(),
        )),
        Line::from(Span::styled(
            format!("config: {}", app.config_path),
            theme.dim(),
        )),
    ];

    // Collapse the box to its content (capped at 90% of the screen height) so the
    // help doesn't sit in a tall, mostly-empty modal. + padding (2) + border (2)
    // + the in-box hint footer (2).
    let want_h = lines.len() as u16 + 6;
    let h = want_h.min((full.height * 90 / 100).max(10));
    let w = (full.width * 70 / 100).max(40);
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(" Help ")
        .padding(Padding::new(2, 2, 1, 1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_hints(frame, area, inner, app, theme);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
        inner,
    );
}

/// The standup overlay: a "Repos" box (per-repo commit counts) beside a box of
/// the focused repo's commits, scrollable. The active pane's title is bold; the
/// other is dim. `None` shows a "collecting" placeholder.
/// Below this inner width a two-pane overlay can't show both panes side-by-side
/// without squeezing one of them, so it stacks the list on top instead.
pub(crate) const TWO_PANE_MIN_WIDTH: u16 = 64;

/// Lay out a two-pane overlay (a list + a detail view). On a wide terminal the
/// panes sit side-by-side (`list_w` wide list, gap, detail fills the rest); on a
/// narrow one the list stacks on top (`stacked_list_h` tall) so neither pane is
/// squeezed. Returns `(list_area, detail_area)`.
pub(crate) fn two_pane(inner: Rect, list_w: u16, stacked_list_h: u16) -> (Rect, Rect) {
    if inner.width >= TWO_PANE_MIN_WIDTH {
        let [list, detail] = Layout::horizontal([Constraint::Length(list_w), Constraint::Min(0)])
            .spacing(2)
            .areas(inner);
        (list, detail)
    } else {
        let [list, detail] =
            Layout::vertical([Constraint::Length(stacked_list_h), Constraint::Min(0)])
                .spacing(1)
                .areas(inner);
        (list, detail)
    }
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
    // body + description(3) + gap(1) + pane border(2) + outer border(2) + pad(1).
    let h = (body + 9).min(full.height.saturating_sub(2)).max(12);
    let w = (full.width as u32 * 84 / 100) as u16;
    let area = Rect {
        x: full.x + full.width.saturating_sub(w) / 2,
        y: full.y + full.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, area);

    let window = app.standup_window.label();
    // The outer frame, padded so the inner boxes breathe.
    let outer = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" Standup · {window} ")).bold())
        .padding(Padding::new(1, 1, 1, 0));

    // Still walking the commits: a placeholder.
    let Some(view) = &app.standup else {
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
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
    let summary = format!(
        " {total} commit{} · {repos} repo{} ",
        if total == 1 { "" } else { "s" },
        if repos == 1 { "" } else { "s" },
    );
    // The count rides the top-right of the frame; the bottom row inside carries
    // the key hints.
    let outer = outer.title(Line::from(summary).right_aligned().style(theme.dim()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
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
    // The description (up to 3 wrapped lines), then a blank gap, then the panes.
    let [desc_area, _gap, panes_area] = Layout::vertical([
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

    // A "Repos" box and a "{repo}" commits box: side-by-side when there's room,
    // stacked (repos on top) on a narrow terminal.
    let (left_area, right_area) = two_pane(panes_area, 26, 8);

    // Highlight the active pane's title (bold) and dim the inactive one.
    let active = |on: bool| {
        if on {
            Style::new().add_modifier(Modifier::BOLD)
        } else {
            theme.dim()
        }
    };

    // Left: the repos with commit counts.
    let repos_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(" Repos ").style(active(!view.commits_focused)))
        .padding(Padding::horizontal(1));
    let repos_inner = repos_block.inner(left_area);
    frame.render_widget(repos_block, left_area);
    let items: Vec<ListItem> = view
        .groups
        .iter()
        .map(|(repo, commits)| {
            ListItem::new(Line::from(vec![
                Span::raw(ellipsize(repo, 16)),
                Span::styled(format!("  {}", commits.len()), theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    list_state.select(Some(view.focus));
    frame.render_stateful_widget(list, repos_inner, &mut list_state);

    // Right: the focused repo's commits, with the commit count in the title and
    // a highlighted cursor (so scrolling is contextual).
    let n = view.focused_len();
    let commits_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(
            Line::from(format!(
                " {repo_name}  ·  {n} commit{} ",
                if n == 1 { "" } else { "s" }
            ))
            .style(active(view.commits_focused)),
        )
        .padding(Padding::new(1, 0, 0, 0));
    let commits_inner = commits_block.inner(right_area);
    frame.render_widget(commits_block, right_area);

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
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(commits_inner);

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

    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
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
        let area = detail_rect(full, 3); // 1 content row + 2 for the hint footer
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from(title).bold())
            .padding(Padding::new(1, 0, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
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
    if let Some(snap) = app.repos.iter().find(|r| r.name == dv.repo_name) {
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
    let content_rows = clines.len().max(lines.len()) as u16;
    let area = detail_rect(full, content_rows + 2); // +2 for the hint footer
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title).bold())
        .padding(Padding::new(1, 0, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_hints(frame, area, inner, app, theme);

    // Two panes (like the standup): repo state on the left, commits on the right.
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(1)
            .areas(inner);

    // Both panes share `dv.scroll`, each clamped to its own overflow — so a short
    // pane stops while the longer one keeps scrolling, and either shows a
    // scrollbar only when it actually overflows.
    let commits_inner = render_pane_block(
        frame,
        right_area,
        format!(" Commits ({}) ", d.recent_commits.len()),
        theme,
    );
    let state_inner = render_pane_block(
        frame,
        left_area,
        " Changes · branches · PRs ".to_string(),
        theme,
    );
    let c_max = (clines.len() as u16).saturating_sub(commits_inner.height);
    let l_max = (lines.len() as u16).saturating_sub(state_inner.height);
    dv.set_max_scroll(c_max.max(l_max));

    render_scrolling_pane(frame, commits_inner, clines, dv.scroll.min(c_max));
    render_scrolling_pane(frame, state_inner, lines, dv.scroll.min(l_max));
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

/// Draw a detail sub-pane's bordered block and return its inner area.
pub(crate) fn render_pane_block(
    frame: &mut Frame,
    area: Rect,
    title: String,
    theme: &Theme,
) -> Rect {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.dim())
        .title(Line::from(title).bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
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

/// The command-run overlay: a left list of repos (status glyph + name, the
/// focused one highlighted) and the focused repo's scrollable output on the
/// right, with a live combined pass/fail summary in the bottom border.
pub(crate) fn render_command_run(frame: &mut Frame, full: Rect, app: &App, theme: &Theme) {
    let area = centered_rect(88, 85, full);
    frame.render_widget(Clear, area);
    let Some(run) = &app.run else {
        return;
    };
    let (ok, fail, running) = run.summary();
    let summary = if running > 0 {
        format!(" {ok} ✓ · {fail} ✗ · {running} running ")
    } else {
        format!(" {ok} ✓ · {fail} ✗ ")
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" Run · {} ", run.command)).bold())
        .title(Line::from(summary).right_aligned())
        .padding(Padding::new(1, 0, 0, 0));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_hints(frame, area, inner, app, theme);

    // Two boxed panes — a "Repos" list and the focused repo's output — matching
    // the standup view's structure. Side-by-side, stacking on a narrow terminal.
    let (list_area, out_area) = two_pane(inner, 26, 6);

    // Left: the repos box.
    let repos_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(" Repos ").bold())
        .padding(Padding::horizontal(1));
    let repos_inner = repos_block.inner(list_area);
    frame.render_widget(repos_block, list_area);

    let spin = spinner_frame(app.spinner);
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
                Span::raw(ellipsize(&r.name, 16)),
                Span::styled(note, theme.dim()),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default();
    if !run.results.is_empty() {
        list_state.select(Some(run.focus));
    }
    frame.render_stateful_widget(list, repos_inner, &mut list_state);

    // Right: the focused repo's output box (stdout, then a dim stderr divider).
    let focused_name = run
        .results
        .get(run.focus)
        .map(|r| r.name.as_str())
        .unwrap_or("");
    let out_block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(format!(" {focused_name} ")).bold())
        .padding(Padding::new(1, 0, 0, 0));
    let out_inner = out_block.inner(out_area);
    frame.render_widget(out_block, out_area);

    let out_lines = run_output_lines(run, theme);
    let total = out_lines.len() as u16;
    let viewport = out_inner.height;
    let max_scroll = total.saturating_sub(viewport);
    run.set_max_scroll(max_scroll);
    let offset = run.scroll.min(max_scroll);

    let [text_area, bar_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(1)]).areas(out_inner);
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
        ConfirmAction::BulkStash(ids) => ids,
        ConfirmAction::BulkCommit { ids, .. } => ids,
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
    let area = centered_rect(54, 32, full);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(" Confirm ").bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = render_modal_hints(frame, area, inner, app, theme);
    let lines = vec![
        Line::from(Span::styled(
            pending.prompt.clone(),
            Style::new().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(names.join(", "), theme.dim())),
        Line::from(""),
        Line::from(vec![
            Span::styled("[y]", theme.ok()),
            Span::raw(" yes    "),
            Span::styled("[N]", theme.warn()),
            Span::raw(" no    "),
            Span::styled("(Esc cancels)", theme.dim()),
        ]),
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
        " Open with… ".to_string()
    } else {
        format!(" Open {repo} with… ")
    };

    // When PATH has no editor CLI, the list is just Reveal/lazygit — explain why
    // and how to fix it, in a two-line note above the list.
    let no_editor = !ow
        .openers
        .iter()
        .any(|o| matches!(o, Opener::Editor { .. }));
    let note_h: u16 = if no_editor { 2 } else { 0 };

    // Size to the content: fixed-ish width, height = items + note + borders + 2
    // for the in-box hint footer (rule + keys).
    let w = 52.min(full.width.saturating_sub(2)).max(20);
    let h = (ow.openers.len() as u16 + note_h + 4)
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
        .border_type(BorderType::Rounded)
        .title(Line::from(title).bold())
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
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
    let list = List::new(items)
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▌ ");
    let mut state = ListState::default();
    state.select(Some(ow.cursor));
    frame.render_stateful_widget(list, list_area, &mut state);
}

/// A rect centered within `area`, sized as a percentage of it.
pub(crate) fn centered_rect(pct_x: u16, pct_y: u16, area: Rect) -> Rect {
    let [_, vmid, _] = Layout::vertical([
        Constraint::Percentage((100 - pct_y) / 2),
        Constraint::Percentage(pct_y),
        Constraint::Percentage((100 - pct_y) / 2),
    ])
    .areas(area);
    let [_, hmid, _] = Layout::horizontal([
        Constraint::Percentage((100 - pct_x) / 2),
        Constraint::Percentage(pct_x),
        Constraint::Percentage((100 - pct_x) / 2),
    ])
    .areas(vmid);
    hmid
}

/// A fixed-height rect centered vertically within `area`.
pub(crate) fn center_v(area: Rect, height: u16) -> Rect {
    let [_, mid, _] = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(height),
        Constraint::Min(0),
    ])
    .areas(area);
    mid
}
