# Plan 024 Step 6: parity checklist against `102568b`

What the native app has to do before it can replace the Tauri app. Sources:
the command list registered in `src-tauri/src/tauri_host.rs` (174 commands
plus the window-chrome command), the component tree under `src/components/`,
the accepted ADRs and `plans/parity-gap-register.md`.

Parity means the behavior at the baseline, including its limits. Items the
gap register already lists as partial or missing stay that way; the native
app does not close them, and does not regress them.

**Native status** is what exists after stage 01: **spiked** (shown working in
the Plan 024 spike, with synthetic data), **core ready** (the host-neutral
service exists after Plan 025), or blank (nothing yet). Nothing is built.

## 1. Shell and workspace

| Item | Baseline source | Native status |
| --- | --- | --- |
| Window chrome, traffic-light inset, title bar | `tauri_host.rs`, `app-shell` | |
| Left rail destinations and view switching | `app-shell`, `workbench` | |
| Workspace tabs: open, close, reorder, dirty marker | `workspace-tabs` slice | |
| Session restoration, hot-exit drafts, caret positions | `session-persistence.ts`, ADR-0003 | |
| Byte-budgeted UI state | `load_ui_state`, `save_ui_state`, `delete_ui_state` | |
| Command palette and Open Anything | `command-palette`, Plan 010 | |
| Status bar | `status-bar.tsx` | |
| Themes and density | `DESIGN-SYSTEM.md`, `density.ts` | |
| Toasts | `themed-toaster.tsx` | |
| Keyboard shortcuts | scattered; Monaco actions for the editor | |
| Window geometry across launches | `tauri-plugin-window-state` | |
| Logging to a file in release builds | `tauri-plugin-log` | |
| Exit cleanup of every socket-owning manager | `app.rs`, `tauri_host.rs` | core ready |

## 2. Connections and credentials

| Item | Baseline source | Native status |
| --- | --- | --- |
| Credential onboarding: keychain, plain SQLite, encrypted SQLite | `credential-onboarding.tsx`, ADR-0005, ADR-0007 | |
| Unlock, change mode, reset | `commands/settings.rs` | |
| Connection list, favorites, folders, colors | `connections-view`, Plan 010 | core ready |
| Create, edit, duplicate, delete | `connection-form`, `connections.rs` | core ready |
| Per-engine connection form with policy union | ADR-0011, ADR-0012 | |
| TLS controls and staged diagnosis | ADR-0025, `commands/diagnosis.rs` | |
| Environment, safe mode, read-only | ADR-0024 | |
| Bastion servers and SSH tunnels | ADR-0018, `bastion-servers` | |
| Managed Docker servers | ADR-0019, `managed-servers-tab.tsx` | |
| Health-check tick and last activity | ADR-0002, ADR-0004 | core ready |
| New local SQLite database | `new-local-database-dialog.tsx` | |

## 3. SQL editor and query execution

| Item | Baseline source | Native status |
| --- | --- | --- |
| Editing: selection, undo, multi-cursor, wrap, find | Monaco | spiked (Zed's editor) |
| SQL highlighting | Monaco monarch | spiked (Tree-sitter grammar) |
| Completion from the connection's schema | editor helpers | spiked (fixed list) |
| Current statement, selection and all | `use-monaco-query-editor.ts` | spiked (current statement) |
| Format SQL | `sql-formatter` | |
| Snippets | toolbar | |
| Bind variables (literal substitution today) | `bind-variables.ts` | |
| Driver-bound parameters and row limit (dark) | Plan 023, ADR-0031 | core ready; UI to be built natively once |
| Query Session: open, execute, cancel, ack, heartbeat | ADR-0021, ADR-0031 | core ready |
| Transaction mode, isolation, commit, rollback, recheck | `transaction-controls.tsx` | core ready |
| Streaming results, partial results, notices, truncation reasons | `query-session-channel.ts` | spiked (synthetic stream, same caps) |
| Legacy `run_query` path for the other engines | `commands/relational.rs` | |
| Result grid: virtual rows and columns, resize, selection, copy | `data-grid` | spiked |
| Value inspection and large values | `data-grid` | |
| Multiple result sets | `results-view.tsx` | |
| Export results (CSV, XLSX and the other formats of ADR-0017) | `xlsx.rs`, frontend exporters | |
| EXPLAIN and its viewer | `query-editor/explain` | |
| Query history and saved queries | `query-sidebar.tsx`, ADR-0003 | |
| Safety confirmation before a write | `safety-confirm-dialog.tsx`, ADR-0024 | core ready |

## 4. Tables and data

| Item | Baseline source | Native status |
| --- | --- | --- |
| Table Browse: paging, sort, filter, count | ADR-0022, `table-editor` | |
| Grid preferences per table | `load_table_grid_prefs` | |
| Staged mutations and review | ADR-0023, `mutation-review` | |
| Specialized cell editors | ADR-0014 | spiked (one editor, staged value) |
| Virtual keys and identity-safe edits | `result_mutation` | |
| Insert, delete, import, copy rows, seed | `commands/relational.rs`, ADR-0020 | |
| Foreign-key drill-down | `relationship-detail-popover.tsx` | |
| Table structure view | `table-structure` | |

## 5. Schema and objects

| Item | Baseline source | Native status |
| --- | --- | --- |
| Database navigator tree with filter | `query-sidebar`, `workbench` | |
| Object catalog and viewers | ADR-0026, `object-viewer` | |
| Object DDL workflow with preview and drop impact | `object-ddl` | |
| Table designer | ADR-0027, `table-designer.tsx` | |
| Routines, triggers, policies, privileges | ADR-0027 | |
| Schema relationship map: layout, saved positions, preferences | `schema-relationship-map` (React Flow) | spiked (pan, zoom, drag, edges) |
| Schema comparison | ADR-0030, `pg-schema-compare` | |
| DDL export | `export_ddl` | |

## 6. Jobs and files

| Item | Baseline source | Native status |
| --- | --- | --- |
| Backup and restore with `pg_dump` / `pg_restore` | ADR-0028, `pg-tool-jobs` | |
| CSV import and export | ADR-0029, `pg-transfer` | |
| Job list, progress, cancel, release | `pg-tool-jobs`, `pg-transfer` | |
| Native file pickers | `tauri-plugin-dialog` | |

## 7. PostgreSQL administration

| Item | Baseline source | Native status |
| --- | --- | --- |
| Database overview and relation statistics | `workspace-overview` | |
| Server details and admin snapshot | `commands/relational.rs` | |
| Cancel and terminate a backend | `cancel_pg_backend`, `terminate_pg_backend` | |
| Maintenance and materialized-view refresh | `run_pg_maintenance` | |
| Safety override audit list | `load_safety_overrides` | |

## 8. Other engines

| Item | Baseline source | Native status |
| --- | --- | --- |
| MySQL, SQLite: browse, query, structure, mutations | `dispatch/relational.rs` | |
| ClickHouse: read-only default, async mutations | ADR-0006 | |
| Redis keyspace, scan sessions, key inspection | `keyvalue` components, 9 inspection commands | |
| Redis typed value viewers and editors | 19 write commands | |
| Redis CLI with history and saved commands | `redis/cli.rs` | |
| Redis Pub/Sub with record to file | `redis/pubsub.rs` | core ready (sink) |
| Redis server info, clients, ACL, config, latency | 7 commands | |

## Existing limits that carry over unchanged

From the gap register: PAR-001, PAR-003, PAR-005 to PAR-011, PAR-013, PAR-014
and PAR-016 are partial; PAR-012 is missing; PAR-015 is deferred. The native
app inherits each as it stands. PAR-013 (desktop platform and release parity)
is the one the migration changes: Windows and Linux are later sequences, and
the proposal says so.

## Counts

- 8 areas, 75 items.
- 8 spiked in stage 01, all on synthetic data.
- 9 core ready after Plan 025: the Query Session family, connection
  services, exit cleanup and the pub/sub sink. Every other backend operation
  still has its logic inside a Tauri command.
- The rest is not started. That is the size of stages 04 to 06.
