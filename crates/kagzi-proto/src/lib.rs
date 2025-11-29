pub mod kagzi {
    tonic::include_proto!("kagzi");
}

/// File descriptor set for gRPC reflection
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("descriptor.bin");
