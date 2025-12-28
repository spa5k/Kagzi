use std::collections::HashMap;

use kagzi_proto::kagzi::{ErrorCode, ErrorDetail, Payload, RetryPolicy};
use prost::Message;
use tonic::{Code, Status};

pub fn err_detail(
    code: ErrorCode,
    message: impl Into<String>,
    non_retryable: bool,
    retry_after_ms: i64,
    subject: impl Into<String>,
    subject_id: impl Into<String>,
) -> ErrorDetail {
    ErrorDetail {
        code: code as i32,
        message: message.into(),
        non_retryable,
        retry_after_ms,
        subject: subject.into(),
        subject_id: subject_id.into(),
        metadata: HashMap::new(),
    }
}

fn status_with_detail(code: Code, detail: ErrorDetail) -> Status {
    Status::with_details(code, detail.message.clone(), detail.encode_to_vec().into())
}

pub fn invalid_argument_error(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::InvalidArgument,
        err_detail(ErrorCode::InvalidArgument, message, true, 0, "", ""),
    )
}

pub fn not_found_error(
    message: impl Into<String>,
    subject: impl Into<String>,
    id: impl Into<String>,
) -> Status {
    status_with_detail(
        Code::NotFound,
        err_detail(ErrorCode::NotFound, message, true, 0, subject, id),
    )
}

pub fn precondition_failed_error(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::FailedPrecondition,
        err_detail(ErrorCode::PreconditionFailed, message, true, 0, "", ""),
    )
}

pub fn conflict_error(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::Aborted,
        err_detail(ErrorCode::Conflict, message, false, 0, "", ""),
    )
}

pub fn unavailable_error(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::Unavailable,
        err_detail(ErrorCode::Unavailable, message, false, 0, "", ""),
    )
}

pub fn deadline_exceeded_error(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::DeadlineExceeded,
        err_detail(ErrorCode::Timeout, message, false, 0, "", ""),
    )
}

pub fn permission_denied_error(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::PermissionDenied,
        err_detail(ErrorCode::Unauthorized, message, true, 0, "", ""),
    )
}

pub fn internal_error(message: impl Into<String>) -> Status {
    status_with_detail(
        Code::Internal,
        err_detail(ErrorCode::Internal, message, true, 0, "", ""),
    )
}

pub fn string_error_detail(message: Option<String>) -> ErrorDetail {
    err_detail(
        ErrorCode::Unspecified,
        message.unwrap_or_default(),
        false,
        0,
        "",
        "",
    )
}

pub fn payload_to_bytes(payload: Option<Payload>) -> Vec<u8> {
    payload.map(|p| p.data).unwrap_or_default()
}

pub fn payload_to_optional_bytes(payload: Option<Payload>) -> Option<Vec<u8>> {
    payload.map(|p| p.data).filter(|d| !d.is_empty())
}

pub fn bytes_to_payload(data: Option<Vec<u8>>) -> Payload {
    Payload {
        data: data.unwrap_or_default(),
        metadata: HashMap::new(),
    }
}

pub fn json_to_payload(value: Option<serde_json::Value>) -> Result<Payload, Status> {
    match value {
        None => Ok(Payload {
            data: Vec::new(),
            metadata: HashMap::new(),
        }),
        Some(v) => {
            let data = serde_json::to_vec(&v)
                .map_err(|e| internal_error(format!("Failed to serialize payload: {}", e)))?;
            Ok(Payload {
                data,
                metadata: HashMap::new(),
            })
        }
    }
}

pub fn merge_proto_policy(
    proto: Option<RetryPolicy>,
    fallback: Option<&kagzi_store::RetryPolicy>,
) -> Option<kagzi_store::RetryPolicy> {
    match (proto, fallback.cloned()) {
        (None, None) => None,
        (None, Some(base)) => Some(base),
        (Some(p), Some(mut base)) => {
            if p.maximum_attempts != 0 {
                base.maximum_attempts = p.maximum_attempts;
            }
            if p.initial_interval_ms != 0 {
                base.initial_interval_ms = p.initial_interval_ms;
            }
            if p.backoff_coefficient != 0.0 {
                base.backoff_coefficient = p.backoff_coefficient;
            }
            if p.maximum_interval_ms != 0 {
                base.maximum_interval_ms = p.maximum_interval_ms;
            }
            if !p.non_retryable_errors.is_empty() {
                base.non_retryable_errors = p.non_retryable_errors;
            }
            Some(base)
        }
        (Some(p), None) => Some(kagzi_store::RetryPolicy {
            maximum_attempts: if p.maximum_attempts == 0 {
                5
            } else {
                p.maximum_attempts
            },
            initial_interval_ms: if p.initial_interval_ms == 0 {
                1000
            } else {
                p.initial_interval_ms
            },
            backoff_coefficient: if p.backoff_coefficient == 0.0 {
                2.0
            } else {
                p.backoff_coefficient
            },
            maximum_interval_ms: if p.maximum_interval_ms == 0 {
                60000
            } else {
                p.maximum_interval_ms
            },
            non_retryable_errors: p.non_retryable_errors,
        }),
    }
}

pub fn map_store_error(e: kagzi_store::StoreError) -> Status {
    match e {
        kagzi_store::StoreError::NotFound { entity, id } => {
            not_found_error(format!("{} not found", entity), entity, id)
        }
        kagzi_store::StoreError::InvalidArgument { message } => invalid_argument_error(message),
        kagzi_store::StoreError::InvalidState { message } => precondition_failed_error(message),
        kagzi_store::StoreError::AlreadyCompleted { message } => precondition_failed_error(message),
        kagzi_store::StoreError::Conflict { message } => conflict_error(message),
        kagzi_store::StoreError::LockConflict { message } => conflict_error(message),
        kagzi_store::StoreError::PreconditionFailed { message } => {
            precondition_failed_error(message)
        }
        kagzi_store::StoreError::Unauthorized { message } => permission_denied_error(message),
        kagzi_store::StoreError::Unavailable { message } => unavailable_error(message),
        kagzi_store::StoreError::Timeout { message } => deadline_exceeded_error(message),
        kagzi_store::StoreError::Database(e) => {
            tracing::error!("Database error: {:?}", e);
            internal_error("Database error")
        }
        kagzi_store::StoreError::Serialization(e) => {
            tracing::error!("Serialization error: {:?}", e);
            internal_error("Serialization error")
        }
    }
}
