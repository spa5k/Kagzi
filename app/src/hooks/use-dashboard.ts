import { WorkflowStatus } from "@/gen/workflow_pb";
import { adminClient, scheduleClient, workflowClient } from "@/lib/api-client";
import { useQuery } from "@tanstack/react-query";

const NAMESPACE_ID = "default";

export function useWorkflows(statusFilter?: WorkflowStatus) {
  return useQuery({
    queryKey: ["workflows", NAMESPACE_ID, statusFilter],
    queryFn: async () => {
      const response = await workflowClient.listWorkflows({
        namespaceId: NAMESPACE_ID,
        statusFilter,
      });
      return response.workflows;
    },
    refetchInterval: 5000,
    retry: 3,
    retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
  });
}

export function useSchedules() {
  return useQuery({
    queryKey: ["schedules", NAMESPACE_ID],
    queryFn: async () => {
      const response = await scheduleClient.listWorkflowSchedules({
        namespaceId: NAMESPACE_ID,
      });
      return response.schedules;
    },
    refetchInterval: 10000,
    retry: 3,
    retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
  });
}

export function useWorkers() {
  return useQuery({
    queryKey: ["workers", NAMESPACE_ID],
    queryFn: async () => {
      const response = await adminClient.listWorkers({
        namespaceId: NAMESPACE_ID,
      });
      return response.workers;
    },
    refetchInterval: 15000,
    retry: 3,
    retryDelay: (attemptIndex) => Math.min(1000 * 2 ** attemptIndex, 30000),
  });
}
