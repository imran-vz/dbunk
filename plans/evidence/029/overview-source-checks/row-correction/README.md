# Overview row-label correction

Changed only `apps/native/src/overview_view/render.rs`: the fixed-height Overview rows now use the existing GPUI `.truncate()` convention, which combines overflow clipping, no wrapping, and ellipsis. Full accessible labels and selected details remain unchanged. This corrects the observed second-line clipping in the previous package; visual verification of this corrected binary remains pending root’s window recheck.

All commands used `CARGO_INCREMENTAL=0`. In `apps/native`:

```sh
cargo +1.98.1 fmt --check
cargo +1.98.1 clippy --locked --all-targets -- -D warnings
cargo +1.98.1 test --locked -- --test-threads=1
```

All passed. Tests: 322 passed, 13 ignored, 0 failed. Logs and exact command results are in `checks.json` and corresponding text files. No additional behavioral test was added for this single style correction.

From the repository root:

```sh
CARGO_INCREMENTAL=0 python3 tools/native/package.py --out /private/tmp/dbunk-native-package-20261003-overview-rows
```

Packaging and its dependency check passed. No `--verify` or application launch was performed. Bundle:

`/private/tmp/dbunk-native-package-20261003-overview-rows/dbunk Native Preflight.app`

Executable SHA-256:

`ecebb91db413d6772322c8c3e96832e1909a32a7945be97e80bebf4b026acab3`

`source-sha256.json` captures the same 594-file set as the preceding Overview freeze. Before this build, only the row rendering file differed from that freeze. All 594 hashes matched after packaging; `package.json` records this comparison. `bundle-identity.json` records the packaged resources and binary hashes. New undeclared modules from subsequent work are outside this preceding source set and were not activated during the build.
