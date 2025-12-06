/// Builds Protobuf/Tonic bindings and writes the compiled descriptor set.
///
/// This `main` runs the build step that compiles the project's Protobuf files
/// into Rust types for `tonic`/`prost` and writes the generated file descriptor
/// set to `src/descriptor.bin`.
///
/// # Returns
///
/// `Ok(())` if compilation and descriptor writing succeed, `Err` with an error
/// otherwise.
///
/// # Examples
///
/// ```no_run
/// fn main() {
///     // In build scripts the returned Result is typically propagated to the
///     // build system; here we call and unwrap for demonstration.
///     build::main().unwrap();
/// }
/// ```
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .file_descriptor_set_path("src/descriptor.bin")
        .compile_protos(
            &[
                "../../proto/errors.proto",
                "../../proto/workflow.proto",
                "../../proto/worker.proto",
                "../../proto/health.proto",
                "../../proto/schedule.proto",
            ],
            &["../../proto"],
        )?;
    Ok(())
}