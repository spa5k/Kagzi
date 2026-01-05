use std::collections::HashMap;
use std::str::FromStr;

use cron::Schedule;
use kagzi_proto::kagzi::workflow_schedule_service_client::WorkflowScheduleServiceClient;
use kagzi_proto::kagzi::workflow_service_client::WorkflowServiceClient;
use kagzi_proto::kagzi::{
    CreateWorkflowScheduleRequest, DeleteWorkflowScheduleRequest, GetWorkflowScheduleRequest,
    ListWorkflowSchedulesRequest, PageRequest, Payload as ProtoPayload, StartWorkflowRequest,
    WorkflowSchedule,
};
use serde::Serialize;
use tonic::Request;
use tonic::transport::Channel;
use uuid::Uuid;

use crate::errors::KagziError;

/// Main client for interacting with Kagzi workflow engine
///
/// The Kagzi client provides methods to start workflows, create schedules, and query schedule state.
///
/// # Example
///
/// ```no_run
/// use kagzi::Kagzi;
///
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// let client = Kagzi::connect("http://localhost:50051").await?;
///
/// // Start a workflow
/// let run = client
///     .start("my_workflow")
///     .namespace("production")
///     .input(&MyInput { value: 42 })?
///     .send()
///     .await?;
/// println!("Workflow started: {}", run.id);
/// # Ok(())
/// # }
/// # use serde::Serialize;
/// # struct MyInput { value: i32 }
/// ```
pub struct Kagzi {
    workflow_client: WorkflowServiceClient<Channel>,
    schedule_client: WorkflowScheduleServiceClient<Channel>,
}

impl Kagzi {
    /// Connect to a Kagzi server
    ///
    /// # Arguments
    ///
    /// * `addr` - Server address (e.g., "http://localhost:50051")
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails or the address is invalid
    #[must_use = "Client connection is required to start workflows"]
    pub async fn connect(addr: &str) -> anyhow::Result<Self> {
        let channel = Channel::from_shared(addr.to_string())
            .map_err(|e| anyhow::anyhow!("Failed to create channel: {e}"))?
            .connect()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to server: {e}"))?;

        Ok(Self {
            workflow_client: WorkflowServiceClient::new(channel.clone()),
            schedule_client: WorkflowScheduleServiceClient::new(channel),
        })
    }

    /// Start building a new workflow execution
    ///
    /// Returns a builder for configuring and starting a workflow execution.
    ///
    /// # Arguments
    ///
    /// * `workflow_type` - The type/identifier of the workflow to run
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::Kagzi;
    /// # use serde::Serialize;
    /// # #[derive(Serialize)]
    /// # struct MyInput { value: i32 }
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = Kagzi::connect("http://localhost:50051").await?;
    /// let run = client
    ///     .start("my_workflow")
    ///     .namespace("production")
    ///     .input(&MyInput { value: 42 })
    ///     .idempotency_key("unique-key-123")
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn start(&self, workflow_type: impl Into<String>) -> StartWorkflowBuilder {
        StartWorkflowBuilder {
            client: self.workflow_client.clone(),
            workflow_type: workflow_type.into(),
            namespace: "default".to_string(),
            input: None,
            idempotency_key: None,
        }
    }

    /// Start building a new workflow schedule
    ///
    /// Returns a builder for configuring and creating a workflow schedule.
    ///
    /// # Arguments
    ///
    /// * `schedule_id` - Unique identifier for this schedule
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::Kagzi;
    /// # use serde::Serialize;
    /// # #[derive(Serialize)]
    /// # struct CleanupInput { table: String }
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = Kagzi::connect("http://localhost:50051").await?;
    /// let schedule = client
    ///     .schedule("daily-cleanup")
    ///     .namespace("production")
    ///     .workflow("cleanup_workflow")
    ///     .cron("0 2 * * *")  // 2 AM daily
    ///     .input(&CleanupInput { table: "logs".to_string() })
    ///     .catchup(10)
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn schedule(&self, schedule_id: impl Into<String>) -> ScheduleBuilder {
        ScheduleBuilder {
            client: self.schedule_client.clone(),
            _schedule_id: schedule_id.into(),
            namespace: "default".to_string(),
            workflow_type: None,
            cron: None,
            input: None,
            max_catchup: 100,
            enabled: true,
        }
    }

    // Schedule query methods

    /// Get a workflow schedule by ID
    ///
    /// # Arguments
    ///
    /// * `schedule_id` - The schedule identifier
    /// * `namespace_id` - Optional namespace (defaults to "default")
    ///
    /// # Returns
    ///
    /// Returns `Some(schedule)` if found, `None` otherwise
    #[must_use = "Returns the workflow schedule if found"]
    pub async fn get_workflow_schedule(
        &self,
        schedule_id: &str,
        namespace_id: Option<&str>,
    ) -> anyhow::Result<Option<WorkflowSchedule>> {
        let mut client = self.schedule_client.clone();
        let request = Request::new(GetWorkflowScheduleRequest {
            schedule_id: schedule_id.to_string(),
            namespace_id: namespace_id.unwrap_or("default").to_string(),
        });

        let resp = client
            .get_workflow_schedule(request)
            .await
            .map_err(|e| anyhow::anyhow!(KagziError::from(e)))?
            .into_inner();

        Ok(resp.schedule)
    }

    /// List all workflow schedules in a namespace
    ///
    /// # Arguments
    ///
    /// * `namespace_id` - The namespace to list schedules from
    /// * `page` - Optional pagination request (defaults to 100 items)
    ///
    /// # Returns
    ///
    /// Returns a vector of workflow schedules
    #[must_use = "Returns the list of workflow schedules"]
    pub async fn list_workflow_schedules(
        &self,
        namespace_id: &str,
        page: Option<PageRequest>,
    ) -> anyhow::Result<Vec<WorkflowSchedule>> {
        let mut client = self.schedule_client.clone();
        let page_request = page.unwrap_or_else(|| PageRequest {
            page_size: 100,
            page_token: "".to_string(),
            include_total_count: false,
        });

        let request = Request::new(ListWorkflowSchedulesRequest {
            namespace_id: namespace_id.to_string(),
            task_queue: None,
            page: Some(page_request),
        });

        let resp = client
            .list_workflow_schedules(request)
            .await
            .map_err(|e| anyhow::anyhow!(KagziError::from(e)))?
            .into_inner();

        Ok(resp.schedules)
    }

    /// Delete a workflow schedule
    ///
    /// # Arguments
    ///
    /// * `schedule_id` - The schedule identifier to delete
    /// * `namespace_id` - Optional namespace (defaults to "default")
    pub async fn delete_workflow_schedule(
        &self,
        schedule_id: &str,
        namespace_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut client = self.schedule_client.clone();
        let request = Request::new(DeleteWorkflowScheduleRequest {
            schedule_id: schedule_id.to_string(),
            namespace_id: namespace_id.unwrap_or("default").to_string(),
        });

        client
            .delete_workflow_schedule(request)
            .await
            .map_err(|e| anyhow::anyhow!(KagziError::from(e)))?;

        Ok(())
    }
}

/// Builder for starting a new workflow execution
///
/// Provides a fluent interface for configuring workflow execution parameters.
///
/// # Example
///
/// ```no_run
/// # use kagzi::Kagzi;
/// # use serde::Serialize;
/// # #[derive(Serialize)]
/// # struct MyInput { value: i32 }
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// # let client = Kagzi::connect("http://localhost:50051").await?;
/// let run = client
///     .start("my_workflow")
///     .namespace("production")
///     .input(&MyInput { value: 42 })
///     .idempotency_key("unique-key")
///     .send()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct StartWorkflowBuilder {
    client: WorkflowServiceClient<Channel>,
    workflow_type: String,
    namespace: String,
    input: Option<Vec<u8>>,
    idempotency_key: Option<String>,
}

impl StartWorkflowBuilder {
    /// Set the namespace for this workflow execution
    ///
    /// # Arguments
    ///
    /// * `ns` - Namespace identifier (defaults to "default" if not set)
    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }

    /// Set the input payload for the workflow
    ///
    /// The input must be serializable as JSON. Serialization errors will panic.
    ///
    /// # Type Parameters
    ///
    /// * `T` - Any type that implements `Serialize`
    ///
    /// # Arguments
    ///
    /// * `input` - Reference to the input data (will be JSON serialized)
    ///
    /// # Example
    ///
    /// ```
    /// # use kagzi::Kagzi;
    /// # use serde::Serialize;
    /// # #[derive(Serialize)]
    /// # struct MyInput { value: i32 }
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = Kagzi::connect("http://localhost:50051").await?;
    /// let input = MyInput { value: 42 };
    /// let run = client
    ///     .start("my_workflow")
    ///     .input(&input)
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn input<T: Serialize>(mut self, input: &T) -> Self {
        self.input = Some(
            serde_json::to_vec(input)
                .expect("Failed to serialize workflow input"),
        );
        self
    }

    /// Set an idempotency key to prevent duplicate workflow executions
    ///
    /// If a workflow with the same idempotency key already exists in the same namespace,
    /// the existing workflow run will be returned instead of creating a new one.
    ///
    /// # Arguments
    ///
    /// * `key` - Unique identifier for idempotency (e.g., "order-123", "payment-abc")
    ///
    /// # Example
    ///
    /// ```
    /// # use kagzi::Kagzi;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = Kagzi::connect("http://localhost:50051").await?;
    /// // Will only create the workflow once, subsequent calls with same key return existing run
    /// let run = client
    ///     .start("charge_payment")
    ///     .idempotency_key("order-123")
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Validate the workflow start configuration
    ///
    /// Checks that required fields are set and valid before sending to server.
    fn validate(&self) -> anyhow::Result<()> {
        if self.workflow_type.is_empty() {
            anyhow::bail!("workflow_type cannot be empty");
        }

        if self.namespace.is_empty() {
            anyhow::bail!("namespace cannot be empty");
        }

        Ok(())
    }

    /// Send the workflow start request to the server
    ///
    /// Validates the configuration and sends the start workflow request.
    ///
    /// # Returns
    ///
    /// Returns a `WorkflowRun` handle containing the run ID
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Validation fails (empty workflow_type or namespace)
    /// - Network communication fails
    /// - Server returns an error
    #[must_use = "Returns the workflow run ID which is needed to track execution"]
    pub async fn send(mut self) -> anyhow::Result<WorkflowRun> {
        // Validate before sending
        self.validate()?;

        let resp = self
            .client
            .start_workflow(Request::new(StartWorkflowRequest {
                external_id: self
                    .idempotency_key
                    .unwrap_or_else(|| Uuid::now_v7().to_string()),
                task_queue: self.workflow_type.clone(), // Queue = workflow type
                workflow_type: self.workflow_type,
                input: self.input.map(|data| ProtoPayload {
                    data,
                    metadata: HashMap::new(),
                }),
                namespace_id: self.namespace,
                version: String::default(),
                retry_policy: None,
            }))
            .await
            .map_err(|e| anyhow::anyhow!(KagziError::from(e)))?;

        Ok(WorkflowRun {
            id: resp.into_inner().run_id,
        })
    }
}

/// Handle to a running workflow execution
///
/// Contains the unique identifier for the workflow run, which can be used
/// to query run status or results (if those features are implemented).
#[derive(Debug, Clone)]
pub struct WorkflowRun {
    /// Unique identifier for this workflow execution
    pub id: String,
}

/// Builder for creating workflow schedules
///
/// Provides a fluent interface for configuring scheduled workflow executions.
///
/// # Example
///
/// ```no_run
/// # use kagzi::Kagzi;
/// # use serde::Serialize;
/// # #[derive(Serialize)]
/// # struct CleanupInput { table: String }
/// # #[tokio::main]
/// # async fn main() -> anyhow::Result<()> {
/// # let client = Kagzi::connect("http://localhost:50051").await?;
/// let schedule = client
///     .schedule("daily-cleanup")
///     .namespace("production")
///     .workflow("cleanup_workflow")
///     .cron("0 2 * * *")  // 2 AM daily
///     .input(&CleanupInput { table: "logs".to_string() })
///     .catchup(10)
///     .enabled(true)
///     .send()
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct ScheduleBuilder {
    client: WorkflowScheduleServiceClient<Channel>,
    _schedule_id: String, // Stored for potential future use in error messages
    namespace: String,
    workflow_type: Option<String>,
    cron: Option<String>,
    input: Option<Vec<u8>>,
    max_catchup: i32,
    enabled: bool,
}

impl ScheduleBuilder {
    /// Set the namespace for this schedule
    ///
    /// # Arguments
    ///
    /// * `ns` - Namespace identifier (defaults to "default" if not set)
    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = ns.into();
        self
    }

    /// Set the workflow type to execute
    ///
    /// # Arguments
    ///
    /// * `workflow_type` - The type/identifier of the workflow to run
    ///
    /// # Note
    ///
    /// This field is required. The schedule will fail to create if not set.
    pub fn workflow(mut self, workflow_type: impl Into<String>) -> Self {
        self.workflow_type = Some(workflow_type.into());
        self
    }

    /// Set the cron expression for the schedule
    ///
    /// The cron expression uses the standard 5-field format:
    /// `minute hour day month weekday`
    ///
    /// # Arguments
    ///
    /// * `cron_expr` - Cron expression (e.g., "0 2 * * *" for 2 AM daily)
    ///
    /// # Examples
    ///
    /// - `"0 2 * * *"` - 2:00 AM every day
    /// - `"*/5 * * * *"` - Every 5 minutes
    /// - `"0 */2 * * *"` - Every 2 hours
    /// - `"0 0 * * 1"` - Midnight every Monday
    ///
    /// # Note
    ///
    /// This field is required. The cron expression will be validated before sending to server.
    pub fn cron(mut self, cron_expr: impl Into<String>) -> Self {
        self.cron = Some(cron_expr.into());
        self
    }

    /// Set the input payload for scheduled workflow executions
    ///
    /// The input must be serializable as JSON. This input will be used for all
    /// executions triggered by this schedule.
    ///
    /// # Type Parameters
    ///
    /// * `T` - Any type that implements `Serialize`
    ///
    /// # Arguments
    ///
    /// * `input` - Reference to the input data (will be JSON serialized)
    pub fn input<T: Serialize>(mut self, input: &T) -> Self {
        self.input = Some(
            serde_json::to_vec(input)
                .expect("Failed to serialize schedule input"),
        );
        self
    }

    /// Set the maximum catchup for missed executions
    ///
    /// When a schedule falls behind (e.g., server was down), the catchup setting
    /// limits how many missed executions will be replayed. This prevents resource
    /// exhaustion when recovering from long outages.
    ///
    /// # Arguments
    ///
    /// * `max` - Maximum number of missed executions to replay (must be non-negative)
    ///
    /// # Example
    ///
    /// ```
    /// # use kagzi::Kagzi;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = Kagzi::connect("http://localhost:50051").await?;
    /// // Will replay at most 10 missed executions
    /// let schedule = client
    ///     .schedule("cleanup")
    ///     .workflow("cleanup_workflow")
    ///     .cron("0 * * * *")
    ///     .catchup(10)
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn catchup(mut self, max: i32) -> Self {
        self.max_catchup = max;
        self
    }

    /// Enable or disable the schedule
    ///
    /// # Arguments
    ///
    /// * `enabled` - If true, the schedule is active; if false, it's created but paused
    ///
    /// # Example
    ///
    /// ```
    /// # use kagzi::Kagzi;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let client = Kagzi::connect("http://localhost:50051").await?;
    /// // Create a disabled schedule that can be enabled later
    /// let schedule = client
    ///     .schedule("maintenance")
    ///     .workflow("maintenance_workflow")
    ///     .cron("0 3 * * *")
    ///     .enabled(false)  // Create in disabled state
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Validate the schedule configuration
    ///
    /// Checks that required fields are set, validates the cron expression,
    /// and ensures constraints are satisfied.
    fn validate(&self) -> anyhow::Result<()> {
        if self.namespace.is_empty() {
            anyhow::bail!("namespace cannot be empty");
        }

        let workflow_type = self
            .workflow_type
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("workflow_type is required"))?;

        if workflow_type.is_empty() {
            anyhow::bail!("workflow_type cannot be empty");
        }

        let cron = self
            .cron
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cron expression is required"))?;

        // Use cron crate for robust validation
        Schedule::from_str(cron)
            .map_err(|e| anyhow::anyhow!("Invalid cron expression '{cron}': {e}"))?;

        if self.max_catchup < 0 {
            anyhow::bail!("max_catchup must be non-negative");
        }

        Ok(())
    }

    /// Send the schedule creation request to the server
    ///
    /// Validates the configuration and creates the workflow schedule.
    ///
    /// # Returns
    ///
    /// Returns the created `WorkflowSchedule` from the server
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Validation fails (missing required fields, invalid cron)
    /// - Network communication fails
    /// - Server returns an error
    #[must_use = "Returns the created workflow schedule"]
    pub async fn send(mut self) -> anyhow::Result<WorkflowSchedule> {
        // Validate before sending
        self.validate()?;

        let workflow_type = self
            .workflow_type
            .ok_or_else(|| anyhow::anyhow!("workflow_type is required"))?;
        let cron = self
            .cron
            .ok_or_else(|| anyhow::anyhow!("cron expression is required"))?;

        let request = CreateWorkflowScheduleRequest {
            namespace_id: self.namespace,
            task_queue: workflow_type.clone(), // Queue = workflow type
            workflow_type,
            cron_expr: cron,
            input: self.input.map(|data| ProtoPayload {
                data,
                metadata: HashMap::new(),
            }),
            enabled: Some(self.enabled),
            max_catchup: Some(self.max_catchup),
            version: None,
        };

        let resp = self
            .client
            .create_workflow_schedule(Request::new(request))
            .await
            .map_err(|e| anyhow::anyhow!(KagziError::from(e)))?
            .into_inner()
            .schedule
            .ok_or_else(|| anyhow::anyhow!("Workflow schedule not returned by server"))?;

        Ok(resp)
    }
}
