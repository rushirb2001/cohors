# vhs tape for the TUI demo assets: drives `cohors demo` (deterministic sample
# fleet) through its headline features. Emits both the README GIF and the still
# preview, so the two can never drift apart.
#   Regenerate with:  vhs assets/tui.tape
Output assets/tui.gif

Require cohors

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 26
Set Width 2560
Set Height 1440
Set Padding 30
# 20fps: the dashboard is a static surface between keystrokes, so the extra
# frames bought nothing but megabytes on a README that has to load fast.
Set Framerate 20
# Light theme (matches a light terminal); rendered at ~2x then downscaled for a
# crisp, compact GIF. GitHub-Light-inspired ANSI palette so the semantic colours
# (green/red/yellow/blue/magenta/cyan) stay readable on white.
Set Theme { "background": "#FCFCFC", "foreground": "#1F2328", "cursor": "#1F2328", "selection": "#D0D7DE", "black": "#24292F", "red": "#CF222E", "green": "#116329", "yellow": "#9A6700", "blue": "#0969DA", "magenta": "#8250DF", "cyan": "#1B7C83", "white": "#6E7781", "brightBlack": "#57606A", "brightRed": "#A40E26", "brightGreen": "#1A7F37", "brightYellow": "#7D4E00", "brightBlue": "#218BFF", "brightMagenta": "#A475F9", "brightCyan": "#3192AA", "brightWhite": "#8C959F" }
Set PlaybackSpeed 1.0

# Launch the demo fleet (nothing touches disk); hide the typing of the command.
Hide
Type "cohors demo"
Enter
Sleep 3s
Show
Sleep 1.5s

# Move the cursor down the fleet — the context card at the bottom follows.
# Land on `payments`, the multi-branch repo, so the still shows a per-branch row
# and a context card with something in it (the top row is a deliberately errored
# repo: good for proving one bad repo can't crash the fleet, bad as a hero shot).
Down@450ms 2
Sleep 2s

# The hero still. The same frame the README leads with, so the GIF and the PNG
# can never drift apart.
Screenshot assets/tui-preview.png
Sleep 1s

# Open the full drill-in detail on that repo, then close it.
Enter
Sleep 3s
Escape
Sleep 1s

# Keep moving down the fleet.
Down@450ms 3
Sleep 1.5s

# Fuzzy-filter the fleet by name.
Type "/pay"
Sleep 2s
Escape
Sleep 1s

# The command palette: every action reachable by name, `!` for a shell command
# across the selection.
Type ":"
Sleep 2.5s
Escape
Sleep 1s

# The weekly standup view, then back.
Tab
Sleep 3s
Escape
Sleep 1s

# No help-overlay scene: the hint bar along the bottom already shows the keymap
# in every frame, so the `?` overlay only added weight.

# Quit.
Type "q"
Sleep 1.5s
