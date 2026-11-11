//! Compile the browser dashboard into the `cohors` binary, when there is one.
//!
//! `cohors web` serves a page built from `crates/cohors-web` with Trunk (Rust
//! compiled to WebAssembly). Building that page needs Trunk and the wasm32
//! target, which an ordinary `cargo build` should not require. So the page is
//! built separately (the release workflow runs `trunk build --release`), and
//! its output directory is handed to this script in `COHORS_WEB_DIST`. Every
//! file in it is then compiled into the binary with `include_bytes!`, and an
//! installed `cohors web` serves them from memory: no checkout, no toolchain.
//!
//! Without `COHORS_WEB_DIST` the list is empty, and `cohors web` falls back to
//! building the page itself, which works inside a checkout of cohors.
//!
//! The output is `$OUT_DIR/web_assets.rs`, one static:
//!
//!     pub static WEB_ASSETS: &[(&str, &[u8])] = &[("index.html", include_bytes!("…")), …];

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=COHORS_WEB_DIST");
    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("web_assets.rs");

    let mut files = Vec::new();
    if let Some(dist) = env::var_os("COHORS_WEB_DIST").filter(|v| !v.is_empty()) {
        let dist = PathBuf::from(dist);
        // A mistake here would ship a binary whose dashboard is a blank page,
        // so stop the build instead.
        assert!(
            dist.join("index.html").is_file(),
            "COHORS_WEB_DIST={} has no index.html; build the page first with \
             `trunk build --release` in crates/cohors-web",
            dist.display()
        );
        collect(&dist, &dist, &mut files);
        files.sort();
    }

    let mut code = String::from("pub static WEB_ASSETS: &[(&str, &[u8])] = &[\n");
    for (rel, path) in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        // `{:?}` writes a quoted, escaped Rust string literal.
        code.push_str(&format!(
            "    ({rel:?}, include_bytes!({:?})),\n",
            path.display().to_string()
        ));
    }
    code.push_str("];\n");
    fs::write(out, code).expect("writing web_assets.rs");
}

/// Every file under `dir`, as (path relative to `root` with `/` separators,
/// absolute path).
fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let rel = path.strip_prefix(root).expect("inside the root");
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            let absolute = fs::canonicalize(&path).expect("an existing file");
            files.push((rel, absolute));
        }
    }
}
