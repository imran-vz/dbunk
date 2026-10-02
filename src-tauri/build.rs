fn main() {
    #[cfg(feature = "tauri-host")]
    tauri_build::build()
}
