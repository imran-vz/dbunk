# Local verification

Automated CI is disabled for this project. There are no self-hosted runner
requirements. Run the applicable checks on the device where development happens,
before opening or merging a pull request. Keep the existing checks and tests;
removing CI does not waive verification.

Run from the repository root:

```sh
just fmt
just lint
just test
# Native app checks require macOS and the pinned Rust toolchain.
just check-native
# For a full backend + native gate:
just check-all
```

Use the owned fixtures and native verification commands in `justfile` for changes involving databases, live integration, packaging, or UI behaviour. Never substitute a production database. The tag-triggered Release workflow is separate from CI and remains available.

Record the commands run and their results in the pull request. State any skipped
checks, unavailable fixtures, or untested platforms explicitly.
