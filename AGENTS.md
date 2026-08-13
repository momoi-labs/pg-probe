## Agent skills

### Issue tracker

Issues and PRDs for this repository live in GitHub Issues. See
`docs/agents/issue-tracker.md`.

### Triage labels

This repository uses the default triage labels. See
`docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repository. See `docs/agents/domain.md`.

## Cursor Cloud specific instructions

`pg-probe` is a single Rust CLI crate (no long-running service). Standard commands
live in `README.md` (`## Development`) and `.github/workflows/ci.yml`.

- The crate sets `rust-version = "1.85"` in `Cargo.toml`. The VM base image ships an
  older default Rust; the startup update script switches to the `stable` channel
  (currently 1.97.x), so use `stable` rather than any pinned older toolchain.
- Lint/test/build: `cargo fmt --check`, `cargo clippy --all-targets --all-features
  --locked -- -D warnings`, `cargo test --all-targets --locked`, `cargo build
  --release --locked` (CI runs these exact commands).
- Running the app: `cargo run -- snapshot --output <path>.sqlite.gz` (or the release
  binary at `target/release/pg-probe`). The `--output` path must NOT already exist —
  `pg-probe` never overwrites an artifact, so use a fresh path each run.
- The collector reads Linux kernel/libc/`/proc` sources directly, so it only runs
  meaningfully on Linux (the cloud VM is Linux, so this is fine).
- Inspect an artifact: `gunzip -k snapshot.sqlite.gz` then
  `sqlite3 snapshot.sqlite '.tables'`. Host identity lives in `linux_identity`
  (columns are `kernel_sysname`, `kernel_release`, `kernel_machine`,
  `distribution_pretty_name`, etc.).
