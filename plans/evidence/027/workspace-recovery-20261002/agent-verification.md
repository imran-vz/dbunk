# Recovery window checks

Final checks passed against the isolated profile `/private/tmp/dbunk-native-workspace-recovery-20261002`:

- `corrupt-quit-fixed` and `future-quit-fixed`: Cmd-Q closes normally without changing the original workspace bytes or revision. The fixed root receives keyboard focus even when restoration fails.
- `explicit-reset`: confirmed reset produces an empty workspace while preserving hashes of connection metadata, credentials, settings, and unrelated UI records.
- `oversize-complete`: a near-limit saved draft receives an exact unsaved tail, reports its failed save, blocks normal close, exports the entire current SQL through the native save dialog, then joins shutdown after explicit discard. `oversize-preserved-final.log` verifies the last durable SQL and all unrelated state remained unchanged. The exported SQL is `/private/tmp/dbunk-native-recovery-complete-20261002.sql`.
- `active-refusal.log`: the recovery helper refused to seed an open profile before any mutation.

Every completed launcher returned its owned fixture backend count to zero. Earlier directories retain diagnostic attempts: the first exposed missing keyboard focus in recovery; later attempts corrected CGEvent newline insertion and native save-panel automation. The final oversize check uses bounded text events and the save panel's AX filename value, and passes end to end.

The helper changes only `ui.v1.native.workspace` after validating private stage04 files, matching marker/SQLite identity, current fixture ownership, and the native exclusive profile lock. Receipts contain hashes rather than credential contents.
