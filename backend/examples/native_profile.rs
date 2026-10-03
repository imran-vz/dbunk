//! Headless, fixture-only stage04 profile setup. No GUI or password arguments.
use dbunk_lib::backend::{Backend, DevelopmentFixtures};
use std::{io::Read, path::PathBuf};

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Native profile: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [operation, path, manifest] = args.as_slice() else {
        return Err("Usage: native_profile <create|check> <canonical-new-or-existing-path> <verified-fixture-manifest>".into());
    };
    let mut json = String::new();
    std::fs::File::open(manifest)
        .map_err(|_| "Fixture manifest is unavailable")?
        .take(4097)
        .read_to_string(&mut json)
        .map_err(|_| "Fixture manifest is unreadable")?;
    let fixtures = DevelopmentFixtures::from_json(&json)?;
    let path = PathBuf::from(path);
    let backend = match operation.to_str() {
        Some("create") => Backend::create_development(&path, fixtures).await?,
        Some("check") => Backend::open_development(&path, &fixtures).await?,
        _ => return Err("Expected create or check".into()),
    };
    let snapshot = backend.development_settings().await;
    backend.shutdown().await?;
    println!(
        "{}",
        serde_json::to_string(&snapshot?).map_err(|_| "Could not encode development settings")?
    );
    Ok(())
}
