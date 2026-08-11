# Issue tracker: GitHub

Issues and PRDs for this repo live as GitHub issues. Use the `gh` CLI for all
operations.

## Conventions

- **Create an issue**: `gh issue create --title "..." --body "..."`. Use a
  heredoc for multi-line bodies.
- **Read an issue**: `gh issue view <number> --comments`, filtering comments by
  `jq` and also fetching labels.
- **List issues**: `gh issue list --state open --json
  number,title,body,labels,comments --jq '[.[] | {number, title, body, labels:
  [.labels[].name], comments: [.comments[].body]}]'` with appropriate `--label`
  and `--state` filters.
- **Comment on an issue**: `gh issue comment <number> --body "..."`
- **Apply / remove labels**: `gh issue edit <number> --add-label "..."` /
  `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

Infer the repo from `git remote -v` — `gh` does this automatically when run
inside a clone.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## When a skill says "publish to the issue tracker"

Create a GitHub issue.

## When a skill says "fetch the relevant ticket"

Run `gh issue view <number> --comments`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is a single issue with **child** issues as
tickets.

- **Map**: a single issue labelled `wayfinder:map`, holding the Notes /
  Decisions-so-far / Fog body. Create it with `gh issue create --label
  wayfinder:map`.
- **Child ticket**: an issue linked to the map as a GitHub sub-issue (`gh api`
  on the sub-issues endpoint). Where sub-issues are not enabled, add the child
  to a task list in the map body and put `Part of #<map>` at the top of the
  child body. Labels: `wayfinder:<type>` (`research`/`prototype`/`grilling`/
  `task`). Once claimed, the ticket is assigned to the driving developer.
- **Blocking**: GitHub native issue dependencies. Add an edge with `gh api
  --method POST repos/<owner>/<repo>/issues/<child>/dependencies/blocked_by -F
  issue_id=<blocker-db-id>`, where `<blocker-db-id>` is the blocker numeric
  database ID (`gh api repos/<owner>/<repo>/issues/<n> --jq .id`). Where
  dependencies are unavailable, use `Blocked by: #<n>, #<n>` at the top of the
  child body.
- **Frontier query**: list the map open children, dropping any with an open
  blocker or assignee; first in map order wins.
- **Claim**: `gh issue edit <n> --add-assignee @me` before work starts.
- **Resolve**: comment with the answer, close the ticket, then append a context
  pointer to the map Decisions-so-far.
