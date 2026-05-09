# dbunk

dbunk is an open-source desktop database workspace for exploring data, running SQL, inspecting schema structure, and editing table data from a fast local app.

It is built with Tauri, React, TypeScript, Rust, Monaco Editor, SQLx, and Bun.

## Status

dbunk is early and moving quickly. Expect rough edges, but contributions are welcome: bug reports, feature ideas, docs improvements, design polish, database engine support, tests, and small fixes are all useful.

## Features

- Manage database connections from a desktop app.
- Browse schemas, tables, and views.
- Run SQL queries with a Monaco-powered editor.
- Use SQL IntelliSense for syntax, tables, views, and columns.
- Execute the current query, a selection, or the whole editor.
- Preview query results in a data grid.
- Inspect table structure, columns, indexes, constraints, and relationships.
- Edit table data where the backend can identify rows safely.
- Export result data.

## Supported Databases

The app currently has support paths for:

- PostgreSQL
- MySQL
- SQLite
- ClickHouse

Some advanced features are engine-specific. PostgreSQL currently has the richest editing and schema-inspection support.

## Getting Started

### Requirements

- [Bun](https://bun.sh/)
- [Rust](https://www.rust-lang.org/tools/install)
- System dependencies required by [Tauri](https://tauri.app/start/prerequisites/)

### Install

```bash
bun install
```

### Run The App

```bash
bun run dev
```

### Run The Web UI Only

```bash
bun run dev:vite
```

### Test And Check

```bash
bun run test
bunx tsc --noEmit
bun run lint
```

### Build

```bash
bun run build
```

## Project Structure

- `src/` - React app, UI components, client store, SQL helpers, tests.
- `src-tauri/` - Tauri app shell and Rust backend commands.
- `docs/` - project and agent-facing documentation.

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

Good first contributions include:

- Reproducing and filing bugs.
- Improving README/docs.
- Adding focused tests for SQL editor behavior.
- Fixing UI polish issues.
- Improving database-specific schema support.
- Making error messages clearer.

## Security

Please do not open public issues for security vulnerabilities. See [SECURITY.md](SECURITY.md).

## License

dbunk is released under the [MIT License](LICENSE).
