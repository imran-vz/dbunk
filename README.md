# dbunk

dbunk is an open-source native database workspace for macOS: browse data, run SQL, inspect and change schemas, and operate databases from one fast, keyboard-friendly window.

The app is built with Rust and [GPUI](https://www.gpui.rs/) (Zed's UI framework, including Zed's editor). The earlier Tauri/React app was retired in favour of this native app.

## Status

dbunk is **pre-alpha** and under heavy development. Expect rough edges, missing features, and breaking changes between releases. Do not point it at production databases yet.

PostgreSQL has the most complete native support today. MySQL, SQLite, ClickHouse and Redis support exists in the backend and is being brought into the native app.

## Install (pre-alpha)

Release builds are published on the [GitHub Releases page](https://github.com/imran-vz/dbunk/releases) as an unsigned **Apple Silicon (arm64) DMG**. Right-click the app → **Open** on first launch, or run `xattr -dr com.apple.quarantine` on it.

## Build from source

Requirements: macOS on Apple Silicon, [Rust](https://www.rust-lang.org/tools/install) with the pinned `1.98.1` toolchain, [just](https://github.com/casey/just), Python 3, and `cmake` (needed by a Zed dependency).

```bash
just run-native                 # build and open your default profile
just dev-native                 # workspace window against the owned fixture, fresh profile
just package-native /tmp/dbunk  # unsigned app bundle in a new directory
```

Checks:

```bash
just fmt && just lint && just test                       # backend
just fmt-native && just lint-native && just test-native  # native app
```

## Project structure

- `apps/native/` – the GPUI desktop app.
- `backend/` – the Rust backend library: engines, storage, credentials, query sessions and services.
- `tools/native/` – packaging, fixtures and verification helpers.
- `infrastructure/test-db/` – disposable database containers for tests.
- `docs/`, `plans/` – architecture decisions, plans and evidence.

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a pull request.

## Security

Please do not open public issues for security vulnerabilities. See [SECURITY.md](SECURITY.md).

## License

The backend library is MIT licensed ([LICENSE](LICENSE)). The native app links Zed's GPL-licensed editor and is distributed under GPL-3.0-or-later (see `apps/native/Cargo.toml` and `apps/native/THIRD-PARTY-NOTICES.txt`).
