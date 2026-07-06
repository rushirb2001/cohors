# vhs tape for the TUI demo GIF: drives `cohors demo` (deterministic sample
# fleet) through its headline features. Regenerate with:  vhs assets/tui.tape
Output assets/tui.gif

Require cohors

Set Shell "bash"
Set FontFamily "JetBrains Mono"
Set FontSize 26
Set Width 2560
Set Height 1440
Set Padding 30
Set Framerate 24
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
Down@450ms 4
Sleep 1.5s

# Open the full drill-in detail, then close it.
Enter
Sleep 3s
Escape
Sleep 1s

# Fuzzy-filter the fleet by name.
Type "/pay"
Sleep 2s
Escape
Sleep 1s

# The weekly standup view, then back.
Tab
Sleep 3s
Escape
Sleep 1s

# Quit.
Type "q"
Sleep 1.5s
