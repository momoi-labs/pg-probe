//! Payload-free process panic reporting.
//!
//! `catch_unwind` isolates collector failures, but Rust invokes the panic hook
//! first. Replacing the default hook prevents payloads, locations, and
//! backtraces from reaching stderr.
use std::panic;

pub const SAFE_PANIC_NOTICE: &str =
    "pg-probe: an internal collector panicked; the failure was isolated and no diagnostic detail is printed here";

pub fn install() {
    panic::set_hook(Box::new(|_info| {
        eprintln!("{SAFE_PANIC_NOTICE}");
    }));
}
