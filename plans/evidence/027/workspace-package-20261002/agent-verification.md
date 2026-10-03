# Packaged credential UI checks

All four actual-window phases passed in the marked preflight bundle from an external working directory. `source-manifest.json` records the copied recovery-fixed binary's source snapshot before the separate Plan 028 backend extraction. Every probe verified bundle identifier `codes.imran.dbunk.native.stage04.preflight`, executable hash/path, launch arguments, fixture/profile identity, and the explicitly supplied credential namespace.

- `keychain-create`: fresh Keychain onboarding, fixture connection Test/Save, explicit connect, query result 42, acknowledged SQL draft, clean quit.
- `keychain-reopen`: exact disconnected draft restoration; blank-password edit Test resolves the stored Keychain secret without displaying it; blank-password Save preserves it; conversion to encrypted SQLite preserves the workspace.
- `encrypted-reopen`: wrong unlock gives an explicit error; correct unlock succeeds; a connection Test uses the preserved encrypted secret; conversion back to scoped Keychain succeeds.
- `keychain-reset`: explicit password-loss confirmation returns to onboarding while preserving exact SQL and connection metadata.

All four processes exited cleanly and returned the fixture backend count from zero to zero. `keychain-cleanup.log` confirms both entries in the announced namespace are absent. Its guarded OS adapter permits only those two identities and omits secret values.

These automated results do not claim human VoiceOver or IME acceptance, repeated overlapping mode-change races, or forced-process termination recovery.
