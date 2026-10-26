---
status: accepted
date: 2026-10-26
supersedes: []
superseded_by: null
tags: [release, ci]
---

# GitHub-hosted runners once the repository is public

CI and releases ran on Blacksmith, whose free tier is 3,000 Linux 2-vCPU
minutes a month for the organization. ADR 0010 added macOS builds there, and
one Blacksmith macOS minute counts as 20 of those. A pull request cost about
76 of them and a release about 300, so the free tier ran out within a month.
Blacksmith gives no extra minutes to public repositories.

## Decision

When the repository becomes public, every workflow moves to GitHub-hosted
runners, which cost nothing for a public repository:

- Linux on `ubuntu-24.04` and `ubuntu-24.04-arm`.
- macOS on `macos-latest` for arm64 and `macos-15-intel` for x86_64. This
  replaces ADR 0010's cross-build of the x86_64 archive on Apple Silicon:
  each archive builds and runs on its own architecture again, without
  Rosetta.

The switch merges together with the change of visibility. Before it, GitHub
bills these runners, and the organization's account cannot pay for them.

## Considered options

- **Stay on Blacksmith and run fewer jobs.** Rejected as the lasting answer:
  dropping macOS and arm64 from CI moves those failures to release time. It
  remains the fallback if the repository stays private.
- **Self-hosted runners on the owner's machines.** Rejected: a machine that is
  off blocks CI and releases.
- **Blacksmith's open-source program.** Not pursued: it takes an application,
  and GitHub's runners already cost nothing.

## Consequences

- GitHub-hosted runners are slower than Blacksmith's, and macOS jobs can wait
  in a queue.
- If the repository ever goes private again, GitHub bills these runners, and
  this ADR needs a successor.
