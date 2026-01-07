import {
  useListWorkflows as useGrpcListWorkflows,
  useListSchedules as useGrpcListSchedules,
  useListWorkers as useGrpcListWorkers,
} from "@/hooks/use-grpc-services";
import { WorkflowStatus as ProtoWorkflowStatus } from "@/gen/workflow_pb";

/**
 * Hook to list workflows with optional status filter
 */
export function useListWorkflows(namespaceId: string, statusFilter?: string) {
  // Convert string status filter to proto enum if provided
  let protoStatusFilter: ProtoWorkflowStatus | undefined;
  if (statusFilter) {
    const statusMap: Record<string, ProtoWorkflowStatus> = {
      pending: ProtoWorkflowStatus.PENDING,
      running: ProtoWorkflowStatus.RUNNING,
      sleeping: ProtoWorkflowStatus.SLEEPING,
      completed: ProtoWorkflowStatus.COMPLETED,
      failed: ProtoWorkflowStatus.FAILED,
      cancelled: ProtoWorkflowStatus.CANCELLED,
      scheduled: ProtoWorkflowStatus.SCHEDULED,
      paused: ProtoWorkflowStatus.PAUSED,
    };
    protoStatusFilter = statusMap[statusFilter.toLowerCase()];
  }

  const result = useGrpcListWorkflows({
    namespaceId,
    statusFilter: protoStatusFilter,
    page: {
      pageSize: 100,
      pageToken: "",
      includeTotalCount: false,
    },
  });

  return {
    ...result,
    data: result.data?.workflows || [],
  };
}

/**
 * Hook to list schedules
 */
export function useListSchedules(namespaceId: string) {
  const result = useGrpcListSchedules({
    namespaceId,
    page: {
      pageSize: 100,
      pageToken: "",
      includeTotalCount: false,
    },
  });

  return {
    ...result,
    data: {
      schedulesList: result.data?.schedules || [],
    },
  };
}

/**
 * Hook to list workers
 */
export function useListWorkers(namespaceId: string) {
  const result = useGrpcListWorkers({
    namespaceId,
    page: {
      pageSize: 100,
      pageToken: "",
      includeTotalCount: false,
    },
  });

  return {
    ...result,
    data: {
      workersList: result.data?.workers || [],
    },
  };
}
