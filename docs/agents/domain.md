# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Domain reference

- **`CONTEXT.md`** at the repo root is the canonical glossary. Consult it as a
  lookup reference when naming or defining domain concepts — don't ingest it
  wholesale before starting work.
- **`docs/adr/`** holds architectural decisions. Read only the ADRs that touch
  the area being changed (each has a one-line `**Status**` header; skip
  superseded or unbuilt ones unless working on that feature).

If any of these files don't exist, proceed silently. Don't flag their absence; don't suggest creating them upfront. Producer skills create them lazily when terms or decisions get resolved.

## File structure

This repo uses a single-context layout:

```text
/
├── CONTEXT.md
├── docs/adr/
└── src/
```

## Use the glossary's vocabulary

When output names a domain concept in an issue title, refactor proposal, hypothesis, or test name, use the term as defined in `CONTEXT.md`. Don't drift to synonyms the glossary explicitly avoids.

If the concept needed isn't in the glossary yet, either reconsider the wording or note the gap for `/grill-with-docs`.

## Flag ADR conflicts

If output contradicts an existing ADR, surface it explicitly rather than silently overriding.
