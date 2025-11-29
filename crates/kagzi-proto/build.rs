fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .file_descriptor_set_path("src/descriptor.bin")
        .compile_protos(&["../../proto/kagzi.proto"], &["../../proto"])?;
    Ok(())
}
