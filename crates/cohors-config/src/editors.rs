//! Detecting which editors are actually installed, by scanning `PATH`.
//!
//! This is binary-only (it reads the environment and the filesystem), so it
//! stays out of the pure `cohors-core`. The candidate list is roughly ordered
//! by how commonly developers use each tool to open a repo folder; GUI editors
//! come before terminal editors because "open this repo" usually means "open the
//! folder in my editor".

/// A known editor: the command to launch it and a human label for the picker.
pub struct Editor {
    pub command: &'static str,
    pub label: &'static str,
}

/// The editors cohors knows how to offer, GUI first, then terminal. Detection
/// keeps only the ones present on `PATH`.
const KNOWN: &[Editor] = &[
    Editor {
        command: "code",
        label: "VS Code",
    },
    Editor {
        command: "cursor",
        label: "Cursor",
    },
    Editor {
        command: "zed",
        label: "Zed",
    },
    Editor {
        command: "subl",
        label: "Sublime Text",
    },
    Editor {
        command: "windsurf",
        label: "Windsurf",
    },
    Editor {
        command: "code-insiders",
        label: "VS Code Insiders",
    },
    Editor {
        command: "codium",
        label: "VSCodium",
    },
    Editor {
        command: "idea",
        label: "IntelliJ IDEA",
    },
    Editor {
        command: "pycharm",
        label: "PyCharm",
    },
    Editor {
        command: "webstorm",
        label: "WebStorm",
    },
    Editor {
        command: "nvim",
        label: "Neovim",
    },
    Editor {
        command: "vim",
        label: "Vim",
    },
    Editor {
        command: "hx",
        label: "Helix",
    },
    Editor {
        command: "emacs",
        label: "Emacs",
    },
    Editor {
        command: "micro",
        label: "micro",
    },
    Editor {
        command: "nano",
        label: "nano",
    },
];

/// The installed editors, in the order above. Cheap enough to call when the
/// picker opens (a handful of directory probes, no process spawning).
pub fn detected() -> Vec<&'static Editor> {
    KNOWN.iter().filter(|e| on_path(e.command)).collect()
}

/// The first installed GUI-style editor's command, used as the last-resort
/// default when neither config nor `$EDITOR`/`$VISUAL` is set.
pub fn first_detected_command() -> Option<&'static str> {
    KNOWN.iter().find(|e| on_path(e.command)).map(|e| e.command)
}

/// Whether an arbitrary command (e.g. `lazygit`) is installed on `PATH`.
pub fn installed(command: &str) -> bool {
    on_path(command)
}

/// Whether `cmd` resolves to an executable on `PATH`. On Windows we also try the
/// `PATHEXT` extensions (`.exe`, `.cmd`, …) since editors ship as `.cmd` shims.
fn on_path(cmd: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string())
            .split(';')
            .map(|s| s.trim_start_matches('.').to_string())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let mut candidate = dir.join(cmd);
            if !ext.is_empty() {
                candidate.set_extension(ext);
            }
            if candidate.is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_does_not_panic() {
        // We can't assert on the host's installed editors, but detection must
        // run cleanly and the candidate list must be non-empty.
        let _ = detected();
        assert!(!KNOWN.is_empty());
    }
}
