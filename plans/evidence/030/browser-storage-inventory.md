# Baseline browser storage inventory (R02), 2026-10-03

Source: Tauri baseline `102568b`, read with `git show`/`git grep` only. No
WebKit storage, daily-driver profile or Keychain entry was opened.

## How the baseline persisted UI state

`102568b:src/lib/ui-state.ts` is the single persisted-UI read/write point. In the
desktop app it stores values in the profile SQLite database under `ui.v1.*`
(caller key `dbunk.x` maps to `ui.v1.x`). At startup a one-shot migration copies
every `dbunk.*` WebKit `localStorage` key into SQLite, records `ui.v1.migrated`,
and removes the browser copy only after the SQLite batch commits. Two exceptions
stay in browser storage:

- **Boot-cache keys**, read by the pre-paint script: `dbunk.theme`,
  `dbunk.theme.preset`, `dbunk.density`.
- **Unpersistable values**: a value over 512 KiB (or a key over 512 bytes) keeps
  its only copy in `localStorage`.

Dead keys `dbunk.workbench.dock.*` and `dbunk.sidebar.global*` are dropped.

## Inventory

| Key | Canonical store at baseline | Browser-only? | Native disposition |
| --- | --- | --- | --- |
| `dbunk.theme` | SQLite app setting `theme` (`commands/settings.rs`) | No, boot-cache mirror | Import from SQLite setting; native appearance stays true-black/white (documented difference). Preserve the value. |
| `dbunk.theme.preset` | SQLite app setting `theme_preset` | No, boot-cache mirror | Same as theme. |
| `dbunk.density` | `localStorage` only (`src/lib/density.ts`: "localStorage for now") | **Yes** | Needs an explicit one-time export/import path. Baseline modes are `compact`/`default`/`comfortable`; native has Compact/Comfortable, so `default` needs a stated mapping. |
| Other `dbunk.*` (session, panels, grid layouts, palette frecency, rail, query split, executeSelection/CurrentQuery/All, …) | SQLite `ui.v1.*` after migration | Only if migration never ran (pre-P8 profile) or a value was unpersistable | Copy-based importer reads `ui.v1.*` from the SQLite snapshot (R01/R03/R04). A profile without `ui.v1.migrated` must be reported as possibly holding browser-only state. |

## Consequences

- A SQLite-only import is sufficient for everything except density, pre-P8
  profiles and oversized values. Those cases must be disclosed, not claimed as
  preserved.
- The browser-only path must be explicit and user-initiated (for example an
  exported JSON of the three boot-cache keys). Reading the real WebKit
  `LocalStorage` database of the daily-driver app is outside this authorization.

Status: inventory complete; import adapter not implemented. R02 remains
not implemented in the [capability ledger](./capability-ledger.md).
