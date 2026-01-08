import type { PageRequest } from "@/gen/common_pb";
import { type ListWorkflowsRequest, WorkflowStatus } from "@/gen/workflow_pb";
import {
  useCancelWorkflow,
  useGetServerInfo,
  useHealthCheck,
  useListWorkflows,
  useStartWorkflow,
} from "@/hooks/use-grpc-services";
import { timestampDate } from "@bufbuild/protobuf/wkt";
import { useState } from "react";

/**
 * Example component demonstrating gRPC service usage
 * This shows how to:
 * - Query data with useQuery hooks
 * - Mutate data with useMutation hooks
 * - Handle loading and error states
 * - Display workflow information
 */
export function WorkflowExample() {
  const [namespace] = useState("default");

  // Health check query - runs automatically
  const { data: health, isLoading: healthLoading } = useHealthCheck();

  // Server info query
  const { data: serverInfo } = useGetServerInfo();

  // List workflows query
  const {
    data: workflows,
    isLoading: workflowsLoading,
    error: workflowsError,
    refetch: refetchWorkflows,
  } = useListWorkflows({
    namespace: namespace,
    page: { pageSize: 10, pageToken: "", includeTotalCount: false },
  });

  // Start workflow mutation
  const startWorkflow = useStartWorkflow();

  // Cancel workflow mutation
  const cancelWorkflow = useCancelWorkflow();

  const handleStartWorkflow = async () => {
    try {
      const result = await startWorkflow.mutateAsync({
        externalId: `example-${Date.now()}`,
        namespaceId: namespace,
        taskQueue: "default",
        workflowType: "example_workflow",
        input: {
          data: new TextEncoder().encode(JSON.stringify({ message: "Hello from UI!" })),
        },
        version: "1.0.0",
      });

      console.log("Workflow started:", result);
      alert(`Workflow started! Run ID: ${result.runId}`);
    } catch (error) {
      console.error("Failed to start workflow:", error);
      alert("Failed to start workflow. Check console for details.");
    }
  };

  const handleCancelWorkflow = async (runId: string) => {
    try {
      await cancelWorkflow.mutateAsync({
        runId,
        namespaceId: namespace,
      });
      alert("Workflow cancelled!");
    } catch (error) {
      console.error("Failed to cancel workflow:", error);
      alert("Failed to cancel workflow. Check console for details.");
    }
  };

  const getStatusColor = (status: WorkflowStatus): string => {
    switch (status) {
      case WorkflowStatus.COMPLETED:
        return "text-green-600";
      case WorkflowStatus.FAILED:
        return "text-red-600";
      case WorkflowStatus.RUNNING:
        return "text-blue-600";
      case WorkflowStatus.CANCELLED:
        return "text-gray-600";
      default:
        return "text-yellow-600";
    }
  };

  const getStatusText = (status: WorkflowStatus): string => {
    return WorkflowStatus[status] || "UNKNOWN";
  };

  return (
    <div className="p-6 max-w-6xl mx-auto">
      <h1 className="text-3xl font-bold mb-6">Kagzi gRPC Example</h1>

      {/* Server Status */}
      <div className="mb-6 p-4 bg-gray-50 rounded-lg">
        <h2 className="text-xl font-semibold mb-2">Server Status</h2>
        {healthLoading ? (
          <p>Checking server health...</p>
        ) : health ? (
          <div>
            <p className="text-green-600">✓ Server is healthy</p>
            {serverInfo && (
              <div className="mt-2 text-sm text-gray-600">
                <p>Version: {serverInfo.version}</p>
                <p>API Version: {serverInfo.apiVersion}</p>
              </div>
            )}
          </div>
        ) : (
          <p className="text-red-600">✗ Cannot connect to server</p>
        )}
      </div>

      {/* Actions */}
      <div className="mb-6">
        <h2 className="text-xl font-semibold mb-3">Actions</h2>
        <div className="flex gap-3">
          <button
            onClick={handleStartWorkflow}
            disabled={startWorkflow.isPending}
            className="px-4 py-2 bg-blue-600 text-white rounded hover:bg-blue-700 disabled:opacity-50"
          >
            {startWorkflow.isPending ? "Starting..." : "Start Example Workflow"}
          </button>

          <button
            onClick={() => refetchWorkflows()}
            disabled={workflowsLoading}
            className="px-4 py-2 bg-gray-600 text-white rounded hover:bg-gray-700 disabled:opacity-50"
          >
            {workflowsLoading ? "Refreshing..." : "Refresh List"}
          </button>
        </div>
      </div>

      {/* Workflows List */}
      <div>
        <h2 className="text-xl font-semibold mb-3">Workflows</h2>

        {workflowsError && (
          <div className="p-4 bg-red-50 border border-red-200 rounded-lg text-red-700 mb-4">
            Error loading workflows: {workflowsError.message}
          </div>
        )}

        {workflowsLoading ? (
          <p>Loading workflows...</p>
        ) : workflows?.workflows && workflows.workflows.length > 0 ? (
          <div className="space-y-3">
            {workflows.workflows.map((workflow) => (
              <div
                key={workflow.runId}
                className="p-4 border border-gray-200 rounded-lg hover:shadow-md transition-shadow"
              >
                <div className="flex justify-between items-start">
                  <div className="flex-1">
                    <div className="flex items-center gap-3 mb-2">
                      <h3 className="font-semibold">{workflow.workflowType}</h3>
                      <span className={`text-sm font-medium ${getStatusColor(workflow.status)}`}>
                        {getStatusText(workflow.status)}
                      </span>
                    </div>

                    <div className="text-sm text-gray-600 space-y-1">
                      <p>Run ID: {workflow.runId}</p>
                      <p>External ID: {workflow.externalId}</p>
                      <p>Task Queue: {workflow.taskQueue}</p>
                      {workflow.createdAt && (
                        <p>Created: {timestampDate(workflow.createdAt).toLocaleString()}</p>
                      )}
                      {workflow.error && (
                        <p className="text-red-600">Error: {workflow.error.message}</p>
                      )}
                    </div>
                  </div>

                  {workflow.status === WorkflowStatus.RUNNING && (
                    <button
                      onClick={() => handleCancelWorkflow(workflow.runId)}
                      disabled={cancelWorkflow.isPending}
                      className="px-3 py-1 bg-red-600 text-white text-sm rounded hover:bg-red-700 disabled:opacity-50"
                    >
                      Cancel
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-gray-500">No workflows found. Start one to see it here!</p>
        )}
      </div>

      {/* Usage Instructions */}
      <div className="mt-8 p-4 bg-blue-50 border border-blue-200 rounded-lg">
        <h3 className="font-semibold text-blue-900 mb-2">How to use this example:</h3>
        <ol className="text-sm text-blue-800 space-y-1 list-decimal list-inside">
          <li>
            Make sure the Kagzi server is running (<code>just dev</code>)
          </li>
          <li>Click "Start Example Workflow" to create a workflow</li>
          <li>Watch it appear in the list below</li>
          <li>Click "Cancel" on running workflows to stop them</li>
          <li>Click "Refresh List" to manually update the list</li>
        </ol>
      </div>
    </div>
  );
}
