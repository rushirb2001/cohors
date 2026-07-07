use super::*;

pub(crate) fn render_repos_panel(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    now: i64,
    theme: &Theme,
) {
    // The list's view state lives in its own title: the (visible) count in bold,
    // then the sort mode, the needs-attention filter, and the active fuzzy query
    // in dim — so a narrowed list always says on-screen WHY it is narrowed
    // (matching the web, where the filter input and toggle stay visible).
    let mut title = vec![Span::styled(
        format!(" Repositories ({}) ", app.visible_len()),
        Style::new().add_modifier(Modifier::BOLD),
    )];
    title.push(Span::styled(
        format!("· sort: {} ", app.sort.label()),
        theme.dim(),
    ));
    if app.dirty_only {
        title.push(Span::styled("· needs attention ", theme.dim()));
    }
    if !app.filter.is_empty() {
        title.push(Span::styled(format!("· /{} ", app.filter), theme.dim()));
    }
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Line::from(title));
    // Transient feedback (scan progress, action results) rides the top-right of
    // this box's border — a self-dismissing toast that doesn't crowd the header.
    if let Some(toast) = toast_line(app, theme) {
        block = block.title(toast.right_aligned());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // When filtering, a boxed input sits at the top of the panel — above the
    // column headers — with a muted placeholder, matching the web's filter field.
    let inner = if app.mode == Mode::Filter {
        let [filter_area, rest] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(inner);
        render_filter_input(frame, filter_area, app, theme);
        rest
    } else {
        inner
    };

    // The context pane lives *inside* this box. When the panel is tall enough,
    // reserve its bottom rows (a divider rule + a blank pad + the content) for the
    // selected repo's facts and lay the table out in what's left; on short panels
    // it doesn't appear and the table uses the whole inner area. `dock_visible`
    // also drives the table's expanded-column layout.
    let dock = build_dock(app, now, theme, inner.width.saturating_sub(2) as usize);
    let dock_reserve = match &dock {
        // rule(1) + the card rows, only if the table keeps ≥6 rows.
        Some(d) if inner.height >= d.rows() + 1 + 6 => d.rows() + 1,
        _ => 0,
    };
    let dock_visible = dock_reserve > 0;
    let [table_inner, dock_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(dock_reserve)]).areas(inner);

    let view = app.view();
    let total = view.len();
    // The header is its own row (no blank margin); the data fills the rest. When
    // the fleet overflows, a scroll hint is reserved above and/or below the data —
    // both at once when repos are hidden in both directions. Because a
    // multi-branch repo occupies several lines, the scroll math counts LINES, not
    // repos (`heights` is one entry per repo, in view order).
    let heights: Vec<usize> = view
        .iter()
        .map(|vr| row_height(&app.repos[vr.index]))
        .collect();
    let total_lines: usize = heights.iter().sum();
    let avail = table_inner.height.saturating_sub(1) as usize; // data lines (header excluded)
    let overflow = total_lines > avail;

    // First assume one hint line; if repos are hidden both above and below, a
    // second hint line is needed (shrinking the data area by one more).
    let vp1 = avail.saturating_sub(usize::from(overflow)).max(1);
    let (off1, cnt1) = window(app.repos_scroll.get(), app.selected, vp1, &heights);
    let two_hints = overflow && off1 > 0 && total.saturating_sub(off1 + cnt1) > 0;
    let (offset, visible) = if two_hints {
        window(
            app.repos_scroll.get(),
            app.selected,
            avail.saturating_sub(2).max(1),
            &heights,
        )
    } else {
        (off1, cnt1)
    };
    app.repos_scroll.set(offset);
    let above = offset;
    let below = total.saturating_sub(offset + visible);

    // header row, then the data, with `↑ N more` below the header when repos are
    // hidden above and `… N more ↓` at the bottom when hidden below — either or
    // both.
    let len1 = || Constraint::Length(1);
    let (header_area, data_area, top_hint, bot_hint) = if !overflow {
        let [hd, dt] = Layout::vertical([len1(), Constraint::Min(0)]).areas(table_inner);
        (hd, dt, None, None)
    } else if above == 0 {
        let [hd, dt, bn] =
            Layout::vertical([len1(), Constraint::Min(0), len1()]).areas(table_inner);
        (hd, dt, None, Some(bn))
    } else if below == 0 {
        let [hd, tn, dt] =
            Layout::vertical([len1(), len1(), Constraint::Min(0)]).areas(table_inner);
        (hd, dt, Some(tn), None)
    } else {
        let [hd, tn, dt, bn] =
            Layout::vertical([len1(), len1(), Constraint::Min(0), len1()]).areas(table_inner);
        (hd, dt, Some(tn), Some(bn))
    };

    // Size each data column to its actual content so it stays tight (a fleet with
    // no PRs gets a narrow PRs column, etc.). The floor is the header-label width;
    // the ceiling guards against one outlier stretching a column. `line_width`
    // measures the same spans the cell will render.
    let col_w = |build: fn(&RepoSnapshot, &Theme) -> Vec<Span<'static>>, floor: u16, ceil: u16| {
        view.iter()
            .map(|vr| line_width(&build(&app.repos[vr.index], theme)))
            .max()
            .unwrap_or(0)
            .clamp(floor, ceil)
    };

    // The commit age column (both layouts show it).
    let age_w: u16 = view
        .iter()
        .map(|vr| match &app.repos[vr.index].last_commit {
            Some(c) => time::relative(c.timestamp, now).chars().count() as u16,
            None => 1,
        })
        .max()
        .unwrap_or(0)
        .clamp(4, 6);

    // Two layouts, one shape: both end in Last (age) + Status (the attention
    // reason that makes the dirty-first ordering self-explaining per row, same
    // as the web). With the dock up the table spends its width on expanded
    // columns: the fused Sync (`↑2 ● 2pr`) and Changes (`4 s1`) split into
    // Sync / PRs / CI and Changes / Stash. With the dock hidden (short
    // terminal) the fused columns stay, but Status remains — the headline
    // signal never disappears with the terminal height (the commit message
    // lives in the dock or the Enter detail instead). The trailing column is
    // Fill so the table spans edge to edge; its text is ellipsized to that
    // width (computed by mirroring the table's own arithmetic: inner width −
    // the 2-col highlight reserve − the fixed columns − the 2-col gaps).
    // Repo and Branch are fused into one `name @branch` column, sized to its
    // widest content (gutter + name + branch) within a cap so a long branch can't
    // run away. This mirrors the web and reclaims a whole column for Status.
    // There is no Changes column on either surface any more: the dirty count is
    // Status's job ("4 uncommitted changes"), exactly as on the web.
    let repo_w = {
        let names = col_w(repo_spans, 12, 30);
        // Multi-branch repos also render `    @branch` lines; size the column to
        // the widest branch line too (4-space indent + `@` + name), same cap.
        let branches = view
            .iter()
            .map(|vr| &app.repos[vr.index])
            .filter(|s| s.branches.len() > 1)
            .flat_map(|s| &s.branches)
            .map(|b| 5 + b.name.chars().count() as u16)
            .max()
            .unwrap_or(0);
        names.max(branches).clamp(12, 30)
    };
    // No Stash column either: like Changes, the stash count is Status's job
    // ("1 stash · stale"), and the dock card spells it out for the selection.
    //
    // The column set degrades gracefully with WIDTH, richest-that-fits, and CI
    // never simply vanishes — the narrow tier keeps a colour-coded CI square in
    // place of the "passing"/"failing" word:
    //   Wide   Repo · Sync · PRs · CI(word) · Last · Activity · Status
    //   Medium Repo · Sync · PRs · CI(word) · Last · Status   (drops the sparkline)
    //   Narrow Repo · Sync · CI(■) · Last · Status            (square CI, no PRs)
    // While scanning, the refreshed columns' floors grow by 2 so the header
    // spinner (` ⠋`) has room beside the label.
    let hpad: u16 = if app.scanning { 2 } else { 0 };
    let sync_x_w = col_w(ahead_behind_spans, 4 + hpad, 9);
    let prs_w = col_w(prs_spans, 3 + hpad, 5);
    let ci_w = col_w(ci_spans, 2 + hpad, 8);
    let ci_sq_w = 2 + hpad; // the square, floored to the "CI" header width
    let act_w: u16 = ACTIVITY_WEEKS as u16; // one cell per week
    let w = inner.width as usize;
    let wide_fixed = (2 + repo_w + sync_x_w + prs_w + ci_w + age_w + act_w) as usize;
    let med_fixed = (2 + repo_w + sync_x_w + prs_w + ci_w + age_w) as usize;
    let layout = if w >= wide_fixed + 2 * 6 + 18 {
        Tier::Wide
    } else if w >= med_fixed + 2 * 5 + 16 {
        Tier::Medium
    } else {
        Tier::Narrow
    };
    let (widths, reason_max): (Vec<Constraint>, usize) = match layout {
        Tier::Wide => {
            let status_w = w.saturating_sub(wide_fixed + 2 * 6); // 7 cols ⇒ 6 gaps
            (
                vec![
                    Constraint::Length(repo_w), // Repo @branch (incl. 2-col gutter)
                    Constraint::Length(sync_x_w),
                    Constraint::Length(prs_w),
                    Constraint::Length(ci_w),
                    Constraint::Length(age_w),
                    Constraint::Length(act_w),
                    Constraint::Fill(1), // Status (primary attention reason)
                ],
                status_w,
            )
        }
        Tier::Medium => {
            let status_w = w.saturating_sub(med_fixed + 2 * 5); // 6 cols ⇒ 5 gaps
            (
                vec![
                    Constraint::Length(repo_w),
                    Constraint::Length(sync_x_w),
                    Constraint::Length(prs_w),
                    Constraint::Length(ci_w),
                    Constraint::Length(age_w),
                    Constraint::Fill(1),
                ],
                status_w,
            )
        }
        Tier::Narrow => {
            let fixed = (2 + repo_w + sync_x_w + ci_sq_w + age_w) as usize;
            let status_w = w.saturating_sub(fixed + 2 * 4); // 5 cols ⇒ 4 gaps
            (
                vec![
                    Constraint::Length(repo_w),
                    Constraint::Length(sync_x_w),
                    Constraint::Length(ci_sq_w),
                    Constraint::Length(age_w),
                    Constraint::Fill(1),
                ],
                status_w,
            )
        }
    };

    let fmt = RowFmt {
        layout,
        reason_max,
        age_w,
        // ~400ms on / ~400ms off at the 100ms tick cadence.
        blink_on: (app.spinner / 4).is_multiple_of(2),
    };
    let spin = spinner_frame(app.spinner);
    let rows: Vec<Row> = view
        .iter()
        .map(|vr| {
            let snap = &app.repos[vr.index];
            // A row spins in its Sync/PRs/CI cells while an action runs on it.
            // A fleet-wide rescan animates beside the column headers instead —
            // replacing every cell made the whole table flicker.
            let busy = app.busy.contains(&snap.id).then_some(spin);
            let marked = app.selection.contains(&snap.id);
            repo_row(snap, &vr.name_highlights, now, theme, busy, marked, &fmt)
        })
        .collect();

    // The header, rendered manually (so the hint can sit between it and the data)
    // but aligned to the data table: a 2-col reserve for the highlight symbol,
    // then the same column widths/spacing. "Repo" is padded by 2 to line up with
    // the names' selection gutter.
    let labels: &[&str] = match layout {
        Tier::Wide => &["  Repo", "Sync", "PRs", "CI", "Last", "Activity", "Status"],
        Tier::Medium => &["  Repo", "Sync", "PRs", "CI", "Last", "Status"],
        Tier::Narrow => &["  Repo", "Sync", "CI", "Last", "Status"],
    };
    let header_style = Style::new().add_modifier(Modifier::BOLD);
    let [_sym, header_cols] =
        Layout::horizontal([Constraint::Length(2), Constraint::Min(0)]).areas(header_area);
    let col_rects = Layout::horizontal(widths.clone())
        .spacing(2)
        .split(header_cols);
    // While a rescan runs, the spinner rides BESIDE the headers of the columns
    // being refreshed (Sync / PRs / CI) — next to the label, never replacing
    // the data underneath.
    for (rect, label) in col_rects.iter().zip(labels.iter()) {
        let mut spans = vec![Span::styled(*label, header_style)];
        if app.scanning && matches!(*label, "Sync" | "PRs" | "CI") {
            spans.push(Span::styled(format!(" {spin}"), theme.ahead()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), *rect);
    }

    let table = Table::new(rows, widths)
        .column_spacing(2)
        .row_highlight_style(theme.selection())
        .highlight_symbol("▌ ");

    let mut state = TableState::default();
    if !view.is_empty() {
        state.select(Some(app.selected));
    }
    // Use the offset we computed above so the hint and the visible rows agree
    // (and the table doesn't re-scroll past our clamp).
    *state.offset_mut() = offset;
    frame.render_stateful_widget(table, data_area, &mut state);

    // The scroll hints, centered on their own rows inside the box: repos hidden
    // above the header and/or below the fold.
    let hint_style = theme.ahead().add_modifier(Modifier::BOLD);
    if let Some(tn) = top_hint {
        frame.render_widget(
            Paragraph::new(
                Line::from(Span::styled(format!("↑ {above} more"), hint_style)).centered(),
            ),
            tn,
        );
    }
    if let Some(bn) = bot_hint {
        frame.render_widget(
            Paragraph::new(
                Line::from(Span::styled(format!("↓ {below} more"), hint_style)).centered(),
            ),
            bn,
        );
    }

    // The context pane, in the reserved bottom rows of this same box.
    if let (true, Some(d)) = (dock_visible, dock.as_ref()) {
        render_inbox_dock(frame, dock_area, d, theme);
    }
}

/// A pre-built docked context pane, laid out before the pane is sized so the box
/// can collapse to its content (no trailing blank rows).
pub(crate) struct DockBox {
    /// The card's heading spans, e.g. `payments  ·  main` — rendered INSIDE the
    /// card, never on the divider rule.
    title: Vec<Span<'static>>,
    /// Right-aligned heading hint (the idle pane's `Enter: full detail`).
    right: Option<&'static str>,
    /// The floating rule's colour — dim when idle, accent while an action runs.
    border: Style,
    /// Left fact column: Changes / Stash / Upstream / Remote (and Operation).
    left: Vec<Line<'static>>,
    /// Right fact column: Release and the wrapped Last commit.
    right_col: Vec<Line<'static>>,
}

impl DockBox {
    /// Content rows: the heading, a pad row, then the taller fact column.
    pub(crate) fn rows(&self) -> u16 {
        2 + self.left.len().max(self.right_col.len()) as u16
    }
}

/// The most message lines the dock shows before truncating with `…`, so one huge
/// commit can't make the collapsing pane dominate the screen.
pub(crate) const DOCK_MSG_MAX: usize = 3;

/// Build the docked context pane's content for inner width `inner_w`: the
/// selected repo at a glance when idle, or live progress while a bulk action
/// runs. `None` when there's nothing to show. It fills the space a short fleet
/// would otherwise leave empty and makes the dashboard a cockpit — move the
/// cursor and the pane follows. Cheap by design: it reads the snapshot we already
/// have (no background fetches); the full remote-enriched view lives behind
/// `Enter`.
pub(crate) fn build_dock(app: &App, now: i64, theme: &Theme, inner_w: usize) -> Option<DockBox> {
    // While a bulk fetch/pull/push/stash runs, the dock shows what's in flight
    // rather than the cursor's repo.
    if !app.busy.is_empty() {
        let spin = spinner_frame(app.spinner);
        let lines: Vec<Line> = app
            .repos
            .iter()
            .filter(|r| app.busy.contains(&r.id))
            .map(|r| {
                Line::from(vec![
                    Span::styled(format!("{spin} "), theme.ahead()),
                    Span::raw(r.name.clone()),
                ])
            })
            .collect();
        return Some(DockBox {
            title: vec![Span::styled(
                format!("Working — {} repo(s)", app.busy.len()),
                theme.ahead().add_modifier(Modifier::BOLD),
            )],
            right: None,
            border: theme.ahead(),
            left: lines,
            right_col: Vec::new(),
        });
    }

    let snap = app.selected_repo()?;
    let branch = match &snap.branch {
        Branch::Named(b) => b.clone(),
        Branch::Detached(id) => format!("@{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => "unborn".to_string(),
    };
    let title = vec![
        Span::styled(snap.name.clone(), Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(format!("  ·  {branch}"), theme.dim()),
    ];

    // A broken repo: just the error, in red — none of the facts below apply.
    if let Some(reason) = &snap.error {
        return Some(DockBox {
            title,
            right: Some("Enter: full detail"),
            // Full-strength (not dim): the separator should read as a solid
            // structural line, matching the box borders around it.
            border: Style::new(),
            left: vec![Line::from(Span::styled(
                format!("⚠ {reason}"),
                theme.error(),
            ))],
            right_col: Vec::new(),
        });
    }

    // Labelled facts — each on its own row with a purple, fixed-width label so
    // the pane reads as a small form, not a cryptic glyph soup. Everything is
    // spelled out (no `s1`/`●`); colour carries urgency. Two columns: the
    // local/remote state on the left, release + last commit on the right.
    let mut lines: Vec<Line> = Vec::new();

    // A paused merge/rebase/… gets its own leading row — it's the most urgent
    // fact about the repo and explains any conflict counts below it.
    if let Some(op) = snap.operation {
        dock_field(
            &mut lines,
            "Operation",
            vec![Span::styled(
                format!("{} in progress", op.label()),
                theme.warn(),
            )],
            theme,
        );
    }

    // Working tree — the staged/modified/untracked breakdown the row's count hides.
    let w = &snap.worktree;
    let changes_val = if w.total() == 0 {
        vec![Span::styled("clean", theme.dim())]
    } else {
        let mut spans: Vec<Span> = Vec::new();
        if w.conflicted > 0 {
            spans.push(Span::styled(
                format!("{} conflicted", w.conflicted),
                theme.risk(),
            ));
        }
        let mut parts = Vec::new();
        if w.staged > 0 {
            parts.push(format!("{} staged", w.staged));
        }
        if w.modified > 0 {
            parts.push(format!("{} modified", w.modified));
        }
        if w.untracked > 0 {
            parts.push(format!("{} untracked", w.untracked));
        }
        if !parts.is_empty() {
            let style = if w.modified > 0 || w.untracked > 0 {
                theme.modified()
            } else {
                theme.staged()
            };
            if !spans.is_empty() {
                spans.push(Span::styled(" · ", theme.dim()));
            }
            spans.push(Span::styled(parts.join(" · "), style));
        }
        spans
    };
    dock_field(&mut lines, "Changes", changes_val, theme);

    // Stash — count, and whether the newest is stale (a forgotten stash).
    let stash_stale = assess(snap, now)
        .reasons
        .iter()
        .any(|r| matches!(r, AttentionReason::Stash { stale: true, .. }));
    let stash_val = if snap.stash_count == 0 {
        vec![Span::styled("none", theme.dim())]
    } else if stash_stale {
        vec![Span::styled(
            format!("{} · stale", snap.stash_count),
            theme.warn(),
        )]
    } else {
        vec![Span::raw(snap.stash_count.to_string())]
    };
    dock_field(&mut lines, "Stash", stash_val, theme);

    // Upstream — even / ahead / behind / no upstream, naming the tracked branch.
    let upstream_val = match &snap.upstream {
        None => vec![Span::styled("no upstream", theme.dim())],
        Some(up) if up.ahead == 0 && up.behind == 0 => {
            vec![Span::styled(format!("even with {}", up.name), theme.dim())]
        }
        Some(up) => {
            let mut spans = Vec::new();
            if up.ahead > 0 {
                spans.push(Span::styled(format!("{} ahead", up.ahead), theme.ahead()));
            }
            if up.ahead > 0 && up.behind > 0 {
                spans.push(Span::styled(" · ", theme.dim()));
            }
            if up.behind > 0 {
                spans.push(Span::styled(
                    format!("{} behind", up.behind),
                    theme.behind(),
                ));
            }
            spans.push(Span::styled(format!("  ({})", up.name), theme.dim()));
            spans
        }
    };
    // A long-unfetched remote means ahead/behind may lie — say so inline.
    let mut upstream_val = upstream_val;
    if let Some(AttentionReason::StaleRemote { days }) = assess(snap, now)
        .reasons
        .iter()
        .find(|r| matches!(r, AttentionReason::StaleRemote { .. }))
    {
        upstream_val.push(Span::styled(
            format!("  ·  not fetched in {days}d"),
            theme.warn(),
        ));
    }
    dock_field(&mut lines, "Upstream", upstream_val, theme);

    // Remote — CI health, open PRs, and any awaiting the user's review.
    let remote_val = match &snap.remote {
        None => vec![Span::styled("no GitHub remote", theme.dim())],
        Some(r) => {
            let (ci_label, ci_style) = match r.ci {
                CiStatus::Passing => ("CI passing", theme.ok()),
                CiStatus::Failing => ("CI failing", theme.risk()),
                CiStatus::Pending => ("CI pending", theme.warn()),
                CiStatus::None => ("no CI", theme.dim()),
            };
            let prs = if r.open_prs == 0 {
                "no open PRs".to_string()
            } else {
                format!(
                    "{} open PR{}",
                    r.open_prs,
                    if r.open_prs == 1 { "" } else { "s" }
                )
            };
            let mut spans = vec![
                Span::styled(ci_label, ci_style),
                Span::styled(format!("  ·  {prs}"), theme.dim()),
            ];
            if r.prs_awaiting_review > 0 {
                spans.push(Span::styled(
                    format!("  ·  {} awaiting your review", r.prs_awaiting_review),
                    theme.warn(),
                ));
            }
            spans
        }
    };
    dock_field(&mut lines, "Remote", remote_val, theme);

    // The left column is complete; the right column (Release + Last commit)
    // wraps to whatever width remains beside it.
    let left_w = lines.iter().map(|l| l.width()).max().unwrap_or(0);
    let right_w = inner_w.saturating_sub(left_w + DOCK_COL_GAP).max(20);
    let mut right_col: Vec<Line> = Vec::new();

    // Release — the newest reachable tag and how far the repo has moved since.
    // Repos with no tag still show the field, with a muted "No Releases" placeholder.
    let release_val = match &snap.latest_tag {
        Some(tag) if tag.commits_since == 0 => {
            vec![Span::styled(format!("{} (current)", tag.name), theme.ok())]
        }
        Some(tag) => vec![
            Span::raw(tag.name.clone()),
            Span::styled(
                format!(
                    "  ·  {} commit{} since",
                    tag.commits_since,
                    if tag.commits_since == 1 { "" } else { "s" }
                ),
                theme.dim(),
            ),
        ],
        None => vec![Span::styled("No Releases", theme.dim())],
    };
    dock_field(&mut right_col, "Release", release_val, theme);

    // Last commit — age + the full message (the row no longer shows it),
    // word-wrapped within the right column rather than truncated.
    match &snap.last_commit {
        None => dock_field(
            &mut right_col,
            "Last commit",
            vec![Span::styled("no commits", theme.dim())],
            theme,
        ),
        Some(c) => {
            let head = format!("{} ago — ", time::relative(c.timestamp, now));
            // First message line shares its row with the label + age; wrapped
            // continuation lines sit under the value column.
            let first_w = right_w
                .saturating_sub(DOCK_LABEL_W + head.chars().count())
                .max(1);
            let rest_w = right_w.saturating_sub(DOCK_LABEL_W).max(1);
            let mut wrapped = word_wrap(&c.summary, first_w, rest_w);

            // Cap to a few lines; mark the cut with an ellipsis.
            if wrapped.len() > DOCK_MSG_MAX {
                wrapped.truncate(DOCK_MSG_MAX);
                let w = if DOCK_MSG_MAX == 1 { first_w } else { rest_w };
                if let Some(last) = wrapped.last_mut() {
                    let mut s: String = last.chars().take(w.saturating_sub(1)).collect();
                    s.push('…');
                    *last = s;
                }
            }

            dock_field(
                &mut right_col,
                "Last commit",
                vec![
                    Span::styled(head, theme.dim()),
                    Span::raw(wrapped.first().cloned().unwrap_or_default()),
                ],
                theme,
            );
            let indent = " ".repeat(DOCK_LABEL_W);
            for chunk in wrapped.iter().skip(1) {
                right_col.push(Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::raw(chunk.clone()),
                ]));
            }
        }
    }

    Some(DockBox {
        title,
        right: Some("Enter: full detail"),
        // Full-strength (not dim): a solid structural line, like the borders.
        border: Style::new(),
        left: lines,
        right_col,
    })
}

/// Greedy word-wrap: split `text` on whitespace into lines that fit `first`
/// columns for the first line and `rest` for the others. A single word wider
/// than the limit is left long (the cell clips it) rather than broken mid-word.
pub(crate) fn word_wrap(text: &str, first: usize, rest: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut width = first;
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            width = rest;
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Width of the dim label column in the dock's labelled facts.
pub(crate) const DOCK_LABEL_W: usize = 13;

/// The gap between the dock card's two fact columns: a space, the floating
/// `│` divider, a space.
pub(crate) const DOCK_COL_GAP: usize = 3;

/// Push one labelled fact row into the dock: a dim, fixed-width label then the
/// value spans.
pub(crate) fn dock_field(
    lines: &mut Vec<Line<'static>>,
    label: &str,
    value: Vec<Span<'static>>,
    theme: &Theme,
) {
    let mut spans = vec![Span::styled(
        format!("{label:<DOCK_LABEL_W$}"),
        theme.fg(SPIDER_PURPLE),
    )];
    spans.extend(value);
    lines.push(Line::from(spans));
}

/// Render a pre-built [`DockBox`] inside the Repositories box, in the reserved
/// bottom rows: a solid rule joined into the box borders (`├`/`┤`), then a
/// grey card — the repo heading (name · branch, `Enter` hint right-aligned)
/// over the two fact columns, split by a floating `│` divider.
pub(crate) fn render_inbox_dock(frame: &mut Frame, area: Rect, dock: &DockBox, theme: &Theme) {
    let [rule_row, card] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);

    // The solid separator: edge to edge, joined into the box's side borders.
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(rule_row.width as usize),
            dock.border,
        ))),
        rule_row,
    );
    if rule_row.x > 0 {
        for (x, ch) in [(rule_row.x - 1, "├"), (rule_row.x + rule_row.width, "┤")] {
            frame.render_widget(
                Paragraph::new(Span::styled(ch, dock.border)),
                Rect {
                    x,
                    y: rule_row.y,
                    width: 1,
                    height: 1,
                },
            );
        }
    }

    // The grey card wash under everything below the rule.
    frame.render_widget(Block::new().style(theme.dock_card()), card);
    let padded = Rect {
        x: card.x + 1,
        width: card.width.saturating_sub(2),
        ..card
    };
    let [head_row, _pad, facts] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(padded);

    // The heading, inside the card: title spans left, the hint pushed to the
    // right edge with a computed spacer (one Paragraph, so nothing overdraws).
    let mut head = dock.title.clone();
    if let Some(right) = dock.right {
        let used: usize = head.iter().map(|s| s.content.chars().count()).sum();
        let spacer = (head_row.width as usize).saturating_sub(used + right.chars().count());
        head.push(Span::raw(" ".repeat(spacer)));
        head.push(Span::styled(right, theme.dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(head)), head_row);

    // The two fact columns, each sized to its content, split by a floating
    // `│` divider that spans only the fact rows (never touching the rule).
    let left_w = dock
        .left
        .iter()
        .map(|l| l.width() as u16)
        .max()
        .unwrap_or(0);
    let [lcol, div, rcol] = Layout::horizontal([
        Constraint::Length(left_w),
        Constraint::Length(DOCK_COL_GAP as u16),
        Constraint::Min(0),
    ])
    .areas(facts);
    frame.render_widget(Paragraph::new(Text::from(dock.left.clone())), lcol);
    if !dock.right_col.is_empty() {
        let rows = dock
            .left
            .len()
            .max(dock.right_col.len())
            .min(div.height as usize);
        let bar: Vec<Line> = (0..rows)
            .map(|_| Line::from(Span::styled(" │ ", theme.dim())))
            .collect();
        frame.render_widget(Paragraph::new(Text::from(bar)), div);
    }
    frame.render_widget(Paragraph::new(Text::from(dock.right_col.clone())), rcol);
}

/// Map an attention [`Severity`] to its accent style for the dock's reason chips.
pub(crate) fn severity_style(sev: Severity, theme: &Theme) -> Style {
    match sev {
        Severity::Ok => theme.ok(),
        Severity::Info => theme.dim(),
        Severity::Notice => theme.ahead(),
        Severity::Warn => theme.warn(),
        Severity::Risk => theme.risk(),
    }
}

/// The width tier that decides a row's column set, so the width arithmetic, the
/// header labels, and the row cells all agree.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Tier {
    /// Repo · Sync · Last · Status (Sync fuses arrows + PR + CI as `↑2 ● 2pr`).
    Narrow,
    /// Repo · Sync · PRs · CI · Last · Status (split remote columns, no sparkline).
    Medium,
    /// Repo · Sync · PRs · CI · Last · Activity · Status.
    Wide,
}

/// How to render a row's columns, chosen by the terminal's width [`Tier`].
pub(crate) struct RowFmt {
    layout: Tier,
    /// Max chars for the reason label (the trailing "Status" column).
    reason_max: usize,
    /// Width of the age column, for right-aligning the age (the dock layout).
    age_w: u16,
    /// Blink phase for the "synced" dot: on this frame the green `●` shows; off,
    /// it's blank. Toggled from the animation tick so the dot blinks (terminals
    /// ignore the ANSI blink attribute, so we animate it ourselves).
    blink_on: bool,
}

pub(crate) fn repo_row<'a>(
    snap: &'a RepoSnapshot,
    highlights: &[u32],
    now: i64,
    theme: &Theme,
    busy: Option<&str>,
    marked: bool,
    fmt: &RowFmt,
) -> Row<'a> {
    if let Some(reason) = &snap.error {
        // A broken repo: a red name, the reason in the wide trailing Status column.
        // The data columns are genuinely unknowable, so they're left blank (a
        // single space keeps the cell non-empty so the table doesn't collapse). The
        // leading "  " keeps the name aligned with the selection gutter.
        let blank = || Cell::from(Span::raw(" "));
        let name = Cell::from(Line::from(vec![
            Span::raw("  "),
            Span::styled(snap.name.clone(), theme.error()),
        ]));
        let status = Cell::from(Span::styled(
            ellipsize(reason, fmt.reason_max),
            theme.risk(),
        ));
        return match fmt.layout {
            Tier::Wide => Row::new(vec![
                name,
                blank(), // Sync
                blank(), // PRs
                blank(), // CI
                blank(), // Last
                blank(), // Activity
                status,
            ]),
            Tier::Medium => Row::new(vec![
                name,
                blank(), // Sync
                blank(), // PRs
                blank(), // CI
                blank(), // Last
                status,
            ]),
            Tier::Narrow => Row::new(vec![
                name,
                blank(), // Sync
                blank(), // CI
                blank(), // Last
                status,
            ]),
        };
    }

    // A repo with several local branches lists each on its own indented line
    // under the repo (display-only: selection, Enter, and actions stay
    // repo-level, so `app.selected` still indexes repos).
    if snap.branches.len() > 1 {
        return multi_branch_row(snap, highlights, now, theme, marked, fmt);
    }

    let assessment = assess(snap, now);
    let severity = assessment.severity;
    let name = name_cell(
        &snap.name,
        highlights,
        severity,
        marked,
        &snap.branch,
        theme,
    );

    // The "synced" state (even with upstream): in the glyph tiers it's a blinking
    // green dot (blank on the off phase); under ASCII it's a steady "ok" word that
    // never blinks (so the state never depends on an animation). `blink_synced()`
    // gates the blanking accordingly.
    let synced = matches!(&snap.upstream, Some(u) if u.ahead == 0 && u.behind == 0);
    let blink_synced = theme.glyphs.blink_synced();

    let age = age_cell(snap, now, fmt.age_w as usize, theme);
    let status = reason_cell(assessment.primary.as_ref(), severity, fmt.reason_max, theme);
    match fmt.layout {
        Tier::Wide | Tier::Medium => {
            // Split remote columns. While an action or a rescan runs, every cell
            // being refreshed — Sync, PRs, and CI — shows the spinner instead of
            // a value about to change under the reader.
            let spin_cell = |spin: &str| Cell::from(Span::styled(spin.to_string(), theme.ahead()));
            let sync = match busy {
                Some(spin) => spin_cell(spin),
                None if synced && blink_synced && !fmt.blink_on => Cell::from(Span::raw(" ")),
                None => Cell::from(Line::from(ahead_behind_spans(snap, theme))),
            };
            let prs = match busy {
                Some(spin) => spin_cell(spin),
                None => Cell::from(Line::from(prs_spans(snap, theme))),
            };
            let ci = match busy {
                Some(spin) => spin_cell(spin),
                None => Cell::from(Line::from(ci_spans(snap, theme))),
            };
            if fmt.layout == Tier::Wide {
                Row::new(vec![
                    name,
                    sync,
                    prs,
                    ci,
                    age,
                    activity_cell(snap, theme),
                    status,
                ])
            } else {
                Row::new(vec![name, sync, prs, ci, age, status])
            }
        }
        Tier::Narrow => {
            // Sync as plain arrows, with CI kept as a colour-coded square in its
            // own column (never fused away).
            let sync = match busy {
                Some(spin) => Cell::from(Span::styled(spin.to_string(), theme.ahead())),
                None if synced && blink_synced && !fmt.blink_on => Cell::from(Span::raw(" ")),
                None => Cell::from(Line::from(ahead_behind_spans(snap, theme))),
            };
            let ci = match busy {
                Some(spin) => Cell::from(Span::styled(spin.to_string(), theme.ahead())),
                None => Cell::from(Line::from(ci_square_spans(snap, theme))),
            };
            Row::new(vec![name, sync, ci, age, status])
        }
    }
}

/// A repo with several local branches. The header line is the repo's HEAD row —
/// bold `name @current`, the checked-out branch's own sync / PR-count / last,
/// plus repo-level CI, activity, and status — so nothing on it is blank. Each
/// OTHER local branch then follows on its own indented line with its sync, open
/// PRs (targeting it as base), and last commit. Display-only: still one
/// selectable repo row.
fn multi_branch_row<'a>(
    snap: &'a RepoSnapshot,
    highlights: &[u32],
    now: i64,
    theme: &Theme,
    marked: bool,
    fmt: &RowFmt,
) -> Row<'a> {
    let assessment = assess(snap, now);
    let severity = assessment.severity;
    let age_w = fmt.age_w as usize;

    // The header carries the checked-out branch (its sync + last come from HEAD);
    // the remaining local branches each get their own indented line below.
    let current = snap.branches.iter().find(|b| b.is_current);
    let others: Vec<&cohors_core::BranchInfo> =
        snap.branches.iter().filter(|b| !b.is_current).collect();
    let extra = others.len();

    // Repo cell: bold `name @current`, then one indented `@branch` per other.
    let mut header_name = name_spans(&snap.name, highlights, severity, marked, theme, true);
    let bstyle = match &snap.branch {
        Branch::Detached(_) => theme.detached(),
        _ => theme.dim(),
    };
    header_name.push(Span::styled(branch_suffix(&snap.branch), bstyle));
    let mut name_lines = vec![Line::from(header_name)];

    // The header carries the checked-out branch's own signals — PRs targeting it,
    // its CI (keyed by ref, so it's the branch's real state, not the default's).
    // The repo total / default CI is the fallback for a detached HEAD with no
    // current branch. Each *other* branch fills its own line below.
    let (header_prs, header_ci, header_ci_sq) = match current {
        Some(bi) => (
            branch_prs_spans(snap, &bi.name, theme),
            branch_ci_spans(snap, bi, theme),
            branch_ci_square_spans(snap, bi, theme),
        ),
        None => (
            prs_spans(snap, theme),
            ci_spans(snap, theme),
            ci_square_spans(snap, theme),
        ),
    };
    let mut sync_lines = vec![Line::from(ahead_behind_spans(snap, theme))];
    let mut prs_lines = vec![Line::from(header_prs)];
    let mut age_lines = vec![age_line(snap.last_commit.as_ref(), now, age_w, theme)];
    let mut ci_lines = vec![Line::from(header_ci)];
    let mut ci_sq_lines = vec![Line::from(header_ci_sq)];
    // The header Status is the repo's compounded attention reason (worktree + sync
    // + stash of the checkout); each branch below shows its own sync state — the
    // only status that exists off the working tree.
    let mut status_lines = vec![Line::from(reason_spans(
        assessment.primary.as_ref(),
        severity,
        fmt.reason_max,
        theme,
    ))];
    for &bi in &others {
        name_lines.push(branch_name_line(bi, theme));
        sync_lines.push(Line::from(sync_spans(
            bi.has_upstream,
            bi.ahead,
            bi.behind,
            theme,
        )));
        prs_lines.push(Line::from(branch_prs_spans(snap, &bi.name, theme)));
        age_lines.push(age_line(bi.last_commit.as_ref(), now, age_w, theme));
        ci_lines.push(Line::from(branch_ci_spans(snap, bi, theme)));
        ci_sq_lines.push(Line::from(branch_ci_square_spans(snap, bi, theme)));
        status_lines.push(Line::from(branch_status_spans(bi, theme)));
    }
    let name = Cell::from(Text::from(name_lines));
    let sync = Cell::from(Text::from(sync_lines));
    let prs = Cell::from(Text::from(prs_lines));
    let age = Cell::from(Text::from(age_lines));
    let ci = Cell::from(Text::from(ci_lines));
    let ci_sq = Cell::from(Text::from(ci_sq_lines));
    let status = Cell::from(Text::from(status_lines));

    // Activity is a repo-level sparkline (per-branch history is a heavier fetch,
    // deferred); it rides the header line, blank on the branch lines below.
    let activity = header_cell(vec![activity_span(snap, theme)], extra);

    let height = 1 + extra as u16;
    match fmt.layout {
        Tier::Wide => Row::new(vec![name, sync, prs, ci, age, activity, status]).height(height),
        Tier::Medium => Row::new(vec![name, sync, prs, ci, age, status]).height(height),
        Tier::Narrow => Row::new(vec![name, sync, ci_sq, age, status]).height(height),
    }
}

/// The height (lines) a repo's row occupies: 1, or 1 header + one line per local
/// branch when it has several.
fn row_height(snap: &RepoSnapshot) -> usize {
    if snap.branches.len() > 1 {
        1 + snap.branches.len()
    } else {
        1
    }
}

/// Height-aware viewport: pick the first-visible repo index so the selected repo
/// fits within `avail_lines`, and count how many repos fit from there. Mirrors
/// `keep_visible` but in LINES, since a multi-branch repo spans several rows.
/// Returns `(offset, visible_repo_count)`.
fn window(prev: usize, selected: usize, avail_lines: usize, heights: &[usize]) -> (usize, usize) {
    let n = heights.len();
    if n == 0 {
        return (0, 0);
    }
    let selected = selected.min(n - 1);
    // Never scroll past the selected repo, then scroll down until it fits.
    let mut offset = prev.min(selected);
    while offset < selected && heights[offset..=selected].iter().sum::<usize>() > avail_lines {
        offset += 1;
    }
    // Count repos that fit from `offset` (always at least one, even if a single
    // tall repo overflows the viewport on its own).
    let mut used = 0usize;
    let mut count = 0usize;
    for &h in &heights[offset..] {
        if count > 0 && used + h > avail_lines {
            break;
        }
        used += h;
        count += 1;
    }
    (offset, count)
}

/// A cell whose first line carries `spans`, followed by `extra` blank lines — a
/// repo-level signal on the header line, with the branch lines below left empty.
fn header_cell(spans: Vec<Span<'static>>, extra: usize) -> Cell<'static> {
    let mut lines = vec![Line::from(spans)];
    lines.extend(std::iter::repeat_with(Line::default).take(extra));
    Cell::from(Text::from(lines))
}

/// One indented `@branch` line under a repo header, dim (matching the `@branch`
/// suffix style). Only the *other* branches list here — the checked-out one rides
/// the bold header line.
fn branch_name_line(bi: &cohors_core::BranchInfo, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw("    "),
        Span::styled(format!("@{}", bi.name), theme.dim()),
    ])
}

/// The open-PR count targeting one branch (as base): a dim "0" when none, the
/// count in accent when there are some, "local" when the repo isn't on a remote.
fn branch_prs_spans(snap: &RepoSnapshot, branch: &str, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.remote {
        None => vec![Span::styled("local", theme.dim())],
        Some(r) => match r.open_prs_by_base.get(branch).copied().unwrap_or(0) {
            0 => vec![Span::styled("0", theme.dim())],
            k => vec![Span::styled(k.to_string(), theme.ahead())],
        },
    }
}

/// One branch's last-commit age, right-aligned like the single-line Last column.
fn age_line(
    last: Option<&cohors_core::CommitMeta>,
    now: i64,
    age_w: usize,
    theme: &Theme,
) -> Line<'static> {
    let age = last
        .map(|c| time::relative(c.timestamp, now))
        .unwrap_or_else(|| "—".to_string());
    Line::from(Span::styled(format!("{age:>age_w$}"), theme.dim()))
}

/// How many weeks of history the Activity sparkline shows (the width of the
/// column, one cell per week) — matches the web's 12-week window.
pub(crate) const ACTIVITY_WEEKS: usize = 12;

/// The "Activity" column: the repo's weekly commit counts as a 12-cell
/// sparkline (oldest first), the same cadence signal the web row shows. Dim on
/// purpose — it's context, not a call to action. A repo with no commits in the
/// window says "quiet" instead of drawing a flat baseline, which reads as
/// data, not as an empty cell.
pub(crate) fn activity_cell<'a>(snap: &RepoSnapshot, theme: &Theme) -> Cell<'a> {
    Cell::from(activity_span(snap, theme))
}

/// The activity sparkline as a single styled span (a repo-level signal), reused
/// on the header line of a multi-branch row.
fn activity_span(snap: &RepoSnapshot, theme: &Theme) -> Span<'static> {
    let max = snap.activity.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return Span::styled(format!("{:^ACTIVITY_WEEKS$}", "quiet"), theme.dim());
    }
    let ramp = theme.glyphs.spark_ramp();
    let mut spark = String::with_capacity(ACTIVITY_WEEKS * 3);
    for w in 0..ACTIVITY_WEEKS {
        let n = snap.activity.get(w).copied().unwrap_or(0) as usize;
        spark.push(ramp[(n * 7).div_ceil(max as usize).min(7)]);
    }
    Span::styled(spark, theme.dim())
}

/// The "Last" column: the commit age alone, right-aligned (the message lives in
/// the dock when this layout is active).
pub(crate) fn age_cell<'a>(snap: &RepoSnapshot, now: i64, age_w: usize, theme: &Theme) -> Cell<'a> {
    Cell::from(age_line(snap.last_commit.as_ref(), now, age_w, theme))
}

/// The "Status" column: the repo's primary attention reason as text, colored by
/// the repo's *overall* severity (which compounds — a repo can escalate past its
/// single loudest reason). Using the overall severity keeps the Status color in
/// step with the row's weight and the Attention summary, instead of reflecting
/// only the primary reason. A dim "up to date" when the repo wants nothing.
pub(crate) fn reason_cell<'a>(
    primary: Option<&AttentionReason>,
    severity: Severity,
    max: usize,
    theme: &Theme,
) -> Cell<'a> {
    Cell::from(Line::from(reason_spans(primary, severity, max, theme)))
}

/// The Status text as spans (primary attention reason, coloured by overall
/// severity; a dim "up to date" when nothing's wrong) — shared by the row cell
/// and a multi-branch repo's header line.
fn reason_spans(
    primary: Option<&AttentionReason>,
    severity: Severity,
    max: usize,
    theme: &Theme,
) -> Vec<Span<'static>> {
    match primary {
        Some(r) => vec![Span::styled(
            ellipsize(&r.label(), max),
            severity_style(severity, theme),
        )],
        None => vec![Span::styled(ellipsize("up to date", max), theme.dim())],
    }
}

/// The "Sync" column (dock layout): ahead/behind arrows — `↑2 ↓5`, `↑2`, `↓5` — a
/// blinking green `●` when in sync with upstream, "local" when there's no
/// upstream. The remote PR/CI signal moved to its own columns.
pub(crate) fn ahead_behind_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.upstream {
        None => sync_spans(false, 0, 0, theme),
        Some(up) => sync_spans(true, up.ahead, up.behind, theme),
    }
}

/// Ahead/behind arrows for a single branch vs its own upstream: "local" (no
/// upstream), a synced dot (0/0), or `↑a ↓b`. Shared by the HEAD row and the
/// per-branch lines of a multi-branch repo.
fn sync_spans(has_upstream: bool, ahead: u32, behind: u32, theme: &Theme) -> Vec<Span<'static>> {
    if !has_upstream {
        return vec![Span::styled("local", theme.dim())];
    }
    if ahead == 0 && behind == 0 {
        return vec![Span::styled(theme.glyphs.synced(), theme.ok())];
    }
    let mut spans = Vec::new();
    if ahead > 0 {
        spans.push(Span::styled(format!("↑{ahead}"), theme.ahead()));
    }
    if ahead > 0 && behind > 0 {
        spans.push(Span::raw(" "));
    }
    if behind > 0 {
        spans.push(Span::styled(format!("↓{behind}"), theme.behind()));
    }
    spans
}

/// The "PRs" column (dock layout): open pull-request count — a dim "0" on a remote
/// with none, "local" when the repo isn't on a remote.
pub(crate) fn prs_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.remote {
        None => vec![Span::styled("local", theme.dim())],
        Some(r) if r.open_prs == 0 => vec![Span::styled("0", theme.dim())],
        Some(r) => vec![Span::styled(r.open_prs.to_string(), theme.ahead())],
    }
}

/// A CI status as a coloured word — `passing`/`failing`/`pending`/"no CI". Shared
/// by the repo CI cell and each per-branch CI line.
fn ci_label_spans(ci: CiStatus, theme: &Theme) -> Vec<Span<'static>> {
    let (label, style) = match ci {
        CiStatus::Passing => ("passing", theme.ok()),
        CiStatus::Failing => ("failing", theme.risk()),
        CiStatus::Pending => ("pending", theme.warn()),
        CiStatus::None => ("no CI", theme.dim()),
    };
    vec![Span::styled(label, style)]
}

/// The colour a CI status reads in, shared by the word and compact-square cells.
fn ci_style(ci: CiStatus, theme: &Theme) -> Style {
    match ci {
        CiStatus::Passing => theme.ok(),
        CiStatus::Failing => theme.risk(),
        CiStatus::Pending => theme.warn(),
        CiStatus::None => theme.dim(),
    }
}

/// The "CI" column (dock layout): the check status spelled out and colored;
/// "local" when the repo isn't on a remote, "no CI" when it is but has no checks.
pub(crate) fn ci_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    match &snap.remote {
        None => vec![Span::styled("local", theme.dim())],
        Some(r) => ci_label_spans(r.ci, theme),
    }
}

/// The CI status for one branch of a multi-branch repo. "local" when the repo has
/// no remote or the branch was never pushed (nothing was ever built for it); else
/// its *own* checks from `ci_by_branch` — "no CI" when the map has no entry for it
/// (e.g. a branch past the per-repo fetch cap). This is why the passing badge sits
/// on the branch that was actually built, not on whichever branch is checked out.
fn branch_ci_spans(
    snap: &RepoSnapshot,
    branch: &cohors_core::BranchInfo,
    theme: &Theme,
) -> Vec<Span<'static>> {
    match &snap.remote {
        Some(r) if branch.has_upstream => ci_label_spans(
            r.ci_by_branch
                .get(&branch.name)
                .copied()
                .unwrap_or(CiStatus::None),
            theme,
        ),
        _ => vec![Span::styled("local", theme.dim())],
    }
}

/// Per-branch Status, derived from sync state alone: a branch that isn't the
/// checkout has no working tree, so there's no dirty/stash signal — only its
/// relationship to its upstream. "never pushed", "N unpushed", "N behind",
/// "N ahead · M behind", or a dim "up to date"; coloured to match the sync arrows.
fn branch_status_spans(bi: &cohors_core::BranchInfo, theme: &Theme) -> Vec<Span<'static>> {
    if !bi.has_upstream {
        return vec![Span::styled("never pushed", theme.warn())];
    }
    match (bi.ahead, bi.behind) {
        (0, 0) => vec![Span::styled("up to date", theme.dim())],
        (a, 0) => vec![Span::styled(format!("{a} unpushed"), theme.ahead())],
        (0, b) => vec![Span::styled(format!("{b} behind"), theme.behind())],
        (a, b) => vec![Span::styled(
            format!("{a} ahead · {b} behind"),
            theme.warn(),
        )],
    }
}

/// The compact CI cell (narrow layout): a colour-coded square standing in for
/// the word — green passing, red failing, amber pending, grey for no CI or no
/// remote. So CI never simply vanishes when the terminal is too narrow for
/// "passing"; it shrinks to a square the colour still reads on.
pub(crate) fn ci_square_spans(snap: &RepoSnapshot, theme: &Theme) -> Vec<Span<'static>> {
    let ci = snap.remote.as_ref().map_or(CiStatus::None, |r| r.ci);
    vec![Span::styled(
        theme.glyphs.ci_square(ci),
        ci_style(ci, theme),
    )]
}

/// The compact CI square for one branch (narrow layout): a grey square when the
/// branch has no remote checks (no remote, or never pushed), else its own colour.
fn branch_ci_square_spans(
    snap: &RepoSnapshot,
    branch: &cohors_core::BranchInfo,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let ci = match &snap.remote {
        Some(r) if branch.has_upstream => r
            .ci_by_branch
            .get(&branch.name)
            .copied()
            .unwrap_or(CiStatus::None),
        _ => CiStatus::None,
    };
    vec![Span::styled(
        theme.glyphs.ci_square(ci),
        ci_style(ci, theme),
    )]
}

/// The repo name, preceded by a selection gutter: a cyan `●` when the repo is
/// marked for a bulk action, two spaces otherwise (so names stay aligned). The
/// name is dimmed when clean, bold in a warn/risk state; fuzzy-matched
/// characters are bold-highlighted.
pub(crate) fn name_cell<'a>(
    name: &str,
    highlights: &[u32],
    severity: Severity,
    marked: bool,
    branch: &Branch,
    theme: &Theme,
) -> Cell<'a> {
    let mut spans = name_spans(name, highlights, severity, marked, theme, false);
    // The branch is fused onto the name (`name @branch`), mirroring the web — a
    // dim suffix (or magenta for a detached `@sha`), so Repo + Branch share one
    // tight column. (Multi-branch repos list branches on their own lines instead.)
    let bstyle = match branch {
        Branch::Detached(_) => theme.detached(),
        _ => theme.dim(),
    };
    spans.push(Span::styled(branch_suffix(branch), bstyle));
    Cell::from(Line::from(spans))
}

/// The selection gutter + styled repo name (fuzzy highlights bolded), WITHOUT the
/// `@branch` suffix — shared by the single-line row and a multi-branch repo's
/// header line.
fn name_spans(
    name: &str,
    highlights: &[u32],
    severity: Severity,
    marked: bool,
    theme: &Theme,
    force_bold: bool,
) -> Vec<Span<'static>> {
    // Red is reserved for genuinely broken repos (the `snap.error` row); a repo
    // that merely wants attention (aging unpushed, behind, dirty) is conveyed by
    // weight here and by the coloured Status cell — never an alarming red name.
    // A multi-branch repo's header always bolds (`force_bold`) so the repo name
    // stands out above its dimmer branch lines.
    let base = if force_bold {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        match severity {
            Severity::Ok | Severity::Info => theme.dim(),
            Severity::Warn | Severity::Risk => Style::new().add_modifier(Modifier::BOLD),
            Severity::Notice => Style::new(),
        }
    };
    // Checkbox gutter: an empty circle that fills brand-purple when marked.
    let gutter = if marked {
        Span::styled(
            format!("{} ", theme.glyphs.marked()),
            theme.fg(SPIDER_PURPLE),
        )
    } else {
        Span::styled(format!("{} ", theme.glyphs.unmarked()), theme.dim())
    };
    let mut spans = vec![gutter];
    if highlights.is_empty() {
        spans.push(Span::styled(name.to_string(), base));
    } else {
        // Bold the fuzzy-matched characters.
        let hl = base.patch(theme.highlight());
        spans.extend(name.chars().enumerate().map(|(i, ch)| {
            let style = if highlights.contains(&(i as u32)) {
                hl
            } else {
                base
            };
            Span::styled(ch.to_string(), style)
        }));
    }
    spans
}

/// The `@branch` suffix appended to the repo name: `@main`, `@a1b2c3d` for a
/// detached HEAD, or `unborn` for a fresh repo. The branch name is ellipsized so
/// one long branch can't blow out the fused Repo column.
pub(crate) fn branch_suffix(branch: &Branch) -> String {
    match branch {
        Branch::Named(b) => format!(" @{}", ellipsize(b, 18)),
        Branch::Detached(id) => format!(" @{}", id.chars().take(7).collect::<String>()),
        Branch::Unborn => " unborn".to_string(),
    }
}

/// The styled spans of the fused Repo cell, *without* fuzzy highlights — used only
/// to size the column to its widest content (see [`col_w`]).
pub(crate) fn repo_spans(snap: &RepoSnapshot, _theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::raw("  "),
        Span::raw(snap.name.clone()),
        Span::raw(branch_suffix(&snap.branch)),
    ]
}

/// The display width (in terminal columns) of a run of spans — all of cohors's
/// glyphs are single-width, so a `char` count is exact. Used to size data
/// columns to their actual content rather than a fixed guess.
pub(crate) fn line_width(spans: &[Span]) -> u16 {
    spans.iter().map(|s| s.content.chars().count() as u16).sum()
}

/// Truncate `s` to at most `max` characters, adding an ellipsis when cut.
pub(crate) fn ellipsize(s: &str, max: usize) -> String {
    let len = s.chars().count();
    if len <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}
