## Agent skills

### Engineering principles

Read `docs/PRINCIPLES.md` before making an implementation choice. It governs
how to decide when more than one implementation is correct; non-invasiveness
toward the observed server outranks collection speed.

Read the ADRs in `docs/adr/` that touch the area being changed. They record
deliberate deviations from the issues — do not "fix" something an ADR says was
chosen on purpose. ADRs are append-only: never rewrite the body of an existing
one. To reverse a decision, add a new ADR and set `status: superseded` and
`superseded_by` in the frontmatter of the old one.

### Release notes

A change that users can see, in the binary, the console or the report, ships
with a changeset in the same pull request. See `docs/releases.md`. Check with
`scripts/check-changeset.sh origin/main` before opening the pull request.

### Issue tracker

Issues and PRDs for this repository live in GitHub Issues. See
`docs/agents/issue-tracker.md`.

An epic is not done when its implementation sub-issues close. Every epic
carries a final conformance-review sub-issue, blocked by all the others, that
checks the delivered code against the epic's own acceptance criteria across the
whole epic rather than slice by slice. Drift accumulates between slices that
are each internally correct.

### Triage labels

This repository uses the default triage labels. See
`docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repository. See `docs/agents/domain.md`.

## Cursor Cloud specific instructions

`pg-probe` is a single Rust CLI crate (no long-running service). Standard commands
live in `README.md` (`## Development`) and `.github/workflows/ci.yml`.

- The crate sets `rust-version = "1.88"` in `Cargo.toml` and pins an exact
  channel in `rust-toolchain.toml` for CI/release reproducibility. Prefer that
  pin (currently 1.97.1) over an unpinned older toolchain.
- Lint/test/build: `cargo fmt --all -- --check`, `cargo clippy --all-targets --all-features
  --locked -- -D warnings`, `cargo test --all-targets --locked`, `cargo build
  --release --locked` (CI runs these, plus musl cross-builds and packaging-script
  tests).
- Running the app: `cargo run -- -D <dir>` (or `--data-directory <dir>`; the
  release binary is at `target/release/pg-probe`). Connection options match
  psql: `-h/--host`, `-p/--port`, `-U/--username`, `-d/--dbname`. Artifacts are
  never overwritten; collisions receive numeric suffixes.
- The collector reads Linux kernel/libc/`/proc` sources directly, so it only runs
  meaningfully on Linux (the cloud VM is Linux, so this is fine).
- Inspect an artifact: `gunzip -k snapshot.sqlite.gz` then
  `sqlite3 snapshot.sqlite '.tables'`. Host identity lives in `linux_identity`
  (columns are `kernel_sysname`, `kernel_release`, `kernel_machine`,
  `distribution_pretty_name`, etc.).

## Kiso

The UI follows Kiso. Read `docs/kiso.md` before changing a screen, and use
the `kiso-page` skill to build or change one.
