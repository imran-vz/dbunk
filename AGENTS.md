### Issue tracker

Issues are tracked in GitHub Issues for `imran-vz/dbunk` using the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Domain docs

This repo uses a single-context domain docs layout. See `docs/agents/domain.md`.

### Status and plans

- Plan status: `plans/README.md` — the single source of truth for plan state.
- Parity gap inventory: `plans/parity-gap-register.md`.
- Feature roadmap and queued work: `ROADMAP.md`.

Core Priorities

1. Performance and Reliability first.
2. Keep behavior predictable under load and during failures (session restarts, reconnects, partial streams).

If a tradeoff is required, choose correctness and robustness over short-term convenience.

Task Completion Requirements
- All of pnpm format, pnpm lint, and pnpm typecheck must pass before considering tasks completed.
- Rust changes are complete only after `just fmt`, `just lint`, and `just test` run successfully.
