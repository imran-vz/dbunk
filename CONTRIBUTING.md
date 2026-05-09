# Contributing

Thanks for helping make dbunk better. Contributions of all sizes are welcome.

## Ways To Help

- Report reproducible bugs.
- Suggest focused improvements.
- Improve documentation.
- Add or refine tests.
- Fix UI issues.
- Improve database engine support.
- Review open pull requests.

## Development Setup

```bash
bun install
bun run dev
```

Before opening a pull request, run:

```bash
bun run test
bunx tsc --noEmit
bun run lint
```

## Pull Request Guidelines

- Keep changes focused.
- Include tests for behavior changes when practical.
- Update docs when user-facing behavior changes.
- Prefer existing patterns over new abstractions.
- Do not include unrelated formatting or generated churn.
- Explain the user impact in the PR description.

## Commit Style

There is no strict commit convention yet. Clear, imperative messages are best:

```text
Add column completions for SQL where clauses
Fix table pagination next button state
Document local development setup
```

## Reporting Bugs

Please include:

- What you expected to happen.
- What actually happened.
- Steps to reproduce.
- Database engine and version, if relevant.
- Screenshots or logs, if useful.

## Feature Requests

Please describe the workflow you want to support. A concrete example is usually more helpful than a broad feature name.

## Code Of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). Be kind, specific, and constructive.
