# Agent instructions

### Working together

- For implementation requests, finish the authorized work, including verification. Resolve routine choices from repo conventions and state material assumptions.
- Questions about feasibility, explanations, or opinions are read-only. Answer first; get authorization before editing. A direct request such as "can you update this file" authorizes that update.
- Ask when missing information materially affects correctness or scope; continue independent work while waiting. Earlier authorization still applies.
- Incorporate corrections during work and answer side questions without losing the original task unless the user cancels or replaces it.
- Never touch production, live databases, or daily-driver build/preview channels without explicit authorization. Name the target before touching it. Prepare authorized work before requesting any remaining approval.

### Skills and instructions

- Explicit user instructions take precedence over skill guidelines. Apply skills within the authorized scope; do not infer extra approval gates.
- If a skill blocks progress or requires confirmation, link its exact `SKILL.md`, quote the relevant rule, and explain why it applies.

### Communication and delegation

- Lead with the result. Keep prose concise and concrete; use lists when they help. Report changes, verification, and any remaining blockers. Avoid stock phrases and em dashes.
- Use one agent for ordinary tasks. Delegate for breadth or independent review when worthwhile, with bounded tasks and explicit file ownership to avoid collisions.

### Issue tracker

Issues are tracked in GitHub Issues for `imran-vz/dbunk` using the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Domain docs

This repo uses a single-context domain docs layout. See `docs/agents/domain.md`.

### Status and plans

- Plan status: `plans/README.md`, the single source of truth for plan state.
- Parity gap inventory: `plans/parity-gap-register.md`.
- Feature roadmap and queued work: `ROADMAP.md`.

### Core priorities

1. Performance and Reliability first.
2. Keep behavior predictable under load and during failures (session restarts, reconnects, partial streams).

If a tradeoff is required, choose correctness and robustness over short-term convenience.

### Task completion requirements

- All of `pnpm format`, `pnpm lint`, and `pnpm typecheck` must pass before considering tasks completed.
- Rust changes are complete only after `just fmt`, `just lint`, and `just test` run successfully.
- Add focused tests for meaningful behavior and failure modes. Avoid tests that merely mirror implementation or assert feature deletion.
- After required checks pass, repeat or expand verification only for changed code, failures, or unresolved concerns. Report blocked checks honestly.
