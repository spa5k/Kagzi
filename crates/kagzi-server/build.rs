use std::path::PathBuf;

fn main() {
    // The server embeds the frontend build output via `rust-embed` from `../../app/dist`.
    // In CI the frontend build step may be skipped, so ensure the folder exists to avoid
    // compile-time failures from `#[derive(RustEmbed)]`.
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dist_dir = manifest_dir.join("../../app/dist");

    if !dist_dir.exists() {
        std::fs::create_dir_all(&dist_dir)
            .unwrap_or_else(|e| panic!("failed to create UI dist directory at {dist_dir:?}: {e}"));
    }

    println!("cargo:rerun-if-changed=build.rs");
}
