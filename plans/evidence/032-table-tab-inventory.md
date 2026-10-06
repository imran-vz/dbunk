# Inventory: current PostgreSQL table tab (dbunk native)

Paths relative to /Users/imran/projects/Code/dbunk. Line numbers as of commit 543595c.

## 1. Sidebar double-click → table tab
- navigator_view.rs:772-786 click_count()>1 → activate_row (:372-399) emits NavigatorEvent::OpenTable{connection,schema,table} → workspace.rs:367-384 → open_table_on (:1302) → open_table_filtered (:1312-1376): WorkspaceDocument with table: Some(WorkspaceTableState{schema,table,filters,sort:[],page_size:100,draft:None}), begin_connect. 16-tab cap.
- document_view.rs:272-274: document.table.is_some() → TableView::new (table_view.rs:183).
- Tab bar: workspace_shell.rs:1231-1420 (tab_bar) with 2px env-coloured bar on active tab. Production strip :1714-1742 keys off the sidebar's selected connection, not the tab's connection.
- PG TableView has no Data/Structure sub-tabs; Structure etc. are toolbar buttons emitting TableEvent::OpenStructure (separate tab). MySQL/ClickHouse/SQLite data tabs have Data/Structure segment buttons (mysql_lane/document.rs:611-673, clickhouse/document.rs:609-650, sqlite_documents.rs:1105-1118).
- impl Render for TableView (table_view.rs:1727-2082) stacks: toolbar (:1754-1805, crumbs + 19 text buttons: Connect, Backup, Restore, Import CSV, Export CSV, Export table, Copy table, Seed table, Structure, Refresh, Edit cell, Insert row, Duplicate row, Bulk edit, Delete row, Follow foreign key, Count, Sort selected column, Cancel); columns row (:1806-1828, 10 buttons Narrow/Widen/Auto-fit/Auto-fit visible/Move left/right/Pin/Hide/Show all/Reload preferences); preferences_status (:2036-2048); BrowseControls (browse_controls.rs:399-736: filter row, active filter/sort list, history/preset/inspect row, summary, message, inspection SQL); grid (:2050 flex_1; grid adds own 24px status/export row grid.rs:1052-1100); FK reference/detail panel (:1831-1923); TableChanges panel (table_changes/view.rs:88-591, always shows Virtual key Choose/Clear/Reload :152-272); footer status_line (:2053-2081 First/Prev/Next/Last, PAGE_SIZES=[10,25,50,100,250,500,1000] buttons, summary, staged count, status, latency).
- ui::toolbar() (ui.rs:14-28) uses flex_wrap → everything wraps into ~6 rows. Click-to-cycle column/operator pickers (browse_controls.rs:438-457).
- Target mock: plans/mocks/native-redesign/index.html:533-545 — one 28px toolbar: crumbs · Filter · Sort · Columns 8/8 · grow · +Row · Export · primary "Review 1 change ⌘S", grid, 24px footer. Grid CSS :155-170.

## 2. Grid
- grid.rs (1116 lines), grid/render.rs, grid/frozen.rs, grid/keyboard.rs, grid/navigation.rs, grid_columns.rs, column_widths.rs.
- ResultGrid grid.rs:74; new_table (:125) sets sortable, sql_target; table_page(Rc<BrowseTableResult>) (:386); GridEvent{Sort{column,append}, Preferences(PreferencePatch)} (:95-98).
- Rows virtualized: uniform_list("rows") grid.rs:1103-1111. Columns: GridColumns::visible_range (grid_columns.rs:112); columns_in/pinned_columns/scrolling_columns (grid.rs:559-587). Frozen pane grid/frozen.rs Panes. Row-number gutter render.rs:58.
- header_cell render.rs:71-130 (name + faint mono type). row_cell :205-265 (cell_kind :18 colouring; selected = select fill + accent border). No staged/edited tint (DESIGN.md:57 says edited cells warn @14%). No sort indicator.
- Widths: column_widths::fit (column_widths.rs:41) INITIAL_MAX 400, AUTO_FIT_MAX 500, MIN_WIDTH 60. Prefs columnWidths[name] clamp 48–1200 in GridColumns::rebuild (grid_columns.rs:165-185), DEFAULT_WIDTH 160. Resize only ColumnAction::Narrow/Widen ±32 (grid_columns.rs:269-271) and auto_fit (grid.rs:176-250) → PreferencePatch::Column/AutoFit; width applied only after SQLite prefs save round-trip (TableView::save_preferences table_view.rs:610). NO drag handle. Hide/order/pin exist (ColumnAction grid_columns.rs:11).
- Selection: anchor/head (GridView grid.rs:56), click/shift-click (render.rs:257-263), arrows, Home/End/PgUp/PgDn (grid/keyboard.rs:38), ⌘A, shift-space, ⌘G, space/shift-enter inspect. Keymap main.rs:201-262. No double-click on cells, no Enter-to-edit.
- Editing only via toolbar Action::Edit (table_view.rs:1032-1037) → TableChanges::edit_cell (table_changes.rs:521); bottom-panel Zed Editor (table_changes/view.rs:273-424) Stage ⌘↵ / Esc / Use NULL / Raw literal / Pretty JSON. cell_value::classify Kind::{Json,Array,Geometry} (cell_value.rs:12-20), table_changes/array_view.rs.
- Sorting: header mouse-down → GridEvent::Sort (shift appends) → table_view.rs:247-259 → TableDocument::cycle_sort (data_model/browse.rs:173) asc→desc→removed → apply_browse. Also toolbar "Sort selected column" (table_view.rs:1066) and BrowseControls sort rows (browse_controls.rs:257-293).
- Filtering: BrowseControls Typed vs WHERE mode, 13 OPERATORS (:43), build_filter (:778-839) → BrowseFilter {Comparison, TextMatch, IsNull, IsNotNull, InList, RawSql} (backend/src/table_browse/protocol.rs:29-53). History, presets, Inspect query. State: BrowseState/FilterMode/PreferencePatch (browse_preferences.rs:12-207).
- Backend SQL: backend/src/table_browse/builder.rs build_browse_query (:125-205) WHERE/ORDER BY/LIMIT n+1/OFFSET or keyset; render_filter (:250) binds ($N::text)::<cast>, ILIKE escape_like; render_order (:374) identity tiebreakers; count/explain :102-120.
- Pagination: PageAction (data_model/browse.rs:27), TableDocument::browse (:231) → BrowsePageRequest::{Offset,Keyset}; BrowseCountPolicy estimated + exact Count.
- Other engines (MySQL/SQLite/ClickHouse) use ResultGrid in query mode, sortable=false, no filter/edit. Table Browse and Result Mutation are PG-only.

## 3. Write path
- data_model/mutations.rs: MutationDraft (:55) stage_update (:347), stage_delete (:436), stage_insert (:463), edit_value (:297), include/remove. Limits 128 changes / 4 MiB (data_model.rs:10-14). capture (:158) identity+guards. review() (:501) → ReviewPlan; begin_apply (:524) revision fence; finish_apply (:538) removes only if each op affected exactly 1 row; connection loss/timeout → outcome_unknown.
- table_changes.rs: request_review (:967) → TableCommand::Review → backend MutationReview → panel "Review the SQL and bound values" (:1230-1257). prepare_apply (:1017) ApplyFlow (apply_flow.rs) persists recovery journal (PersistApply) → apply_saved (:1048) TableCommand::Apply. Backend MutationSubmission::NeedsConfirmation → confirming (:1265-1278); "Confirm changes" (Action::Confirm :1378-1390) re-persists → TableCommand::Confirm(MutationConfirmation). Commands table_runtime.rs:92-151.
- Preview UI table_changes/view.rs:522-562: raw statement.sql + JSON params mono, max h 160. change_summary (:594) only "Update schema.table col1 col2" — no old→new values.
- Backend builder backend/src/result_mutation/builder.rs: build_update (:112) `UPDATE "s"."t" SET "c" = ($1::text)::type WHERE <identity AND guards>`, build_delete (:151), build_insert (:171) DEFAULT VALUES fallback, render_predicate (:344).
- Transaction result_mutation/postgres.rs:1036-1120: BEGIN, lock timeout, lock_and_refresh_for_apply, rebuild plan, execute each; 0 rows → Conflict, >1 → IdentityNotUnique, rollback; COMMIT failure → unknown.
- ResultMutationError (protocol.rs:381-430): Conflict, IdentityNotUnique, LockTimeout, PolicyBlocked, PolicyNeedsConfirmation, Database{op_index}... Native shows `{error:?}`.
- MutationIdentityKind {PrimaryKey, UniqueIndex, VirtualKey, CtidFallback, None} (protocol.rs:91).

## 4. Environment and safety
backend/src/types.rs:36-111:
```rust
pub(crate) enum Environment { #[default] Development, Test, Staging, Production }
pub(crate) enum SafeMode { #[default] Inherit, Disabled, Protected, Strict }
pub(crate) struct ConnectionPolicy { environment, safe_mode, read_only: bool }
```
- Host mirrors DevelopmentEnvironment/DevelopmentSafeMode (backend/src/backend/development/connections.rs:22-38); DevelopmentPostgresConnection.environment/safe_mode/read_only (:80-82); DevelopmentConnection.environment (:189).
- backend/src/safety/policy.rs resolve_policy (:81-103): Inherit → Dev/Test Disabled, Staging Protected, Production Strict. assert_permitted (:105). ApplyMutations/RowMutation need confirmation only under Strict (:163-194). Read-only blocks all writes. safety/gate.rs legacy tags + record_override audit.
- Enforced result_mutation/mod.rs:254-272 start_apply → assert_permitted(&spec.safety_policy, …, payload.confirmed). Token MutationConfirmation (backend/src/backend/data/mutations.rs:39-60).
- Settings UI forms.rs:51,95-98,284-294,706-724 env chips, FormAction::Safe, read-only toggle; new connections default safe: Protected (:132).
- style::env colours, env_label, ENVIRONMENTS (style.rs:196-223). workspace_shell.rs:828 env_color frame tint.
- Confirm UIs to borrow: workbench.rs:2258-2276 "Safe Mode requires confirmation" + SQL + "Confirm and run"; sqlite_documents.rs:470-490; redis_view.rs:1040-1065. No native typed-confirm component exists.
- TableView/TableChanges do not receive the connection environment / safe mode.

## 5. Styling
- Tokens apps/native/src/style.rs:8-128: bg #0c0d0f, panel, raised, hover, pressed, select #22324a, line, line_soft, text/dim/faint, accent #6aa6ff, ok/warn/bad, primary_*, bad_*, ok_fill, warn_fill, row_hover, number, boolean. ROW 20, TOOLBAR 28, FOOTER 24, TOOL 20, BAR 34; MONO ".ZedMono"; font 11/10.
- ui.rs kit: toolbar (14), tool_button (33), pressed (97), segmented/segment (108/124), separator, grow, badge (162), crumbs (176), status_line (189), field (216), press (231), Variant{Primary,Secondary,Ghost,Danger} button (257/270), appear, shake, error_banner (366), tooltip (453), input_frame (466), labelled (483), section (515), check_box (529), choice_card (561).
- Popover/menu: workspace_shell.rs:1618-1706 menu(): panel, 8px radius, shadow_lg, anchored(), occluding backdrop. Dialog layer workspace.rs:2617-2627 (full-window). No in-document modal, no dropdown/select component.
- DESIGN.md: density first; one loud environment signal; ≤1 Primary button per page; Danger only for destructive confirmations; warn_fill for confirmations; popovers 8px radius; edited cells warn 14%; every control Tab-reachable with AX role/label; motion limited to appear/shake/tooltip.

## 6. Docs
- plans/README.md: Plan 028 (native PG data workflow, "Bottom review" mock A) and 031 (redesign) IN PROGRESS. Plan 028:10 "visual wheel/resize acceptance remains open".
- parity register PAR-002 browse (:155), PAR-003 mutations (:185), PAR-004 safety (:263-325).
- ADRs: 0014 specialized editors write into pending edits, 0022 table browse contract, 0023 staged result mutation, 0024 backend-enforced safety (UI confirmation is not a security boundary), 0032 host seam.
- CONTEXT.md: Environment, Production Identity (:102-109), Safe Mode, Safety Policy, Confirmed Override (:110-123), Row Identity, Result Mutation, Mutation Draft, Staged Change, Virtual Key (:213-241), Table Browse, Browse Filter, Grid Preferences (:312-338), DML Preview (:398).

## 7. Tests
- Native: pure model unit tests, no gpui::test. Inline #[cfg(test)] in grid/render.rs, grid/keyboard.rs, grid/frozen.rs, column_widths.rs, apply_flow.rs, table_view.rs:2120, browse_controls.rs:840, table_changes.rs:1592+; files grid_columns/pinning_tests.rs, browse_preferences/pinning_tests.rs, data_model_tests.rs (helpers document(), page(), analysis(), row(), value()), data_model/mutations/batch/tests.rs, table_changes/batch_edit/tests.rs, table_changes/retention/tests.rs, table_runtime_tests.rs, connection_settings_model/tests.rs.
- Backend: inline builder tests, */service_tests.rs, */native_tests.rs, safety/policy.rs + gate.rs tests, ignored live tests.

## 8. Running
- justfile: dev-native (tools/native/workspace_launch.py fresh-profile fixture), run-native, native-fixture-up/down, test-native, lint-native, fmt-native, check-native. cargo +1.98.1.
- Fixtures: tools/native/fixture.py postgres:17.6 on 127.0.0.1:15432/dbunk_demo; infrastructure/test-db postgres:16 15432 dbunk/dbunk. Docker currently not running.
