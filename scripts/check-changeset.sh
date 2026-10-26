#!/usr/bin/env bash
# Fails when the commits on HEAD since it left BASE touch what ships in the pg-probe
# binary but add or edit no changeset. See docs/releases.md.
#
#   scripts/check-changeset.sh BASE [HEAD]
set -euo pipefail

base=${1:?usage: check-changeset.sh BASE [HEAD]}
head=${2:-HEAD}

# The binary is built from the Rust sources, the schema, and the console in
# web/, which build.rs embeds, with the pinned toolchain and the SQLite flags
# in .cargo/config.toml. The sample artifacts in web/dev-artifacts/ do not
# ship.
shipped='^(src/|schema/|web/|build\.rs$|Cargo\.(toml|lock)$|\.cargo/config\.toml$|rust-toolchain\.toml$)'
unshipped='^web/dev-artifacts/'
changeset='^\.changeset/.+\.md$'

# Three dots compare HEAD with the point where it left BASE, so newer commits
# on BASE do not count as changes.
files=$(git diff --name-only "$base...$head" | grep -Ev "$unshipped" || true)
if ! grep -Eq "$shipped" <<<"$files"; then
  echo "No change to what ships in the binary; no changeset needed."
  exit 0
fi

notes=$(git diff --name-only --diff-filter=ACMR "$base...$head" | grep -E "$changeset" | grep -v '^\.changeset/README\.md$' || true)
if [ -n "$notes" ]; then
  printf 'Changeset found:\n%s\n' "$notes"
  exit 0
fi

echo "This change touches what ships in the binary but has no changeset:"
grep -E "$shipped" <<<"$files" | sed 's/^/  /'
echo
echo "Run 'npm run changeset' and commit the .changeset/*.md it writes."
echo "If users see no difference, such as a refactor or a test, add the"
echo "no-changeset label to the pull request instead."
exit 1
