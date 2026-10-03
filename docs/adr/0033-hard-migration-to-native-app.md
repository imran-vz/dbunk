# 0033. Hard migration to the native GPUI app

Date: 2026-10-03. Status: accepted.

## Context

Plans 024–030 migrated dbunk to a native GPUI host gradually, keeping the
Tauri/React app as the daily driver and the parity baseline (`102568b`). The
host-neutral backend seam (ADR-0032) made the backend buildable without Tauri.
The app has effectively no users, and maintaining two hosts slowed the native
work. The native app's UI also needs a new design.

## Decision

Retire the Tauri/React app now instead of after full parity:

- Delete the React frontend (`src/`), the JS toolchain and configs, the Tauri
  command adapters, host, config and capabilities, and the `tauri-host` feature.
- Keep the backend crate as the native app's library and move it from
  `src-tauri/` to `backend/`. Its `isolated-profile` feature still exposes the
  native facade.
- The native app (`apps/native`) is the product. CI and releases build and
  package it only.
- Engine code for MySQL, SQLite, ClickHouse and Redis stays in the backend for
  the native app to adopt; until then those engines are unavailable to users.
- A new compact design replaces the previous native layout (see
  `plans/mocks/native-redesign/`).

## Consequences

- The Tauri baseline is now historical: `102568b` and Plans 024–030 remain the
  record of what the old app did, not a running app to compare against.
- Tests that only drove Tauri commands were removed with them. Engine code with
  no native caller is allowed to be unused until it is adopted.
- The completion gates are the backend `just fmt/lint/test` and the native
  `just fmt-native/lint-native/test-native`; pnpm checks no longer exist.
