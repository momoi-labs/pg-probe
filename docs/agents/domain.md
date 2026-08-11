# Domain Docs

How the engineering skills should consume this repository's domain documentation
when exploring the codebase.

## Before exploring, read these

- `CONTEXT.md` at the repository root, or
- `CONTEXT-MAP.md` at the repository root if it exists — it points at one
  `CONTEXT.md` per context.
- `docs/adr/` — read ADRs that touch the area being worked on.

If any of these files do not exist, proceed silently. The `/domain-modeling`
skill creates them lazily when terms or decisions are resolved.

## File structure

This is a single-context repository:

```
/
├── CONTEXT.md
├── docs/adr/
└── src/
```

## Use the glossary's vocabulary

When naming a domain concept, use the term defined in `CONTEXT.md`. If a needed
concept is not in the glossary, reconsider whether existing language applies or
note the gap for `/domain-modeling`.

## Flag ADR conflicts

If output contradicts an existing ADR, surface it explicitly rather than
silently overriding it.
