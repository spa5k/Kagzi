use std::collections::HashMap;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{de::DeserializeOwned, Serialize};
use crate::context::WorkflowContext;

/// Type alias for workflow functions
/// A workflow function takes a context and input, and returns a future that resolves to the output
pub type WorkflowFn<I, O> = Arc<
    dyn Fn(WorkflowContext, I) -> Pin<Box<dyn Future<Output = anyhow::Result<O>> + Send>> + Send + Sync
>;

/// Type-erased workflow function for storage in the registry
pub trait DynWorkflowFn: Send + Sync {
    fn call(&self, ctx: WorkflowContext, input: serde_json::Value) -> Pin<Box<dyn Future<Output = anyhow::Result<serde_json::Value>> + Send>>;
}

/// Wrapper that implements DynWorkflowFn for concrete workflow functions
struct WorkflowFnWrapper<I, O, F>
where
    I: DeserializeOwned + Send + Sync + 'static,
    O: Serialize + Send + Sync + 'static,
    F: Fn(WorkflowContext, I) -> Pin<Box<dyn Future<Output = anyhow::Result<O>> + Send>> + Send + Sync + 'static,
{
    func: F,
    _phantom_i: PhantomData<fn() -> I>,
    _phantom_o: PhantomData<fn() -> O>,
}

impl<I, O, F> DynWorkflowFn for WorkflowFnWrapper<I, O, F>
where
    I: DeserializeOwned + Send + Sync + 'static,
    O: Serialize + Send + Sync + 'static,
    F: Fn(WorkflowContext, I) -> Pin<Box<dyn Future<Output = anyhow::Result<O>> + Send>> + Send + Sync + 'static,
{
    fn call(&self, ctx: WorkflowContext, input: serde_json::Value) -> Pin<Box<dyn Future<Output = anyhow::Result<serde_json::Value>> + Send>> {
        let input_typed: I = match serde_json::from_value(input) {
            Ok(i) => i,
            Err(e) => return Box::pin(async move { Err(anyhow::anyhow!("Failed to deserialize input: {}", e)) }),
        };

        let fut = (self.func)(ctx, input_typed);
        Box::pin(async move {
            let output = fut.await?;
            Ok(serde_json::to_value(output)?)
        })
    }
}

/// Registry for managing workflow versions
/// Stores workflow functions by (name, version) and tracks default versions
pub struct WorkflowRegistry {
    /// Map of (workflow_name, version) -> workflow function
    workflows: RwLock<HashMap<(String, i32), Arc<dyn DynWorkflowFn>>>,

    /// Map of workflow_name -> default version
    default_versions: RwLock<HashMap<String, i32>>,
}

impl WorkflowRegistry {
    /// Create a new empty workflow registry
    pub fn new() -> Self {
        Self {
            workflows: RwLock::new(HashMap::new()),
            default_versions: RwLock::new(HashMap::new()),
        }
    }

    /// Register a workflow with a specific version
    pub async fn register<I, O, F>(
        &self,
        workflow_name: impl Into<String>,
        version: i32,
        workflow_fn: F,
    ) where
        I: DeserializeOwned + Send + Sync + 'static,
        O: Serialize + Send + Sync + 'static,
        F: Fn(WorkflowContext, I) -> Pin<Box<dyn Future<Output = anyhow::Result<O>> + Send>> + Send + Sync + 'static,
    {
        let name = workflow_name.into();
        let wrapper = WorkflowFnWrapper {
            func: workflow_fn,
            _phantom_i: PhantomData,
            _phantom_o: PhantomData,
        };

        let mut workflows = self.workflows.write().await;
        workflows.insert((name, version), Arc::new(wrapper));
    }

    /// Set the default version for a workflow
    pub async fn set_default_version(
        &self,
        workflow_name: impl Into<String>,
        version: i32,
    ) -> anyhow::Result<()> {
        let name = workflow_name.into();

        // Check if the version is registered
        let workflows = self.workflows.read().await;
        if !workflows.contains_key(&(name.clone(), version)) {
            return Err(anyhow::anyhow!(
                "Cannot set default version: workflow '{}' version {} is not registered",
                name,
                version
            ));
        }
        drop(workflows);

        let mut defaults = self.default_versions.write().await;
        defaults.insert(name, version);
        Ok(())
    }

    /// Get the default version for a workflow
    pub async fn get_default_version(&self, workflow_name: &str) -> Option<i32> {
        let defaults = self.default_versions.read().await;
        defaults.get(workflow_name).copied()
    }

    /// Look up a workflow function by name and version
    pub async fn get_workflow(
        &self,
        workflow_name: &str,
        version: i32,
    ) -> Option<Arc<dyn DynWorkflowFn>> {
        let workflows = self.workflows.read().await;
        workflows.get(&(workflow_name.to_string(), version)).cloned()
    }

    /// Get all registered versions for a workflow
    pub async fn get_versions(&self, workflow_name: &str) -> Vec<i32> {
        let workflows = self.workflows.read().await;
        let mut versions: Vec<i32> = workflows
            .keys()
            .filter(|(name, _)| name == workflow_name)
            .map(|(_, version)| *version)
            .collect();
        versions.sort();
        versions
    }

    /// Check if a specific workflow version is registered
    pub async fn is_registered(&self, workflow_name: &str, version: i32) -> bool {
        let workflows = self.workflows.read().await;
        workflows.contains_key(&(workflow_name.to_string(), version))
    }

    /// Remove a workflow version from the registry
    pub async fn unregister(&self, workflow_name: &str, version: i32) -> bool {
        let mut workflows = self.workflows.write().await;
        workflows.remove(&(workflow_name.to_string(), version)).is_some()
    }

    /// Get the total number of registered workflows (across all versions)
    pub async fn workflow_count(&self) -> usize {
        let workflows = self.workflows.read().await;
        workflows.len()
    }
}

impl Default for WorkflowRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct TestInput {
        value: i32,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestOutput {
        result: i32,
    }

    async fn test_workflow_v1(_ctx: WorkflowContext, input: TestInput) -> anyhow::Result<TestOutput> {
        Ok(TestOutput { result: input.value * 2 })
    }

    async fn test_workflow_v2(_ctx: WorkflowContext, input: TestInput) -> anyhow::Result<TestOutput> {
        Ok(TestOutput { result: input.value * 3 })
    }

    #[tokio::test]
    async fn test_register_and_lookup() {
        let registry = WorkflowRegistry::new();

        registry.register("test-workflow", 1, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;
        registry.register("test-workflow", 2, |ctx, input| Box::pin(test_workflow_v2(ctx, input))).await;

        assert!(registry.is_registered("test-workflow", 1).await);
        assert!(registry.is_registered("test-workflow", 2).await);
        assert!(!registry.is_registered("test-workflow", 3).await);
        assert!(!registry.is_registered("other-workflow", 1).await);
    }

    #[tokio::test]
    async fn test_default_version() {
        let registry = WorkflowRegistry::new();

        registry.register("test-workflow", 1, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;
        registry.register("test-workflow", 2, |ctx, input| Box::pin(test_workflow_v2(ctx, input))).await;

        // Initially no default
        assert_eq!(registry.get_default_version("test-workflow").await, None);

        // Set version 1 as default
        registry.set_default_version("test-workflow", 1).await.unwrap();
        assert_eq!(registry.get_default_version("test-workflow").await, Some(1));

        // Update to version 2
        registry.set_default_version("test-workflow", 2).await.unwrap();
        assert_eq!(registry.get_default_version("test-workflow").await, Some(2));
    }

    #[tokio::test]
    async fn test_set_default_for_unregistered_version() {
        let registry = WorkflowRegistry::new();

        registry.register("test-workflow", 1, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;

        // Try to set unregistered version as default
        let result = registry.set_default_version("test-workflow", 2).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_versions() {
        let registry = WorkflowRegistry::new();

        registry.register("test-workflow", 3, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;
        registry.register("test-workflow", 1, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;
        registry.register("test-workflow", 2, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;

        let versions = registry.get_versions("test-workflow").await;
        assert_eq!(versions, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_unregister() {
        let registry = WorkflowRegistry::new();

        registry.register("test-workflow", 1, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;
        registry.register("test-workflow", 2, |ctx, input| Box::pin(test_workflow_v2(ctx, input))).await;

        assert!(registry.is_registered("test-workflow", 1).await);

        // Unregister version 1
        assert!(registry.unregister("test-workflow", 1).await);
        assert!(!registry.is_registered("test-workflow", 1).await);
        assert!(registry.is_registered("test-workflow", 2).await);

        // Try to unregister again
        assert!(!registry.unregister("test-workflow", 1).await);
    }

    #[tokio::test]
    async fn test_workflow_count() {
        let registry = WorkflowRegistry::new();

        assert_eq!(registry.workflow_count().await, 0);

        registry.register("workflow-a", 1, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;
        assert_eq!(registry.workflow_count().await, 1);

        registry.register("workflow-a", 2, |ctx, input| Box::pin(test_workflow_v2(ctx, input))).await;
        assert_eq!(registry.workflow_count().await, 2);

        registry.register("workflow-b", 1, |ctx, input| Box::pin(test_workflow_v1(ctx, input))).await;
        assert_eq!(registry.workflow_count().await, 3);
    }
}
