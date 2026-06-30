//! cohors-web binary — one source, two targets.
//!
//! - On `wasm32` (Trunk): the Leptos browser page ([`browser`]) talking to the
//!   server over HTTP; it links `cohors-core` only.
//! - On the host: a thin standalone launcher for the native server in the lib
//!   ([`cohors_web::standalone`]). The primary launcher is still `cohors web` in
//!   the TUI; this lets the server also run on its own.
#![cfg_attr(not(target_arch = "wasm32"), allow(unused_crate_dependencies))]

#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_arch = "wasm32")]
fn main() {
    browser::mount();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> anyhow::Result<()> {
    cohors_web::standalone::main(std::env::args().skip(1).collect())
}
