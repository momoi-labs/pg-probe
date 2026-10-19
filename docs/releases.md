# Releases

Changesets keeps one version PR open on `main`. Merging it tags the version
and runs the release workflow, which builds, checks and publishes the musl
archives on GitHub. The setup follows self-host's, without Homebrew or macOS.

## Add release notes

From the repository root:

```sh
npm ci
npm run changeset
```

Select `pg-probe`, choose the version bump, and describe the user-visible
change in English. Commit the generated `.changeset/*.md` with the change.
Use a patch for fixes and a minor for new behavior. Documentation-only changes
need no changeset.

On `main`, the Version workflow opens or updates `chore: release pg-probe`.
Review its changelog and version, wait for CI, then merge it. The PR updates
`CHANGELOG.md`, `package.json`, `package-lock.json`, `Cargo.toml` and
`Cargo.lock` together. Do not bump these versions by hand.

## First release

The root `package.json` starts at `0.0.0`, and the first changeset proposes a
minor bump, so the first version PR releases `v0.1.0`. `Cargo.toml` already
says `0.1.0`; the version PR keeps it there.

## How it runs

The Version workflow uses the built-in `GITHUB_TOKEN`. GitHub does not trigger
workflows from events that token creates, so the workflow dispatches CI on the
version PR itself, and after tagging it calls the release workflow directly.
The repository setting **Allow GitHub Actions to create and approve pull
requests** must stay on.

The release workflow checks that the tag, `package.json`, `Cargo.toml` and
`Cargo.lock` agree, runs fmt, clippy and the tests, builds the review console,
and builds both musl targets with GoReleaser. It fails when the console is not
embedded in a binary. It smoke-tests the archives on Ubuntu, Rocky Linux and
Alpine, verifies `SHA256SUMS`, attests the assets, and publishes the release
with that version's section of `CHANGELOG.md` as its notes.

GoReleaser also wraps each binary in a `.deb` and an `.rpm`. The binary is
static musl, so the packages declare no dependencies, not even libc. The
release fails if either package declares one, and it installs each package
and runs the smoke test against `/usr/bin/pg-probe`.

## Retry a release

In **Actions > Release > Run workflow**, enter an existing tag to retry a
failed release. Re-running the Version workflow alone does not retry a tag it
has already created. The release fails if a GitHub release for the tag
already exists.

## Verification

Run these checks before merging release changes:

```sh
npm ci
npm run test:release
goreleaser check
actionlint
```

The version tests run Changesets in a temporary Git repository, check Cargo
synchronization, and verify that tagging a version twice does not publish
twice.
