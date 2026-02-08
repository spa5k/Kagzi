pub mod kagzi {
    pub mod v1 {
        tonic::include_proto!("kagzi.v1");
    }

    // Temporary re-export to ease transition from unversioned package paths.
    pub use v1::*;
}

/// File descriptor set for gRPC reflection
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("descriptor.bin");
