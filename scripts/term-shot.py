#!/usr/bin/env python3
"""Screenshot a terminal program: run it in a real pseudo-terminal, press real
keys, and save what it drew.

Used to check the cohors dashboard and its plain-terminal reports the way a
person sees them, rather than only through unit tests. The program under test
gets a genuine tty of the size you ask for, so it takes exactly the code paths
it takes in a terminal (alternate screen, raw mode, colour, key handling).

No dependencies beyond the Python standard library. The terminal emulation is
deliberately small: cursor addressing, erasing, colours and text attributes,
which is all a full-screen program like the dashboard uses.

    scripts/term-shot.py --size 119x53 --out shots --name overview \\
        --keys "wait:1500 shot:overview 5 enter wait:400 shot:detail q" \\
        -- cohors --root ~/code --watch

Each `shot:NAME` writes NAME.txt (plain text, as the screen reads) and
NAME.html (the same with colours, to open in a browser). Without any `shot:`
one is taken at the end under --name.

Keys: a single character presses that key; also enter, esc, tab, backtab,
space, up, down, left, right, pgup, pgdn, home, end, ctrl-c; `text:abc` types a
string; `wait:MS` lets the program draw; `resize:COLSxROWS` resizes the
terminal while the program is running, exactly as dragging a window edge does
(the size changes and the program is sent SIGWINCH).
"""

import argparse
import fcntl
import html
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

KEYS = {
    "enter": "\r", "esc": "\x1b", "tab": "\t", "backtab": "\x1b[Z", "space": " ",
    "up": "\x1b[A", "down": "\x1b[B", "right": "\x1b[C", "left": "\x1b[D",
    "pgup": "\x1b[5~", "pgdn": "\x1b[6~", "home": "\x1b[H", "end": "\x1b[F",
    "ctrl-c": "\x03",
}

# The 16 standard colours as a typical dark terminal draws them.
ANSI16 = [
    "#1c1b1a", "#d9776e", "#8cba99", "#cfa255", "#7f9fc9", "#c3a2bb", "#7fb8b8", "#d9d6d0",
    "#6b6862", "#e89a92", "#a9d4b5", "#e3bd77", "#9db8dc", "#d9bdd2", "#9fd0d0", "#f4f2ee",
]
THEMES = {
    "dark": {"bg": "#1c1b1a", "fg": "#d9d6d0", "osc": "1c1c/1b1b/1a1a"},
    "light": {"bg": "#f7f7f5", "fg": "#1f1e1c", "osc": "f7f7/f7f7/f5f5"},
}


def color256(n):
    """An xterm 256-colour index as a CSS colour."""
    if n < 16:
        return ANSI16[n]
    if n < 232:
        n -= 16
        levels = [0, 95, 135, 175, 215, 255]
        return "#%02x%02x%02x" % (levels[n // 36], levels[(n // 6) % 6], levels[n % 6])
    grey = 8 + (n - 232) * 10
    return "#%02x%02x%02x" % (grey, grey, grey)


class Style:
    __slots__ = ("fg", "bg", "bold", "dim", "italic", "underline", "reverse")

    def __init__(self):
        self.reset()

    def reset(self):
        self.fg = self.bg = None
        self.bold = self.dim = self.italic = self.underline = self.reverse = False

    def key(self):
        return (self.fg, self.bg, self.bold, self.dim, self.italic, self.underline, self.reverse)


class Screen:
    """A grid of (character, style) cells driven by the bytes a program writes."""

    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        self.style = Style()
        self.row = self.col = 0
        self.saved = (0, 0)
        self.blank = (" ", Style().key())
        self.grid = [[self.blank] * cols for _ in range(rows)]
        self.replies = []  # answers to queries the program made

    # ── writing ──────────────────────────────────────────────────────────────
    def put(self, ch):
        if self.col >= self.cols:
            self.col = 0
            self.newline()
        self.grid[self.row][self.col] = (ch, self.style.key())
        self.col += 1

    def newline(self):
        if self.row == self.rows - 1:
            self.grid.pop(0)
            self.grid.append([self.blank] * self.cols)
        else:
            self.row += 1

    def clamp(self):
        self.row = max(0, min(self.rows - 1, self.row))
        self.col = max(0, min(self.cols - 1, self.col))

    # ── escape sequences ─────────────────────────────────────────────────────
    def sgr(self, params):
        s, i = self.style, 0
        params = params or [0]
        while i < len(params):
            p = params[i]
            if p == 0:
                s.reset()
            elif p == 1:
                s.bold = True
            elif p == 2:
                s.dim = True
            elif p == 3:
                s.italic = True
            elif p == 4:
                s.underline = True
            elif p == 7:
                s.reverse = True
            elif p == 22:
                s.bold = s.dim = False
            elif p == 23:
                s.italic = False
            elif p == 24:
                s.underline = False
            elif p == 27:
                s.reverse = False
            elif 30 <= p <= 37:
                s.fg = ANSI16[p - 30]
            elif 90 <= p <= 97:
                s.fg = ANSI16[p - 90 + 8]
            elif p == 39:
                s.fg = None
            elif 40 <= p <= 47:
                s.bg = ANSI16[p - 40]
            elif 100 <= p <= 107:
                s.bg = ANSI16[p - 100 + 8]
            elif p == 49:
                s.bg = None
            elif p in (38, 48) and i + 1 < len(params):
                color = None
                if params[i + 1] == 5 and i + 2 < len(params):
                    color, i = color256(params[i + 2]), i + 2
                elif params[i + 1] == 2 and i + 4 < len(params):
                    color, i = "#%02x%02x%02x" % tuple(params[i + 2:i + 5]), i + 4
                if p == 38:
                    s.fg = color
                else:
                    s.bg = color
            i += 1

    def csi(self, body, final):
        private = body.startswith("?")
        nums = [int(x) if x.isdigit() else 0 for x in body.lstrip("?>=").split(";")] if body.strip("?>=") else []
        n = nums[0] if nums else 0
        if final == "m" and not private:
            self.sgr(nums)
        elif final in "Hf":
            self.row = (nums[0] if nums and nums[0] else 1) - 1
            self.col = (nums[1] if len(nums) > 1 and nums[1] else 1) - 1
        elif final == "A":
            self.row -= n or 1
        elif final == "B":
            self.row += n or 1
        elif final == "C":
            self.col += n or 1
        elif final == "D":
            self.col -= n or 1
        elif final == "G":
            self.col = (n or 1) - 1
        elif final == "d":
            self.row = (n or 1) - 1
        elif final == "J":
            if n == 2 or n == 3:
                self.grid = [[self.blank] * self.cols for _ in range(self.rows)]
            elif n == 0:
                self.grid[self.row][self.col:] = [self.blank] * (self.cols - self.col)
                for r in range(self.row + 1, self.rows):
                    self.grid[r] = [self.blank] * self.cols
        elif final == "K":
            if n == 0:
                self.grid[self.row][self.col:] = [self.blank] * (self.cols - self.col)
            elif n == 2:
                self.grid[self.row] = [self.blank] * self.cols
        elif final == "n" and n == 6:  # "where is the cursor?"
            self.replies.append("\x1b[%d;%dR" % (self.row + 1, self.col + 1))
        elif final == "c" and not private:  # "what are you?"
            self.replies.append("\x1b[?62;c")
        elif final in "hl" and private and n in (1049, 47, 1047) and final == "h":
            self.grid = [[self.blank] * self.cols for _ in range(self.rows)]
            self.row = self.col = 0
        self.clamp()

    def feed(self, text, theme):
        i, n = 0, len(text)
        while i < n:
            ch = text[i]
            if ch == "\x1b":
                if i + 1 >= n:
                    return text[i:]  # incomplete: keep for the next read
                nxt = text[i + 1]
                if nxt == "[":
                    j = i + 2
                    while j < n and not ("@" <= text[j] <= "~"):
                        j += 1
                    if j >= n:
                        return text[i:]
                    self.csi(text[i + 2:j], text[j])
                    i = j + 1
                elif nxt == "]":  # operating-system command, ends with BEL or ESC \
                    j = i + 2
                    while j < n and text[j] != "\x07" and not (text[j] == "\x1b" and j + 1 < n and text[j + 1] == "\\"):
                        j += 1
                    if j >= n:
                        return text[i:]
                    body = text[i + 2:j]
                    if body.startswith("11;?"):  # "what is your background colour?"
                        self.replies.append("\x1b]11;rgb:%s\x1b\\" % THEMES[theme]["osc"])
                    elif body.startswith("10;?"):
                        self.replies.append("\x1b]10;rgb:d9d9/d6d6/d0d0\x1b\\")
                    i = j + (1 if text[j] == "\x07" else 2)
                elif nxt == "7":
                    self.saved, i = (self.row, self.col), i + 2
                elif nxt == "8":
                    (self.row, self.col), i = self.saved, i + 2
                elif nxt in "()":  # character-set selection: ESC ( B
                    i += 3
                else:
                    i += 2
            elif ch == "\r":
                self.col, i = 0, i + 1
            elif ch == "\n":
                self.newline()
                i += 1
            elif ch == "\b":
                self.col, i = max(0, self.col - 1), i + 1
            elif ch == "\t":
                self.col, i = min(self.cols - 1, (self.col // 8 + 1) * 8), i + 1
            elif ch < " ":
                i += 1
            else:
                self.put(ch)
                i += 1
        return ""

    # ── reading it back ──────────────────────────────────────────────────────
    def used_rows(self):
        last = max((r for r in range(self.rows) if any(c[0] != " " for c in self.grid[r])), default=-1)
        return self.grid[:last + 1]

    def text(self):
        return "\n".join("".join(c[0] for c in row).rstrip() for row in self.used_rows()) + "\n"

    def html(self, theme, title):
        t = THEMES[theme]
        out = []
        for row in self.used_rows():
            line, run, current = [], [], None
            for ch, key in row + [(None, None)]:
                if key != current and run:
                    line.append(self.span("".join(run), current, t))
                    run = []
                current = key
                if ch is not None:
                    run.append(ch)
            out.append("".join(line).rstrip())
        # One block per screen row, each cell-run an inline block of the full
        # row height: backgrounds then meet edge to edge, as they do in a
        # terminal, instead of leaving the gaps a plain <pre> leaves between
        # lines.
        body = "".join("<div class=r>%s</div>" % (line or " ") for line in out)
        return (
            "<!doctype html><meta charset=utf-8><title>%s</title>"
            "<style>.t{margin:0;padding:14px 16px;background:%s;color:%s;"
            "font:13px/16px 'SF Mono',Menlo,'DejaVu Sans Mono',Consolas,monospace;"
            "width:max-content}.r{height:16px;white-space:pre}"
            ".r span{display:inline-block;height:16px;vertical-align:top}</style>"
            "<body style='margin:0;background:%s'><div class=t>%s</div></body>\n"
        ) % (html.escape(title), t["bg"], t["fg"], t["bg"], body)

    @staticmethod
    def mix(a, b, share):
        """Blend two #rrggbb colours: `share` of `a`, the rest of `b`."""
        if not (isinstance(a, str) and isinstance(b, str) and len(a) == 7 and len(b) == 7):
            return a
        parts = [round(int(a[i:i + 2], 16) * share + int(b[i:i + 2], 16) * (1 - share)) for i in (1, 3, 5)]
        return "#%02x%02x%02x" % tuple(parts)

    @staticmethod
    def span(text, key, theme):
        fg, bg, bold, dim, italic, underline, reverse = key
        fg, bg = fg or theme["fg"], bg or theme["bg"]
        if reverse:
            fg, bg = bg, fg
        if dim:
            # Dim fades the text towards its background and leaves the
            # background alone, which is what a terminal does.
            fg = Screen.mix(fg, bg, 0.55)
        css = []
        if fg != theme["fg"]:
            css.append("color:%s" % fg)
        if bg != theme["bg"]:
            css.append("background:%s" % bg)
        if bold:
            css.append("font-weight:700")
        if italic:
            css.append("font-style:italic")
        if underline:
            css.append("text-decoration:underline")
        safe = html.escape(text)
        return "<span style='%s'>%s</span>" % (";".join(css), safe) if css else "<span>%s</span>" % safe


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--size", default="119x53", help="COLSxROWS of the terminal (default 119x53)")
    ap.add_argument("--out", default="shots", help="directory for the shots")
    ap.add_argument("--name", default="shot", help="name of the final shot")
    ap.add_argument("--keys", default="", help="space-separated keys, wait:MS and shot:NAME steps")
    ap.add_argument("--theme", default="dark", choices=sorted(THEMES), help="terminal background")
    ap.add_argument("--settle", type=int, default=1200, help="ms to let the program draw before the first step")
    ap.add_argument("--env", action="append", default=[], help="NAME=VALUE for the program (repeatable)")
    ap.add_argument("command", nargs=argparse.REMAINDER, help="-- program and its arguments")
    args = ap.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        ap.error("give the program to run after --")
    cols, rows = (int(x) for x in args.size.lower().split("x"))
    os.makedirs(args.out, exist_ok=True)

    pid, fd = pty.fork()
    if pid == 0:  # the program under test
        env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor", COLUMNS=str(cols), LINES=str(rows))
        env.pop("NO_COLOR", None)
        for pair in args.env:
            name, _, value = pair.partition("=")
            env[name] = value
        os.execvpe(command[0], command, env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    screen, pending, alive = Screen(cols, rows), "", True
    decoder = __import__("codecs").getincrementaldecoder("utf-8")("replace")

    def pump(ms):
        """Read what the program writes for `ms` milliseconds."""
        nonlocal pending, alive, screen
        deadline = time.time() + ms / 1000
        while alive and time.time() < deadline:
            ready, _, _ = select.select([fd], [], [], max(0, min(0.05, deadline - time.time())))
            if not ready:
                continue
            try:
                data = os.read(fd, 65536)
            except OSError:
                data = b""
            if not data:
                alive = False
                break
            pending = screen.feed(pending + decoder.decode(data), args.theme)
            for reply in screen.replies:
                os.write(fd, reply.encode())
            screen.replies.clear()

    def shot(name):
        with open(os.path.join(args.out, name + ".txt"), "w") as f:
            f.write(screen.text())
        with open(os.path.join(args.out, name + ".html"), "w") as f:
            f.write(screen.html(args.theme, name))
        print("shot %s (%d lines)" % (name, len(screen.used_rows())))

    pump(args.settle)
    took = False
    for step in args.keys.split():
        if step.startswith("wait:"):
            pump(int(step[5:]))
        elif step.startswith("resize:"):
            cols, rows = (int(x) for x in step[7:].lower().split("x"))
            fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            # A real terminal keeps what was on screen and lets the program
            # repaint; start from a blank grid of the new size so anything the
            # program fails to redraw shows up as missing rather than stale.
            screen = Screen(cols, rows)
            if alive:
                os.kill(pid, signal.SIGWINCH)
            pump(600)
        elif step.startswith("shot:"):
            shot(step[5:])
            took = True
        elif alive:
            keys = step[5:] if step.startswith("text:") else KEYS.get(step, step)
            os.write(fd, keys.encode())
            pump(250)
    if not took:
        pump(300)
        shot(args.name)

    if alive:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        _, status = os.waitpid(pid, 0)
        code = os.waitstatus_to_exitcode(status)
    except ChildProcessError:
        code = 0
    print("exit %s" % code)


if __name__ == "__main__":
    sys.exit(main())
