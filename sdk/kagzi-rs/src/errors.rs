use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::time::Duration;

use kagzi_proto::kagzi::{ErrorCode, ErrorDetail};
use prost::Message;
use tonic::{Code, Status};

#[derive(Debug)]
pub struct WorkflowPaused;

impl Display for WorkflowPaused {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Workflow paused")
    }
}

impl std::error::Error for WorkflowPaused {}

#[derive(Debug, Clone)]
pub struct KagziError {
    pub code: ErrorCode,
    pub message: String,
    pub non_retryable: bool,
    pub retry_after: Option<Duration>,
    pub subject: Option<String>,
    pub subject_id: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl KagziError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            non_retryable: matches!(
                code,
                ErrorCode::InvalidArgument
                    | ErrorCode::PreconditionFailed
                    | ErrorCode::Conflict
                    | ErrorCode::Unauthorized
            ),
            retry_after: None,
            subject: None,
            subject_id: None,
            metadata: HashMap::new(),
        }
    }

    pub fn non_retryable(message: impl Into<String>) -> Self {
        Self {
            code: ErrorCode::PreconditionFailed,
            message: message.into(),
            non_retryable: true,
            retry_after: None,
            subject: None,
            subject_id: None,
            metadata: HashMap::new(),
        }
    }

    pub fn retry_after(message: impl Into<String>, retry_after: Duration) -> Self {
        Self {
            code: ErrorCode::Unavailable,
            message: message.into(),
            non_retryable: false,
            retry_after: Some(retry_after),
            subject: None,
            subject_id: None,
            metadata: HashMap::new(),
        }
    }

    pub fn to_detail(&self) -> ErrorDetail {
        ErrorDetail {
            code: self.code as i32,
            message: self.message.clone(),
            non_retryable: self.non_retryable,
            retry_after_ms: self.retry_after.map(|d| d.as_millis() as i64).unwrap_or(0),
            subject: self.subject.clone().unwrap_or_default(),
            subject_id: self.subject_id.clone().unwrap_or_default(),
            metadata: self.metadata.clone(),
        }
    }
}

impl Display for KagziError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({:?})", self.message, self.code)
    }
}

impl std::error::Error for KagziError {}

fn error_code_from_status(code: Code) -> ErrorCode {
    match code {
        Code::NotFound => ErrorCode::NotFound,
        Code::InvalidArgument => ErrorCode::InvalidArgument,
        Code::FailedPrecondition => ErrorCode::PreconditionFailed,
        Code::Aborted => ErrorCode::Conflict,
        Code::PermissionDenied => ErrorCode::Unauthorized,
        Code::Unavailable => ErrorCode::Unavailable,
        Code::DeadlineExceeded | Code::Cancelled | Code::ResourceExhausted => {
            ErrorCode::Unavailable
        }
        _ => ErrorCode::Internal,
    }
}

impl From<Status> for KagziError {
    fn from(status: Status) -> Self {
        if let Ok(detail) = ErrorDetail::decode(status.details()) {
            return Self {
                code: ErrorCode::try_from(detail.code).unwrap_or(ErrorCode::Internal),
                message: if detail.message.is_empty() {
                    status.message().to_string()
                } else {
                    detail.message
                },
                non_retryable: detail.non_retryable,
                retry_after: if detail.retry_after_ms > 0 {
                    Some(Duration::from_millis(detail.retry_after_ms as u64))
                } else {
                    None
                },
                subject: if detail.subject.is_empty() {
                    None
                } else {
                    Some(detail.subject)
                },
                subject_id: if detail.subject_id.is_empty() {
                    None
                } else {
                    Some(detail.subject_id)
                },
                metadata: detail.metadata,
            };
        }

        Self {
            code: error_code_from_status(status.code()),
            message: status.message().to_string(),
            non_retryable: matches!(
                status.code(),
                Code::InvalidArgument | Code::FailedPrecondition | Code::PermissionDenied
            ),
            retry_after: None,
            subject: None,
            subject_id: None,
            metadata: HashMap::new(),
        }
    }
}

pub(crate) fn map_grpc_error(status: Status) -> anyhow::Error {
    anyhow::Error::new(KagziError::from(status))
}
