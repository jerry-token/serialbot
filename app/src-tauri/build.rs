//! Build script: prepares the Tauri context.

fn main() {
    // `tauri::generate_context!` requires the frontend output directory to
    // exist. Creating it (empty) lets `cargo check/clippy/test` run without
    // building the frontend first; `npm run tauri build` fills it.
    let dist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist");
    if let Err(e) = std::fs::create_dir_all(&dist) {
        panic!("cannot create {}: {e}", dist.display());
    }
    tauri_build::build();
}
