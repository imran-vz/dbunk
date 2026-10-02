# Plan 025 implementation review for stage 03

2026-10-02. Baseline `102568b8003461c9821fb84fc0957ccd3e4d0b13` plus
the existing uncommitted implementation. Review covers tracked diffs and new
host/service files, not merely `HEAD` versus `HEAD`. Existing work is preserved.
No commit, push or PR. Plan 025 remains separately reviewable.

## Conclusion

No confirmed extraction regression requiring a source repair was found.
Hydration, engine checks, policy resolution, pre-dispatch refusal, successful
override auditing, event JSON, sequence, cumulative ACK and credit semantics
remain behind the same services. Connection service extraction is a documented
dependency of the core-only safety tests, not a request to extract more families.

The review corrected the evidence's lifecycle interpretation in Plan 025 and
ADR-0032. It does not claim that the future native shutdown barrier exists.
Stage 03 is [Plan 026](../../026-native-postgres-workflow.md).

## Standards

Independent read-only standards review: no introduced correctness or
maintainability defect. Three inherited native integration obligations:

- `query_session/mod.rs`, `close_window`: registered sessions close, but the
  owner and pending opens remain. Retire native admission/owner before waiting
  for a delayed connection, then assert no late SessionState or backend.
- `app.rs`, exit cleanup: `close_all` permits later opens; a three-second
  timeout does not establish task termination. Native shutdown needs a fence
  and tracked completion, preserving Tauri behavior.
- `app.rs`, monitor startup: detached monitors retain manager clones.
  Dropping backend state alone is insufficient; native runtime/monitor
  lifetime must be explicit.

Standards: zero extraction violations; three inherited lifecycle obligations.

## Spec

Independent read-only spec review: no extraction regression, undocumented
scope expansion or safety/event drift. Three inherited stage03 obligations:

- Draft requirement: “Closing the window must cancel work and await backend
  cleanup.” Execution handles currently detach. The global-teardown test
  asserts logical closure and subsequently polls for database disappearance;
  it is not a joined native shutdown barrier.
- Window closure does not fence an opening session. A connect completing
  after `close_window` can still pass the unchanged owner check. Test with a
  controlled delayed handshake, close, then release it.
- Draft requires Stop under a “slow or full consumer queue.” Cancellation
  deliberately does not release credit. Native Stop must keep consuming and
  ACKing, or explicitly retire a failed stream. A failed/full queue also needs
  an independent local failure notification.

Spec: zero extraction defects; three stage03 lifecycle/delivery obligations.
These overlap the standards findings and are not six distinct regressions.

## Fresh verification

Run by the primary agent against the preserved tree on 2026-10-02:

| Check | Result |
| --- | --- |
| `pnpm format` | PASS |
| `pnpm lint` | PASS |
| `pnpm typecheck` | PASS |
| `pnpm test` | PASS: 130 files, 1,488 tests |
| `just fmt` | PASS |
| `just lint` | PASS: default and `--no-default-features` |
| `just test` | PASS: default 657 passed / 85 ignored; core 633 passed / 70 ignored |
| Core normal dependency graph | PASS: no Tauri, Wry, Tao or WebKit dependency |

Logs from this review are retained alongside this record. No source change
was needed, so there are no added implementation-mirroring tests. Formatting
did not alter the verified spike adapter or its lockfile.

## Limits of this review

Live PostgreSQL tests were **not rerun**. Docker could not connect to the
configured OrbStack socket, and the available libpq installation has client
tools but no PostgreSQL server. No unidentified service was contacted, no
profile/keychain accessed, and no database was started or reset. The 17 actor
tests and two live safety tests reported in Plan 025 remain historical evidence.

The custom-protocol build, native release checks and external AX probe were
inspected in the saved Plan 024/025 evidence, not rerun for this planning-only
turn. No native UI or editor source changed. Plan 026 requires fresh native
checks, actual fixture E2E and the adapted AX probe after implementation;
historical synthetic results do not satisfy those gates.
