#!/usr/bin/env python3
"""Build every table in the README as a designed SVG, in the "pixel" style.

    python3 scripts/readme-art.py

GitHub strips styling from Markdown and HTML, so a designed table has to be an
image. The words live in the tables below: change them and run this again.

The style borrows from the logo's pixel creature: square corners, stepped
"pixel" notches, ticks and dots drawn as pixel blocks, and each table's lead
column as a solid plum bar. The colours are the logo's: plum, orchid and the
lavender of its eyes, with a deep aubergine for dark backgrounds.

Each table is written three times into assets/readme/:

  <name>-light.svg, <name>-dark.svg  GitHub's <picture> picks one by the
                                     reader's GitHub theme.
  <name>.svg                         Carries both colour sets and follows the
                                     reader's system theme, for sites that
                                     ignore <picture> (npm, crates.io).

Text uses the reader's system fonts, and widths leave room for differences.
"""

from pathlib import Path
from xml.sax.saxutils import escape

OUT = Path(__file__).resolve().parent.parent / "assets" / "readme"
W = 1000
PX = 6  # one "pixel" of the style

FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif"
MONO = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace"

# (light, dark)
PALETTE = {
    "panel": ("#FBF7FA", "#1A1219"),  # the table's ground
    "band": ("#F3E8F1", "#2A1A27"),   # every other row
    "bar": ("#954688", "#B55CA4"),    # the lead column: plum / orchid
    "on-bar": ("#FFFFFF", "#FFFFFF"),
    "accent": ("#954688", "#D9A6CF"), # code and ticks outside the bar
    "ink": ("#2B1A28", "#F2E9F0"),
    "sub": ("#6F5A6B", "#A99AA6"),
    "off": ("#CDBFCA", "#5A4A57"),    # the "no" dot
    "cap": ("#FFFFFF", "#2A1A27"),    # a key cap
    "cap-edge": ("#D9C4D5", "#5A4A57"),
    # The terminal window stays dark in both themes, like a real terminal.
    "term": ("#1A1219", "#120C11"),
    "term-bar": ("#2A1A27", "#221520"),
    "term-ink": ("#F2E9F0", "#F2E9F0"),
    "term-sub": ("#A99AA6", "#A99AA6"),
    "term-prompt": ("#D9A6CF", "#D9A6CF"),
    "lavender": ("#C98AC0", "#C98AC0"),
    "orchid": ("#B55CA4", "#B55CA4"),
}
# The dashboard's section colours (crates/cohors-tui/src/ui/mod.rs), used as
# small pixel markers on the answer cards: (light, dark).
SECTIONS = {
    "do-next": ("#8E3F7E", "#B55CA4"),
    "risk": ("#B4483F", "#D9776E"),
    "tasks": ("#4D8A62", "#7FB08C"),
    "commits": ("#4A6A94", "#7E9CC2"),
    "repos": ("#2F7F78", "#5FAEA6"),
}


def css(i):
    rules = []
    for name, pair in PALETTE.items():
        rules.append(f".f-{name}{{fill:{pair[i]}}}.s-{name}{{stroke:{pair[i]}}}")
    for name, pair in SECTIONS.items():
        rules.append(f".f-{name}{{fill:{pair[i]}}}")
    return "".join(rules)


STYLES = {
    "light": css(0),
    "dark": css(1),
    "auto": css(0) + "@media (prefers-color-scheme: dark){" + css(1) + "}",
}


def svg(height, title, desc, body, theme):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{height}" '
        f'viewBox="0 0 {W} {height}" role="img" aria-labelledby="t d">'
        f'<title id="t">{escape(title)}</title><desc id="d">{escape(desc)}</desc>'
        f"<style>{STYLES[theme]}</style>{body}</svg>\n"
    )


def text(x, y, body, size, fill, weight=400, anchor="start", family=FONT):
    return (
        f'<text x="{x}" y="{y}" font-family="{family}" font-size="{size}" font-weight="{weight}" '
        f'class="f-{fill}" text-anchor="{anchor}">{escape(body)}</text>'
    )


def notched(x, y, w, h, fill, step=PX):
    """A rectangle with one-pixel steps cut from each corner, like the logo."""
    s = step
    d = (f"M{x + s} {y}h{w - 2 * s}v{s}h{s}v{h - 2 * s}h{-s}v{s}h{-(w - 2 * s)}"
         f"v{-s}h{-s}v{-(h - 2 * s)}h{s}z")
    return f'<path d="{d}" class="f-{fill}"/>'


def tick(cx, cy, fill):
    """A tick made of four pixel blocks."""
    return "".join(
        f'<rect x="{cx + dx - 3}" y="{cy + dy - 3}" width="6" height="6" class="f-{fill}"/>'
        for dx, dy in ((-9, 0), (-3, 6), (3, 0), (9, -6))
    )


def dot(cx, cy, fill):
    return f'<rect x="{cx - 3}" y="{cy - 3}" width="6" height="6" class="f-{fill}"/>'


def rich(x, y, segments, size, on_bar=False):
    """One line of mixed text: (kind, words), where kind is "t" text, "b" bold,
    "m" muted, "i" italic or "c" code. Runs flow as tspans, so no widths are
    guessed; xml:space keeps the spaces between runs."""
    spans = []
    for kind, words in segments:
        if on_bar:
            fill = "on-bar"
        else:
            fill = {"t": "ink", "b": "ink", "i": "ink", "m": "sub", "c": "accent"}[kind]
        attrs = f'class="f-{fill}"'
        if kind == "b":
            attrs += ' font-weight="700"'
        if kind == "i":
            attrs += ' font-style="italic"'
        if kind == "c":
            attrs += f' font-family="{MONO}" font-size="{size - 1}"'
        spans.append(f"<tspan {attrs}>{escape(words)}</tspan>")
    return (
        f'<text x="{x}" y="{y}" font-family="{FONT}" font-size="{size}" '
        f'xml:space="preserve">{"".join(spans)}</text>'
    )


def keycaps(x, y, keys):
    """Square key caps with a one-pixel shadow step. "–" is a plain dash."""
    out, cx = [], x
    for key in keys:
        if key == "–":
            out.append(text(cx + 3, y + 5, "–", 14, "sub", 600))
            cx += 18
            continue
        w = max(28, len(key) * 8.6 + 16)
        out.append(f'<rect x="{cx + 3}" y="{y - 11}" width="{w}" height="27" class="f-cap-edge"/>')
        out.append(f'<rect x="{cx}" y="{y - 14}" width="{w}" height="27" class="f-cap"/>')
        out.append(
            f'<text x="{cx + w / 2}" y="{y + 4}" font-family="{MONO}" font-size="14" font-weight="700" '
            f'class="f-accent" text-anchor="middle">{escape(key)}</text>'
        )
        cx += w + 10
    return "".join(out)


def plain(cell):
    if isinstance(cell, dict):
        return " ".join(cell["keys"])
    return "".join(words for _, words in cell)


# ── What it answers: one card per dashboard card ────────────────────────────

ANSWERS = [
    (1, "do-next", "Do next", "What should I do first?",
     "Next steps, most urgent first, each with the command that does it"),
    (2, "risk", "Work at risk", "What's about to be lost?",
     "Every repo at risk, why, and the step that fixes it"),
    (3, "tasks", "Open tasks", "What's waiting in each repo?",
     "Open tasks and the commands worth running"),
    (4, "commits", "Commits, last 7 days", "What did I ship this week?",
     "Your commits across every repo, ready for standup"),
    (5, "repos", "Repositories", "What exists only on this laptop?",
     "Commits never pushed and branches never published, by repo"),
]


def answers(theme):
    row = 80
    h = len(ANSWERS) * row
    b = [f'<rect width="{W}" height="{h}" class="f-panel"/>']
    for n, (key, section, card, question, answer) in enumerate(ANSWERS):
        y = n * row
        if n % 2 == 0:
            b.append(f'<rect x="0" y="{y}" width="{W}" height="{row}" class="f-band"/>')
        # The dashboard key that jumps to this card, as a plum pixel block.
        b.append(notched(24, y + row / 2 - 22, 44, 44, "bar"))
        b.append(text(46, y + row / 2 + 7, str(key), 20, "on-bar", 800, "middle", MONO))
        b.append(text(92, y + 34, question, 18, "ink", 700))
        b.append(text(92, y + 58, answer, 15, "sub"))
        # Which dashboard card answers it, marked in that card's colour.
        label_w = len(card) * 8.4
        lx = W - 28 - label_w
        b.append(f'<rect x="{lx - 18}" y="{y + row / 2 - 4}" width="8" height="8" class="f-{section}"/>')
        b.append(text(lx, y + row / 2 + 5, card, 14, "sub", 600, family=MONO))
    desc = "; ".join(f"{q} {a} (the {c} card, key {k})" for k, _, c, q, a in ANSWERS)
    return svg(h, "What cohors answers", desc, "".join(b), theme)


# ── How it compares ─────────────────────────────────────────────────────────

TOOLS = ["cohors", "git-scope", "mani / gita", "lazygit / gitui"]
COMPARE = [
    ("Every repo at once", [True, True, "text only", None]),
    ("Bulk fetch, pull, push, run", [True, "read-only", True, None]),
    ("Pull requests and CI", [True, None, None, "one repo"]),
    ("Cross-repo search and standup", [True, None, None, None]),
    ("MCP server for coding agents", [True, None, None, None]),
]


def compare(theme):
    label, head, row = 330, 58, 54
    cw = (W - label) / len(TOOLS)
    h = head + len(COMPARE) * row + PX
    b = [f'<rect width="{W}" height="{h}" class="f-panel"/>']
    for r in range(len(COMPARE)):
        if r % 2 == 0:
            b.append(f'<rect x="0" y="{head + r * row}" width="{W}" height="{row}" class="f-band"/>')
    b.append(notched(label + 8, 0, cw - 16, h, "bar"))
    for i, tool in enumerate(TOOLS):
        cx = label + cw * i + cw / 2
        own = i == 0
        b.append(text(cx, 36, tool, 16 if own else 14, "on-bar" if own else "sub",
                      800 if own else 700, "middle", MONO))
    for r, (feature, marks) in enumerate(COMPARE):
        mid = head + r * row + row / 2
        b.append(text(26, mid + 5, feature, 15, "ink", 500))
        for i, mark in enumerate(marks):
            cx = label + cw * i + cw / 2
            if mark is True:
                b.append(tick(cx, mid, "on-bar" if i == 0 else "accent"))
            elif mark is None:
                b.append(dot(cx, mid, "off"))
            else:
                b.append(text(cx, mid + 5, mark, 13, "sub", 500, "middle", MONO))
    desc = "; ".join(
        f"{feature}: " + ", ".join(
            f"{tool} {'yes' if m is True else 'no' if m is None else m}" for tool, m in zip(TOOLS, marks)
        )
        for feature, marks in COMPARE
    )
    return svg(h, "How cohors compares", desc, "".join(b), theme)


# ── Keys: tiles laid out like a keyboard ─────────────────────────────────────

KEYS = [
    (["P", "p", "f"], "Push, pull, fetch", ""),
    (["c", "S"], "Commit, stash", "asks first"),
    (["F"], "Fetch everything", ""),
    (["1", "–", "5"], "Jump to a card", ""),
    ([":"], "Command line", "Tab completes"),
    (["Enter"], "Open a repo", ""),
    (["/"], "Filter as you type", ""),
    (["s", "Y"], "Standup, copy it", ""),
    (["t"], "Tasks across repos", ""),
    (["?"], "Every key", ""),
]


def caps_width(keys):
    total = 0
    for key in keys:
        total += 18 if key == "–" else max(28, len(key) * 8.6 + 16) + 10
    return total - 10


def keys(theme):
    per_row, gap, tile_h = 5, 12, 112
    tile_w = (W - gap * (per_row - 1)) / per_row
    rows = (len(KEYS) + per_row - 1) // per_row
    h = rows * tile_h + (rows - 1) * gap
    b = []
    for n, (caps, label, note) in enumerate(KEYS):
        x = (n % per_row) * (tile_w + gap)
        y = (n // per_row) * (tile_h + gap)
        b.append(notched(x, y, tile_w, tile_h, "band"))
        b.append(keycaps(x + tile_w / 2 - caps_width(caps) / 2, y + 36, caps))
        b.append(text(x + tile_w / 2, y + 76, label, 14, "ink", 600, "middle"))
        if note:
            b.append(text(x + tile_w / 2, y + 96, note, 12, "sub", 500, "middle", MONO))
    desc = "; ".join(f"{' '.join(c)}: {l}" + (f" ({n})" if n else "") for c, l, n in KEYS)
    return svg(h, "Dashboard keys", desc, "".join(b), theme)


# ── More to ask: chat bubbles pointing at the tool that answers ─────────────

ASK = [
    ("What have I changed in web-app?", [("c", "changes"), ("m", ", with the diff if you want it")]),
    ("Where do we handle retries?", [("c", "search"), ("m", ", across every repo at once")]),
    ("Write my standup for this week.", [("c", "standup"), ("m", ", your commits by repo")]),
    ("Back up everything only on this laptop.", [("c", "push"), ("m", ", as a dry run first")]),
    ("Which of my PRs are failing CI?", [("c", "ci_status"), ("m", " and "), ("c", "list_prs")]),
    ("Plan my day.", [("c", "plan-focus"), ("m", ", saved to your board")]),
]


def ask(theme):
    row, bubble_h = 60, 40
    h = len(ASK) * row - (row - bubble_h) + 4
    b = []
    for n, (said, answer) in enumerate(ASK):
        y = n * row
        mid = y + bubble_h / 2
        w = len(said) * 7.9 + 44
        # You: a lavender bubble with a pixel tail.
        b.append(notched(28, y, w, bubble_h, "band"))
        b.append(f'<rect x="16" y="{y + bubble_h - 12}" width="12" height="6" class="f-band"/>')
        b.append(f'<rect x="10" y="{y + bubble_h - 6}" width="6" height="6" class="f-band"/>')
        b.append(text(50, mid + 5, said, 15, "ink", 500))
        # Then the tool that answers, after a pixel arrow.
        ax = 560
        # A pixel pointer, ▶: columns of 5, 3 and 1 blocks.
        for dx, dy in ((0, -12), (0, -6), (0, 0), (0, 6), (0, 12), (6, -6), (6, 0), (6, 6), (12, 0)):
            b.append(f'<rect x="{ax + dx}" y="{mid - 3 + dy}" width="6" height="6" class="f-bar"/>')
        b.append(rich(ax + 34, mid + 5, answer, 15))
    desc = "; ".join(f"“{s}” uses {plain(a)}" for s, a in ASK)
    return svg(h, "More to ask Claude", desc, "".join(b), theme)


# ── Permissions: three cards, each with an "off" pixel switch ───────────────

PERMISSIONS = [
    ("Git actions",
     [[("c", "fetch  pull  push")], [("c", "commit  stash")]],
     [[("m", "No force push")], [("m", "Fast-forward-only pull")], [("c", "confirm"), ("m", " to commit or stash")]]),
    ("Run commands",
     [[("t", "One command across")], [("t", "many repos")]],
     [[("c", "confirm"), ("m", " before it runs")], [("m", "A cap on repos per call")], [("m", "An optional allowlist")]]),
    ("Launchpad writes",
     [[("t", "Each repo's "), ("c", ".cohors/")], [("t", "Your focus board")]],
     [[("m", "Validated before writing")], [("m", "Never touches git")], [("m", "Never runs a command")]]),
]


def switch(x, y):
    """An "off" switch drawn in pixels: an outlined track, the knob left."""
    return (
        f'<rect x="{x}" y="{y}" width="44" height="22" class="f-off"/>'
        f'<rect x="{x + 3}" y="{y + 3}" width="38" height="16" class="f-panel"/>'
        f'<rect x="{x + 6}" y="{y + 6}" width="10" height="10" class="f-off"/>'
    )


def permissions(theme):
    gap, card_h = 14, 268
    card_w = (W - gap * (len(PERMISSIONS) - 1)) / len(PERMISSIONS)
    b = []
    for n, (name, allows, guards) in enumerate(PERMISSIONS):
        x = n * (card_w + gap)
        b.append(notched(x, 0, card_w, card_h, "band"))
        b.append(text(x + 24, 40, name, 18, "ink", 800))
        b.append(switch(x + card_w - 24 - 44, 22))
        b.append(text(x + card_w - 24, 62, "off by default", 11.5, "sub", 600, "end", MONO))
        b.append(text(x + 24, 92, "allows", 12.5, "accent", 700, family=MONO))
        for i, line in enumerate(allows):
            b.append(rich(x + 24, 116 + i * 22, line, 15))
        b.append(text(x + 24, 180, "still guarded by", 12.5, "accent", 700, family=MONO))
        for i, line in enumerate(guards):
            b.append(f'<rect x="{x + 24}" y="{198 + i * 22}" width="6" height="6" class="f-bar"/>')
            b.append(rich(x + 40, 204 + i * 22, line, 14))
    desc = "; ".join(
        f"{name}, off by default: allows {' '.join(plain(l) for l in a)}; guarded by {', '.join(plain(l) for l in g)}"
        for name, a, g in PERMISSIONS
    )
    return svg(card_h, "What Claude may do", desc, "".join(b), theme)


# ── --select: filter chips ──────────────────────────────────────────────────

SELECT = [
    (["dirty", "ahead", "behind", "diverged"], "Repos in that state"),
    (["unpublished", "stale-fetch", "attention"], "No upstream, not fetched lately, needs a look"),
    (["name:pay*", "ci:failing"], "By name, or by CI result"),
    (["all"], "Every repo"),
]


def chip(x, y, word, fill="band", ink="accent"):
    w = len(word) * 8.4 + 22
    return notched(x, y, w, 30, fill, 4) + (
        f'<text x="{x + w / 2}" y="{y + 20}" font-family="{MONO}" font-size="14" font-weight="600" '
        f'class="f-{ink}" text-anchor="middle">{escape(word)}</text>'
    ), w


def select(theme):
    row = 48
    b = []
    for n, (words, meaning) in enumerate(SELECT):
        y = n * row
        x = 0
        for word in words:
            part, w = chip(x, y, word)
            b.append(part)
            x += w + 8
        b.append(text(470, y + 20, meaning, 15, "ink", 500))
    # A worked example: terms joined into one selector.
    y = len(SELECT) * row + 18
    b.append(f'<rect x="0" y="{y - 10}" width="{W}" height="2" class="f-band"/>')
    y += 8
    x = 0
    for word, fill, ink in (("behind", "bar", "on-bar"), (",", "panel", "sub"), ("!diverged", "bar", "on-bar")):
        if word == ",":
            b.append(text(x + 2, y + 21, ",", 18, "sub", 700, family=MONO))
            x += 16
            continue
        part, w = chip(x, y, word, fill, ink)
        b.append(part)
        x += w + 8
    b.append(rich(470, y + 20, [("t", "A comma means "), ("b", "all"), ("t", " must match, "),
                                ("c", "!"), ("t", " excludes")], 15))
    h = y + 34
    desc = "; ".join(f"{' '.join(w)}: {m}" for w, m in SELECT) + \
        "; behind,!diverged: a comma means all must match, ! excludes"
    return svg(h, "--select terms", desc, "".join(b), theme)


# ── Updating by hand: a small terminal window ───────────────────────────────

UPDATE = [
    ("Homebrew", "brew update && brew upgrade cohors"),
    ("pip", "pip install -U cohors"),
    ("npm", "npm install -g @cohors/cli@latest"),
    ("Cargo", "cargo install cohors"),
    ("Shell installer: run the install line again", None),
    ("uvx or npx with @latest: nothing to do, each run is the newest", None),
]


def update(theme):
    bar, line = 38, 25
    lines = sum(2 if cmd else 1 for _, cmd in UPDATE)
    h = bar + 18 + lines * line + 14
    b = [notched(0, 0, W, h, "term"), notched(0, 0, W, bar, "term-bar")]
    for i, fill in enumerate(("orchid", "lavender", "band")):
        b.append(f'<rect x="{20 + i * 16}" y="{bar / 2 - 5}" width="10" height="10" class="f-{fill}"/>')
    b.append(text(W / 2, bar / 2 + 5, "updating cohors by hand", 13, "term-sub", 600, "middle", MONO))
    y = bar + 30
    for label, cmd in UPDATE:
        b.append(text(24, y, f"# {label}", 14, "term-sub", 400, family=MONO))
        y += line
        if cmd:
            b.append(
                f'<text x="24" y="{y}" font-family="{MONO}" font-size="14.5" xml:space="preserve">'
                f'<tspan class="f-term-prompt" font-weight="700">$ </tspan>'
                f'<tspan class="f-term-ink">{escape(cmd)}</tspan></text>'
            )
            y += line
    desc = "; ".join(f"{l}: {c}" if c else l for l, c in UPDATE)
    return svg(h, "Updating cohors by hand", desc, "".join(b), theme)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    builders = {
        "answers": answers, "compare": compare, "keys": keys, "ask": ask,
        "permissions": permissions, "select": select, "update": update,
    }
    for name, build in builders.items():
        for theme in ("light", "dark", "auto"):
            suffix = "" if theme == "auto" else f"-{theme}"
            (OUT / f"{name}{suffix}.svg").write_text(build(theme))
    print(f"wrote {len(builders)} tables x 3 to {OUT}")


if __name__ == "__main__":
    main()
