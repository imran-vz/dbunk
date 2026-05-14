tauri_manifest := "src-tauri/Cargo.toml"

default:
    @just --list

lint:
    cargo clippy --manifest-path {{tauri_manifest}} --all-targets -- -D warnings

test:
    cargo test --manifest-path {{tauri_manifest}}

build:
    cargo build --manifest-path {{tauri_manifest}}

fmt:
    cargo fmt --manifest-path {{tauri_manifest}}
