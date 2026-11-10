//! Small, pure text helpers the cards share: clipping, padding and wording.
//!
//! Widths are counted in `char`s. Every glyph cohors draws itself is one column
//! wide, so that is exact for the interface; a repository name with wide
//! characters only shifts its own row and can never cause a panic, because
//! nothing here indexes into a string by byte.

/// Drop control characters from text that came from a repository (a branch
/// name, a commit subject, a task title) before it reaches the terminal: an
/// escape sequence in a commit message must not be able to move the cursor.
pub(crate) fn safe(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Cut `text` to at most `width` columns, ending it with `ellipsis` when
/// something was removed.
pub(crate) fn clip(text: &str, width: usize, ellipsis: char) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push(ellipsis);
    out
}

/// Clip `text` to `width` columns, then pad it with spaces on the right so it
/// is exactly `width` wide: a left-aligned table cell.
pub(crate) fn cell(text: &str, width: usize, ellipsis: char) -> String {
    let clipped = clip(text, width, ellipsis);
    let used = clipped.chars().count();
    format!("{clipped}{}", " ".repeat(width.saturating_sub(used)))
}

/// A left-aligned table column `width` wide whose text always ends at least
/// one space before the next column, so a clipped name never runs into its
/// neighbour.
pub(crate) fn column(text: &str, width: usize, ellipsis: char) -> String {
    cell(
        &clip(text, width.saturating_sub(1), ellipsis),
        width,
        ellipsis,
    )
}

/// Like [`cell`], but right-aligned: the padding goes on the left. Numbers
/// line up this way.
pub(crate) fn cell_right(text: &str, width: usize, ellipsis: char) -> String {
    let clipped = clip(text, width, ellipsis);
    let used = clipped.chars().count();
    format!("{}{clipped}", " ".repeat(width.saturating_sub(used)))
}

/// Trim `text` to at most `width` columns, keeping the *end* (the most
/// specific part of a path) and marking the cut at the front.
pub(crate) fn clip_start(text: &str, width: usize, ellipsis: char) -> String {
    let len = text.chars().count();
    if len <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let tail: String = text.chars().skip(len - (width - 1)).collect();
    format!("{ellipsis}{tail}")
}

/// Make the first letter upper case: the overview's reasons are written to sit
/// inside a sentence, and a card starts each one on its own line.
pub(crate) fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `1 repository` / `3 repositories`: the count followed by the right noun.
pub(crate) fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Greedy word wrap: split `text` into lines of at most `width` columns. A
/// single word wider than the limit is left whole (the card clips it) rather
/// than broken in the middle.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let fits = current.chars().count() + 1 + word.chars().count() <= width;
        if current.is_empty() {
            current.push_str(word);
        } else if fits {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Replace a leading `$HOME` with `~` for a compact path display.
pub(crate) fn abbrev_home(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_marks_what_it_cut_and_never_overflows() {
        assert_eq!(clip("short", 10, '…'), "short");
        assert_eq!(clip("exactly-10", 10, '…'), "exactly-10");
        assert_eq!(clip("a long repository name", 10, '…'), "a long re…");
        assert_eq!(clip("anything", 1, '…'), "…");
        assert_eq!(clip("anything", 0, '…'), "");
        // Multi-byte text is cut on character boundaries, not bytes.
        assert_eq!(clip("日本語のリポジトリ", 4, '~'), "日本語~");
    }

    #[test]
    fn cells_are_exactly_as_wide_as_asked() {
        assert_eq!(cell("main", 8, '…'), "main    ");
        assert_eq!(cell_right("12", 5, '…'), "   12");
        assert_eq!(cell("feat/a-very-long-branch", 8, '…'), "feat/a-…");
        for width in 0..12 {
            assert_eq!(cell("repository", width, '…').chars().count(), width);
            assert_eq!(cell_right("repository", width, '…').chars().count(), width);
        }
    }

    #[test]
    fn a_column_always_leaves_a_gap_before_the_next_one() {
        assert_eq!(column("main", 8, '…'), "main    ");
        // Exactly as wide as the column: clipped, so a space still follows.
        assert_eq!(column("12345678", 8, '…'), "123456… ");
        assert_eq!(column("feat/a-very-long-branch", 8, '…'), "feat/a… ");
        for width in 0..12 {
            let text = column("repository", width, '…');
            assert_eq!(text.chars().count(), width);
            assert!(width == 0 || text.ends_with(' '));
        }
    }

    #[test]
    fn clip_start_keeps_the_end_of_a_path() {
        assert_eq!(clip_start("~/code/app", 20, '…'), "~/code/app");
        assert_eq!(clip_start("~/code/projects/app", 8, '…'), "…cts/app");
        assert_eq!(clip_start("anything", 0, '…'), "");
    }

    #[test]
    fn control_characters_never_reach_the_terminal() {
        assert_eq!(safe("fix: \u{1b}[31mred\u{7} bell"), "fix: [31mred bell");
        assert_eq!(safe("tab\tand\nnewline"), "tabandnewline");
    }

    #[test]
    fn wording_helpers() {
        assert_eq!(capitalize("branch never pushed"), "Branch never pushed");
        assert_eq!(capitalize("9 uncommitted files"), "9 uncommitted files");
        assert_eq!(capitalize(""), "");
        assert_eq!(count(1, "repository", "repositories"), "1 repository");
        assert_eq!(count(0, "repository", "repositories"), "0 repositories");
    }

    #[test]
    fn wrap_breaks_between_words() {
        assert_eq!(
            wrap("Tasks appear once a repository has a launchpad file", 20),
            ["Tasks appear once a", "repository has a", "launchpad file"]
        );
        assert_eq!(wrap("", 10), Vec::<String>::new());
        // A word longer than the line is kept whole.
        assert_eq!(
            wrap("a .cohors/repo.json b", 6),
            ["a", ".cohors/repo.json", "b"]
        );
    }
}
