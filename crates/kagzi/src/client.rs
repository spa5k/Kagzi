//! Kagzi client for workflow management

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;
use uuid::Uuid;

use kagzi_core::{queries, CreateWorkflowRun, Database, WorkflowRun};

use crate::context::WorkflowContext;
use crate::worker::Worker;

/// Type alias for workflow functions
pub type WorkflowFn<I> = Arc<
    dyn Fn(WorkflowContext, I) -> Pin<Box<dyn Future<Output = Result<serde_json::Value>> + Send>>
        + Send
        + Sync,
>;

/// Main Kagzi client for managing workflows
pub struct Kagzi {
    db: Arc<Database>,
    workflows: Arc<RwLock<HashMap<String, WorkflowFn<serde_json::Value>>>>,
}

impl Kagzi {
    /// Connect to Kagzi using a database URL
    ///
    /// This will establish a connection pool and automatically run migrations.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::Kagzi;
    /// # async fn example() -> anyhow::Result<()> {
    /// let kagzi = Kagzi::connect("postgres://localhost/kagzi").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn connect(database_url: &str) -> Result<Self> {
        let db = Database::connect(database_url).await?;

        // Run migrations automatically
        db.migrate().await?;

        info!("Kagzi initialized successfully");

        Ok(Self {
            db: Arc::new(db),
            workflows: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Register a workflow
    ///
    /// # Example
    ///
    /// ```no_run
    /// # use kagzi::{Kagzi, WorkflowContext};
    /// # use serde::{Deserialize, Serialize};
    /// # async fn example() -> anyhow::Result<()> {
    /// let kagzi = Kagzi::connect("postgres://localhost/kagzi").await?;
    ///
    /// #[derive(Serialize, Deserialize)]
    /// struct MyInput { name: String }
    ///
    /// kagzi.register_workflow("my-workflow", |ctx, input: MyInput| async move {
    ///     let result = ctx.step("greet", async {
    ///         Ok::<_, anyhow::Error>(format!("Hello, {}", input.name))
    ///     }).await?;
    ///     Ok(result)
    /// }).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register_workflow<I, O, F, Fut>(&self, name: &str, workflow_fn: F)
    where
        I: for<'de> Deserialize<'de> + Send + 'static,
        O: Serialize + Send + 'static,
        F: Fn(WorkflowContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O>> + Send + 'static,
    {
        let workflow_fn = Arc::new(move |ctx: WorkflowContext, input: serde_json::Value| {
            let input: I = serde_json::from_value(input).expect("Failed to deserialize input");
            let fut = workflow_fn(ctx, input);
            Box::pin(async move {
                let result = fut.await?;
                Ok(serde_json::to_value(result)?)
            }) as Pin<Box<dyn Future<Output = Result<serde_json::Value>> + Send>>
        });

        let mut workflows = self.workflows.write().await;
        workflows.insert(name.to_string(), workflow_fn);

        info!("Registered workflow: {}", name);
    }

    /// Start a workflow run
    ///
    /// Returns a handle to the workflow run that can be used to query status
    /// or wait for results.
    pub async fn start_workflow<I>(&self, name: &str, input: I) -> Result<WorkflowHandle>
    where
        I: Serialize,
    {
        let input_value = serde_json::to_value(input)?;

        let run = queries::create_workflow_run(
            self.db.pool(),
            CreateWorkflowRun {
                workflow_name: name.to_string(),
                workflow_version: None,
                input: input_value,
            },
        )
        .await?;

        info!("Started workflow: {} (run_id: {})", name, run.id);

        Ok(WorkflowHandle {
            run_id: run.id,
            db: self.db.clone(),
        })
    }

    /// Create a new worker
    ///
    /// Workers poll the database for pending workflows and execute them.
    pub fn create_worker(&self) -> Worker {
        Worker::new(self.db.clone(), self.workflows.clone())
    }

    /// Get a handle to an existing workflow run
    pub fn get_workflow_handle(&self, run_id: Uuid) -> WorkflowHandle {
        WorkflowHandle {
            run_id,
            db: self.db.clone(),
        }
    }

    /// Health check
    pub async fn health_check(&self) -> Result<()> {
        self.db.health_check().await?;
        Ok(())
    }
}

/// Handle to a workflow run
pub struct WorkflowHandle {
    run_id: Uuid,
    db: Arc<Database>,
}

impl WorkflowHandle {
    /// Get the workflow run ID
    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    /// Get the current status of the workflow
    pub async fn status(&self) -> Result<WorkflowRun> {
        Ok(queries::get_workflow_run(self.db.pool(), self.run_id).await?)
    }

    /// Wait for the workflow to complete and return the result
    ///
    /// This polls the database until the workflow reaches a terminal state.
    pub async fn result(&self) -> Result<serde_json::Value> {
        loop {
            let run = self.status().await?;

            match run.status {
                kagzi_core::WorkflowStatus::Completed => {
                    return run.output.ok_or_else(|| {
                        anyhow::anyhow!("Workflow completed but no output was stored")
                    });
                }
                kagzi_core::WorkflowStatus::Failed => {
                    return Err(anyhow::anyhow!(
                        "Workflow failed: {}",
                        run.error.unwrap_or_else(|| "Unknown error".to_string())
                    ));
                }
                kagzi_core::WorkflowStatus::Cancelled => {
                    return Err(anyhow::anyhow!("Workflow was cancelled"));
                }
                _ => {
                    // Still running, sleep and check again
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Cancel the workflow
    pub async fn cancel(&self) -> Result<()> {
        queries::update_workflow_status(
            self.db.pool(),
            self.run_id,
            kagzi_core::WorkflowStatus::Cancelled,
            Some("Cancelled by user".to_string()),
        )
        .await?;

        info!("Cancelled workflow: {}", self.run_id);
        Ok(())
    }
}
