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
    let proto = match proto {
        None => return fallback.cloned(),
        Some(p) => p,
    };

    // Start with fallback or default, then override with proto
    let mut policy = fallback.cloned().unwrap_or_default();

    if proto.maximum_attempts != 0 {
        policy.maximum_attempts = proto.maximum_attempts;
    }
    if proto.initial_interval_ms != 0 {
        policy.initial_interval_ms = proto.initial_interval_ms;
    }
    if proto.backoff_coefficient != 0.0 {
        policy.backoff_coefficient = proto.backoff_coefficient;
    }
    if proto.maximum_interval_ms != 0 {
        policy.maximum_interval_ms = proto.maximum_interval_ms;
    }
    if !proto.non_retryable_errors.is_empty() {
        policy.non_retryable_errors = proto.non_retryable_errors;
    }

    Some(policy)
}

pub fn require_non_empty(value: String, field: &str) -> Result<String, Status> {
    if value.is_empty() {
        Err(invalid_argument_error(format!("{} is required", field)))
    } else {
        Ok(value)
    }
}

pub fn normalize_page_size(requested: i32, default: i32, max: i32) -> i32 {
    if requested <= 0 {
        default
    } else {
        requested.min(max)
    }
}

pub fn encode_cursor(timestamp_ms: i64, id: &uuid::Uuid) -> String {
    use base64::Engine;
    let cursor_str = format!("{}:{}", timestamp_ms, id);
    base64::engine::general_purpose::STANDARD.encode(cursor_str.as_bytes())
}

pub fn decode_cursor(token: &str) -> Result<(chrono::DateTime<chrono::Utc>, uuid::Uuid), Status> {
    use base64::Engine;
    use chrono::TimeZone;

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(token)
        .map_err(|_| invalid_argument_error("Invalid page_token"))?;

    let token_str =
        std::str::from_utf8(&decoded).map_err(|_| invalid_argument_error("Invalid page_token"))?;

    let mut parts = token_str.splitn(2, ':');
    let created_at_ms = parts
        .next()
        .and_then(|p| p.parse::<i64>().ok())
        .ok_or_else(|| invalid_argument_error("Invalid page_token"))?;
    let id_str = parts
        .next()
        .ok_or_else(|| invalid_argument_error("Invalid page_token"))?;

    let created_at = chrono::Utc
        .timestamp_millis_opt(created_at_ms)
        .single()
        .ok_or_else(|| invalid_argument_error("Invalid page_token"))?;
    let id =
        uuid::Uuid::parse_str(id_str).map_err(|_| invalid_argument_error("Invalid page_token"))?;

    Ok((created_at, id))
}

pub fn parse_uuid(s: &str) -> Result<uuid::Uuid, Status> {
    uuid::Uuid::parse_str(s)
        .map_err(|_| invalid_argument_error(format!("Invalid UUID format: {}", s)))
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
