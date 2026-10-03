backend_manifest := "backend/Cargo.toml"

default:
    @just --list

# The backend builds bare (engine core) and with the native host facade.
lint:
    cargo clippy --manifest-path {{backend_manifest}} --all-targets -- -D warnings
    cargo clippy --manifest-path {{backend_manifest}} --features isolated-profile --all-targets -- -D warnings

test:
    cargo test --manifest-path {{backend_manifest}}
    cargo test --manifest-path {{backend_manifest}} --features isolated-profile

build:
    cargo build --manifest-path {{backend_manifest}}

fmt:
    cargo fmt --manifest-path {{backend_manifest}}

# Native target is macOS-only and uses the pinned Rust/Zed graph in its workspace.
fmt-native:
    cd apps/native && cargo +1.98.1 fmt --check

lint-native:
    cd apps/native && cargo +1.98.1 clippy --locked --all-targets -- -D warnings
    cd apps/native && cargo +1.98.1 clippy --locked --features fixture-verification --all-targets -- -D warnings

test-native:
    cd apps/native && cargo +1.98.1 test --locked
    python3 -m unittest discover -s tools/native -p 'test_*.py'

check-native: fmt-native lint-native test-native
    cd apps/native && cargo +1.98.1 build --locked
    cd apps/native && cargo +1.98.1 clippy --release --locked --all-targets -- -D warnings
    cd apps/native && cargo +1.98.1 test --release --locked
    cd apps/native && cargo +1.98.1 build --release --locked
    python3 tools/native/dependencies.py

test-native-live:
    python3 tools/native/fixture.py check
    cd apps/native && DBUNK_NATIVE_FIXTURE_VERIFIED=1 cargo +1.98.1 test --locked live_tests -- --ignored --test-threads=1

check-all: fmt lint test check-native

native-fixture-up:
    python3 tools/native/fixture.py up

native-fixture-down:
    python3 tools/native/fixture.py down

# Workspace window against the owned fixture, on a fresh retained profile.
dev-native:
    python3 tools/native/workspace_launch.py

# Opens YOUR default profile (~/Library/Application Support/dbunk Native).
run-native:
    cd apps/native && cargo +1.98.1 run --release --locked

# Legacy stage03 single-query window; the AX probe below still drives it.
dev-native-stage03:
    python3 tools/native/launch.py

test-native-e2e:
    python3 tools/native/launch.py --verify

# Separate unsigned development bundle; output must be a new directory.
package-native out:
    python3 tools/native/package.py --out {{quote(out)}}

# Foreground AX probe, with the owned fixture and a fresh isolated profile.
test-native-bundle out:
    python3 tools/native/package.py --out {{quote(out)}} --verify

# Serial foreground probes; each output directory must be new.
test-native-window-races out:
    python3 tools/native/verify_window.py races --out {{quote(out)}}

measure-native out:
    python3 tools/native/verify_window.py performance --out {{quote(out)}}

# Persistent stage04 profiles; the helper verifies the owned fixture first.
native-profile-create path:
    python3 tools/native/profile.py create {{quote(path)}}

native-profile-check path:
    python3 tools/native/profile.py check {{quote(path)}}

# Headless public-facade acceptance, two processes and an owned Postgres fixture.
test-native-workspace path:
    python3 tools/native/workspace_probe.py {{quote(path)}}

# Persistent native workspace window; restores drafts without reconnecting.
dev-native-workspace path:
    python3 tools/native/workspace_launch.py {{quote(path)}}

native-tls-fixture-up:
    python3 tools/native/tls_fixture.py up

native-tls-fixture-down:
    python3 tools/native/tls_fixture.py down
