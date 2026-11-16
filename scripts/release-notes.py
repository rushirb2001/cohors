#!/usr/bin/env python3
"""Release notes for the listings that are rebuilt on every release.

CHANGELOG.md is the one place a release is described. This turns one
version's section of it into what a listing shows, so no listing needs a hand
edit when a release goes out:

  python3 scripts/release-notes.py summary 0.7.7
      One line per entry of the [0.7.7] section: its bold lead, finished to
      the end of its first sentence, under Added / Changed / Fixed.

  python3 scripts/release-notes.py stamp packaging/pypi/README.md 0.7.7
      Puts that summary between the file's `<!-- whats-new:start -->` and
      `<!-- whats-new:end -->` markers. The PyPI workflow runs this before it
      builds the wheels, so pypi.org always shows the release it serves.

  python3 scripts/release-notes.py footer 0.7.7
      packaging/github-release/footer.md with the version filled in: the
      install routes cargo-dist does not write into the GitHub Release. The
      release-notes workflow appends it to the release.

`--unreleased` reads the [Unreleased] section instead, to preview a release
that has not been cut yet. Without it, a version the CHANGELOG has no section
for stamps a link to the changelog and says so on stderr (the publish goes on).
"""

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHANGELOG = ROOT / "CHANGELOG.md"
FOOTER = ROOT / "packaging" / "github-release" / "footer.md"
REPO = "https://github.com/rushirb2001/cohors"
START, END = "<!-- whats-new:start -->", "<!-- whats-new:end -->"


def section(text, version):
    """The body of the `## [version]` section, or None if there is none."""
    body, found = [], False
    for line in text.splitlines():
        if line.startswith("## "):
            if found:
                break
            m = re.match(r"## \[([^\]]+)\]", line)
            found = bool(m) and m.group(1) == version
            continue
        if found:
            body.append(line)
    return "\n".join(body) if found else None


def groups(body):
    """The section's first paragraph (if it has one before any `###`), and
    its `### Heading` groups as (heading, [entry]) with each entry's wrapped
    lines joined. Nested bullets belong to their entry and are dropped."""
    intro, out, current = [], [], None
    for line in body.splitlines():
        if line.startswith("### "):
            current = (line[4:].strip(), [])
            out.append(current)
        elif current is None:
            if line.strip():
                intro.append(line.strip())
            elif intro:
                current = ("", [])  # the first paragraph is over
        elif line.startswith("- "):
            current[1].append(line[2:].strip())
        elif line.startswith("  ") and line.strip() and current[1]:
            if not line.strip().startswith("- "):
                current[1][-1] += " " + line.strip()
    return " ".join(intro), [g for g in out if g[0] and g[1]]


def words(text):
    return len(text.replace("**", "").replace("`", "").split())


def lead(entry):
    """An entry cut to its first sentence. A bold lead that is a sentence of
    its own ends there; otherwise the cut is the first `. ` (or a `:`/`;` once
    five words have been said) outside code spans and parentheses."""
    in_code = in_bold = False
    depth = 0
    i, n = 0, len(entry)
    while i < n:
        c = entry[i]
        if c == "`":
            in_code = not in_code
        elif not in_code and entry.startswith("**", i):
            in_bold = not in_bold
            if not in_bold and entry[i - 1] == ".":
                return entry[: i + 2]
            i += 2
            continue
        elif not in_code and c == "(":
            depth += 1
        elif not in_code and c == ")" and depth:
            depth -= 1
        elif not in_code and not in_bold and depth == 0:
            at_break = i + 1 == n or entry[i + 1] == " "
            if c == "." and at_break:
                return entry[: i + 1]
            if c in ":;" and at_break and words(entry[:i]) >= 5:
                return entry[:i].rstrip() + "."
        i += 1
    return entry if entry.endswith(".") else entry + "."


def summary(version, unreleased=False):
    """The Markdown summary of a version, or None if the CHANGELOG lacks it."""
    body = section(CHANGELOG.read_text(encoding="utf-8"), "Unreleased" if unreleased else version)
    if body is None:
        return None
    intro, grouped = groups(body)
    out = [intro, ""] if intro else []
    for heading, entries in grouped:
        out += [f"### {heading}", ""] + [f"- {lead(e)}" for e in entries] + [""]
    return "\n".join(out).rstrip() + "\n"


def stamp(path, version, unreleased=False):
    text = Path(path).read_text(encoding="utf-8")
    if START not in text or END not in text or text.index(START) > text.index(END):
        sys.exit(f"{path}: needs a {START} line before a {END} line")
    notes = summary(version, unreleased)
    if notes is None:
        print(f"CHANGELOG.md has no [{version}] section; {path} links to the changelog instead",
              file=sys.stderr)
        block = f"## What's new\n\nSee the [changelog]({REPO}/blob/main/CHANGELOG.md).\n"
    else:
        block = (f"## What's new in {version}\n\n{notes}\n"
                 f"Full notes: [cohors {version} on GitHub]({REPO}/releases/tag/v{version}).\n")
    head, rest = text.split(START, 1)
    tail = rest.split(END, 1)[1]
    Path(path).write_text(f"{head}{START}\n{block}{END}{tail}", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--unreleased", action="store_true",
                        help="read the [Unreleased] section (to preview a release)")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("summary").add_argument("version")
    p = sub.add_parser("stamp")
    p.add_argument("file")
    p.add_argument("version")
    sub.add_parser("footer").add_argument("version")
    args = parser.parse_args()
    version = args.version.removeprefix("v")

    if args.command == "summary":
        notes = summary(version, args.unreleased)
        if notes is None:
            sys.exit(f"CHANGELOG.md has no [{version}] section")
        sys.stdout.write(notes)
    elif args.command == "stamp":
        stamp(args.file, version, args.unreleased)
    else:
        sys.stdout.write(FOOTER.read_text(encoding="utf-8").replace("{version}", version))


if __name__ == "__main__":
    main()
