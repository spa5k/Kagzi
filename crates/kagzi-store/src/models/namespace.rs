use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Namespace represents a multi-tenancy isolation boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    pub namespace_id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// CreateNamespace contains parameters for creating a new namespace.
#[derive(Debug, Clone)]
pub struct CreateNamespace {
    pub namespace_id: String,
    pub display_name: String,
    pub description: Option<String>,
}

/// ResourceCount tracks dependent resources in a namespace.
#[derive(Debug, Clone, Default)]
pub struct ResourceCount {
    pub workflows: i64,
    pub workers: i64,
    pub schedules: i64,
}

impl ResourceCount {
    /// Returns true if there are any resources in the namespace.
    pub fn has_resources(&self) -> bool {
        self.workflows > 0 || self.workers > 0 || self.schedules > 0
    }
}
