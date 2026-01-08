import type { ListWorkersRequest } from "@/gen/admin_pb";
import type { PageRequest } from "@/gen/common_pb";
import {
  type ListWorkflowsRequest,
  WorkflowStatus as ProtoWorkflowStatus,
} from "@/gen/workflow_pb";
import type { ListWorkflowSchedulesRequest } from "@/gen/workflow_schedule_pb";
import {
  useListSchedules as useGrpcListSchedules,
  useListWorkers as useGrpcListWorkers,
  useListWorkflows as useGrpcListWorkflows,
} from "@/hooks/use-grpc-services";

/**
 * Hook to list workflows with optional status filter
 */
export function useListWorkflows(namespace: string, statusFilter?: string) {
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

  const request: ListWorkflowsRequest = {
    namespace,
    statusFilter: protoStatusFilter,
    page: {
      pageSize: 100,
      pageToken: "",
      includeTotalCount: false,
    },
  };

  const result = useGrpcListWorkflows(request);

  return {
    ...result,
    data: result.data?.workflows || [],
  };
}

/**
 * Hook to list schedules
 */
export function useListSchedules(namespace: string) {
  const request: ListWorkflowSchedulesRequest = {
    namespace,
    page: {
      pageSize: 100,
      pageToken: "",
      includeTotalCount: false,
    },
  };

  const result = useGrpcListSchedules(request);

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
export function useListWorkers(namespace: string) {
  const request: ListWorkersRequest = {
    namespace,
    page: {
      pageSize: 100,
      pageToken: "",
      includeTotalCount: false,
    },
  };

  const result = useGrpcListWorkers(request);

  return {
    ...result,
    data: {
      workersList: result.data?.workers || [],
    },
  };
}
