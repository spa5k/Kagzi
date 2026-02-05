fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../../proto");
    println!("cargo:rerun-if-changed=../../proto/common.proto");
    println!("cargo:rerun-if-changed=../../proto/namespace.proto");
    println!("cargo:rerun-if-changed=../../proto/workflow.proto");
    println!("cargo:rerun-if-changed=../../proto/worker.proto");
    println!("cargo:rerun-if-changed=../../proto/admin.proto");
    println!("cargo:rerun-if-changed=../../proto/workflow_schedule.proto");
    println!("cargo:rerun-if-changed=../../proto/telemetry.proto");
    println!("cargo:rerun-if-changed=../../proto/queue.proto");

    tonic_prost_build::configure()
        .file_descriptor_set_path("src/descriptor.bin")
        .compile_protos(
            &[
                "../../proto/common.proto",
                "../../proto/namespace.proto",
                "../../proto/workflow.proto",
                "../../proto/worker.proto",
                "../../proto/admin.proto",
                "../../proto/workflow_schedule.proto",
                "../../proto/telemetry.proto",
                "../../proto/queue.proto",
            ],
            &["../../proto"],
        )?;
    Ok(())
}
