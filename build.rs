//! Embeds the built review console from `web/dist` into the binary, so
//! `pg-probe analyze` serves it with no runtime dependency (ADR-0007).
//!
//! When `web/dist` has not been built, the list is empty and the binary
//! serves a page that explains how to build it. That keeps `cargo build`
//! working without Node.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let dist = Path::new("web/dist");
    println!("cargo:rerun-if-changed=web/dist");

    let mut files = Vec::new();
    if dist.join("index.html").is_file() {
        collect(dist, &mut files);
    }
    files.sort();

    let mut entries = String::new();
    for path in files {
        let relative = path
            .strip_prefix(dist)
            .expect("collected under dist")
            .to_string_lossy()
            .replace('\\', "/");
        let absolute = fs::canonicalize(&path).expect("collected file exists");
        entries.push_str(&format!(
            "    ({relative:?}, include_bytes!({:?})),\n",
            absolute.to_string_lossy()
        ));
    }

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(
        out.join("console_assets.rs"),
        format!("pub static ASSETS: &[(&str, &[u8])] = &[\n{entries}];\n"),
    )
    .expect("write console_assets.rs");
}

fn collect(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("read web/dist") {
        let path = entry.expect("read web/dist entry").path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}
