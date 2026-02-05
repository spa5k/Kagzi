import { WorkflowStatus as ProtoWorkflowStatus } from "@/gen/workflow_pb";
import {
  useListSchedules as useGrpcListSchedules,
  useListWorkers as useGrpcListWorkers,
  useListWorkflows as useGrpcListWorkflows,
  useGetWorkerTelemetryState as useGrpcGetWorkerTelemetryState,
  useGetQueueTelemetryState as useGrpcGetQueueTelemetryState,
  useListQueues as useGrpcListQueues,
  useListQueueTelemetryStates as useGrpcListQueueTelemetryStates,
  useListServerTelemetryEvents as useGrpcListServerTelemetryEvents,
  useListWorkerTelemetryEvents as useGrpcListWorkerTelemetryEvents,
  useListWorkerTelemetryStates as useGrpcListWorkerTelemetryStates,
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

  const request = {
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
  const request = {
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
  const request = {
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

export function useListWorkerTelemetryStates(namespace: string, taskQueue?: string) {
  const request = {
    namespace,
    taskQueue,
    page: {
      pageSize: 200,
      pageToken: "",
      includeTotalCount: false,
    },
  };
  return useGrpcListWorkerTelemetryStates(request);
}

export function useGetWorkerTelemetryState(workerId: string) {
  const request = { workerId };
  return useGrpcGetWorkerTelemetryState(request);
}

export function useListWorkerTelemetryEvents(namespace: string, workerId: string) {
  const request = {
    namespace,
    workerId,
    page: {
      pageSize: 200,
      pageToken: "",
      includeTotalCount: false,
    },
  };
  return useGrpcListWorkerTelemetryEvents(request);
}

export function useListQueueTelemetryStates(namespace: string) {
  const request = {
    namespace,
    page: {
      pageSize: 200,
      pageToken: "",
      includeTotalCount: false,
    },
  };
  return useGrpcListQueueTelemetryStates(request);
}

export function useListQueues(namespace: string) {
  const request = {
    namespace,
    page: {
      pageSize: 200,
      pageToken: "",
      includeTotalCount: false,
    },
  };
  return useGrpcListQueues(request);
}

export function useGetQueueTelemetryState(namespace: string, taskQueue: string) {
  const request = {
    namespace,
    taskQueue,
  };
  return useGrpcGetQueueTelemetryState(request);
}

export function useListServerTelemetryEvents(namespace?: string) {
  const request = {
    namespace,
    page: {
      pageSize: 200,
      pageToken: "",
      includeTotalCount: false,
    },
  };
  return useGrpcListServerTelemetryEvents(request);
}
