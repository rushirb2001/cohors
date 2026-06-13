//! File-based logging.
//!
//! Logs go to `<cache>/cohors.log`, never stdout/stderr — the `scan` JSON and,
//! later, the TUI own those. Setup is best-effort: if the log file can't be
//! opened we simply run without logging rather than failing.
//!
//! Set `COHORS_LOG` (e.g. `COHORS_LOG=debug`) to change the level.

pub fn init() {
    let Ok(path) = cohors_config::paths::log_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };

    // `MakeWriter` wants a fresh writer per event; hand out clones of the file
    // handle (falling back to a sink if a clone ever fails, so we never panic).
    let make_writer = move || -> Box<dyn std::io::Write> {
        match file.try_clone() {
            Ok(handle) => Box::new(handle),
            Err(_) => Box::new(std::io::sink()),
        }
    };

    let filter = tracing_subscriber::EnvFilter::try_from_env("COHORS_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_writer(make_writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init();
}
