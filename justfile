tauri_manifest := "src-tauri/Cargo.toml"

default:
    @just --list

lint: lint-core
    cargo clippy --manifest-path {{tauri_manifest}} --all-targets -- -D warnings

# The backend without the Tauri host: nothing below the command layer may
# depend on Tauri (ADR-0032).
lint-core:
    cargo clippy --manifest-path {{tauri_manifest}} --no-default-features --all-targets -- -D warnings

test: test-core
    cargo test --manifest-path {{tauri_manifest}}

test-core:
    cargo test --manifest-path {{tauri_manifest}} --no-default-features

build:
    cargo build --manifest-path {{tauri_manifest}}

fmt:
    cargo fmt --manifest-path {{tauri_manifest}}

# Native target is macOS-only and uses the pinned Rust/Zed graph in its workspace.
fmt-native:
    cd apps/native && cargo +1.98.1 fmt --check

lint-native:
    cd apps/native && cargo +1.98.1 clippy --locked --all-targets -- -D warnings
    cd apps/native && cargo +1.98.1 clippy --locked --features fixture-verification --all-targets -- -D warnings

test-native:
    cd apps/native && cargo +1.98.1 test --locked
    python3 -m unittest discover -s tools/native -p 'test_*.py'

check-native: fmt-native lint-native test-native check-native-backend
    cd apps/native && cargo +1.98.1 build --locked
    cd apps/native && cargo +1.98.1 clippy --release --locked --all-targets -- -D warnings
    cd apps/native && cargo +1.98.1 test --release --locked
    cd apps/native && cargo +1.98.1 build --release --locked
    python3 tools/native/dependencies.py

# Opt-in facade remains covered independently from the default Tauri adapter.
check-native-backend:
    cargo clippy --manifest-path {{tauri_manifest}} --no-default-features --features isolated-profile --all-targets -- -D warnings
    cargo test --manifest-path {{tauri_manifest}} --no-default-features --features isolated-profile
    cargo test --manifest-path {{tauri_manifest}} --features isolated-profile backend::

test-native-live:
    python3 tools/native/fixture.py check
    cd apps/native && DBUNK_NATIVE_FIXTURE_VERIFIED=1 cargo +1.98.1 test --locked live_tests -- --ignored --test-threads=1

# Includes native checks on macOS without breaking the backend's Linux checks.
check-all: fmt lint test check-native

native-fixture-up:
    python3 tools/native/fixture.py up

native-fixture-down:
    python3 tools/native/fixture.py down

dev-native:
    python3 tools/native/launch.py

test-native-e2e:
    python3 tools/native/launch.py --verify

# Serial foreground probes; each output directory must be new.
test-native-window-races out:
    python3 tools/native/verify_window.py races --out {{quote(out)}}

measure-native out:
    python3 tools/native/verify_window.py performance --out {{quote(out)}}
