import { AdminService } from "@/gen/admin_connect";
import {
  GetServerInfoRequest,
  GetStepRequest,
  GetWorkerRequest,
  HealthCheckRequest,
  ListStepsRequest,
  ListWorkersRequest,
} from "@/gen/admin_pb";
import { WorkerService } from "@/gen/worker_connect";
import { WorkflowService } from "@/gen/workflow_connect";
import type { GetWorkflowRequest, ListWorkflowsRequest } from "@/gen/workflow_pb";
import { WorkflowScheduleService } from "@/gen/workflow_schedule_connect";
import {
  GetWorkflowScheduleRequest,
  ListWorkflowSchedulesRequest,
} from "@/gen/workflow_schedule_pb";
import { getGrpcTransport } from "@/lib/grpc-client";
import { createPromiseClient } from "@connectrpc/connect";
import {
  useMutation as useTanstackMutation,
  useQuery as useTanstackQuery,
  useQueryClient,
} from "@tanstack/react-query";

// Create clients for each service
const transport = getGrpcTransport();
const workflowClient = createPromiseClient(WorkflowService, transport);
const adminClient = createPromiseClient(AdminService, transport);
const scheduleClient = createPromiseClient(WorkflowScheduleService, transport);

// ============================================
// WORKFLOW SERVICE HOOKS
// ============================================

/**
 * Hook to start a new workflow
 */
export function useStartWorkflow() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: Parameters<typeof workflowClient.startWorkflow>[0]) =>
      workflowClient.startWorkflow(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
  });
}

/**
 * Hook to get a workflow by run_id
 */
export function useGetWorkflow(request: GetWorkflowRequest) {
  return useTanstackQuery({
    queryKey: ["workflow", request.runId, request.namespaceId],
    queryFn: () => workflowClient.getWorkflow(request),
    enabled: !!request.runId && !!request.namespaceId,
  });
}

/**
 * Hook to list workflows
 */
export function useListWorkflows(request: ListWorkflowsRequest) {
  return useTanstackQuery({
    queryKey: ["workflows", request.namespaceId, request.statusFilter],
    queryFn: () => workflowClient.listWorkflows(request),
    enabled: !!request.namespaceId,
  });
}

/**
 * Hook to cancel a workflow
 */
export function useCancelWorkflow() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: Parameters<typeof workflowClient.cancelWorkflow>[0]) =>
      workflowClient.cancelWorkflow(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["workflow", variables.runId] });
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
  });
}

// ============================================
// ADMIN SERVICE HOOKS
// ============================================

/**
 * Hook to check server health
 */
export function useHealthCheck(request?: HealthCheckRequest) {
  return useTanstackQuery({
    queryKey: ["health"],
    queryFn: () => adminClient.healthCheck(request || new HealthCheckRequest()),
  });
}

/**
 * Hook to get server info
 */
export function useGetServerInfo(request?: GetServerInfoRequest) {
  return useTanstackQuery({
    queryKey: ["serverInfo"],
    queryFn: () => adminClient.getServerInfo(request || new GetServerInfoRequest()),
  });
}

/**
 * Hook to list workers
 */
export function useListWorkers(request: ListWorkersRequest) {
  return useTanstackQuery({
    queryKey: ["workers", request.namespaceId, request.taskQueue],
    queryFn: () => adminClient.listWorkers(request),
    enabled: !!request.namespaceId,
  });
}

/**
 * Hook to get a specific worker
 */
export function useGetWorker(request: GetWorkerRequest) {
  return useTanstackQuery({
    queryKey: ["worker", request.workerId],
    queryFn: () => adminClient.getWorker(request),
    enabled: !!request.workerId,
  });
}

/**
 * Hook to get a specific step
 */
export function useGetStep(request: GetStepRequest) {
  return useTanstackQuery({
    queryKey: ["step", request.stepId, request.namespaceId],
    queryFn: () => adminClient.getStep(request),
    enabled: !!request.stepId && !!request.namespaceId,
  });
}

/**
 * Hook to list steps for a workflow run
 */
export function useListSteps(request: ListStepsRequest) {
  return useTanstackQuery({
    queryKey: ["steps", request.runId],
    queryFn: () => adminClient.listSteps(request),
    enabled: !!request.runId,
  });
}

// ============================================
// SCHEDULE SERVICE HOOKS
// ============================================

/**
 * Hook to create a new schedule
 */
export function useCreateSchedule() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: Parameters<typeof scheduleClient.createWorkflowSchedule>[0]) =>
      scheduleClient.createWorkflowSchedule(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
    },
  });
}

/**
 * Hook to update a schedule
 */
export function useUpdateSchedule() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: Parameters<typeof scheduleClient.updateWorkflowSchedule>[0]) =>
      scheduleClient.updateWorkflowSchedule(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["schedule", variables.scheduleId] });
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
    },
  });
}

/**
 * Hook to delete a schedule
 */
export function useDeleteSchedule() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: Parameters<typeof scheduleClient.deleteWorkflowSchedule>[0]) =>
      scheduleClient.deleteWorkflowSchedule(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
    },
  });
}

/**
 * Hook to get a schedule by ID
 */
export function useGetSchedule(request: GetWorkflowScheduleRequest) {
  return useTanstackQuery({
    queryKey: ["schedule", request.scheduleId, request.namespaceId],
    queryFn: () => scheduleClient.getWorkflowSchedule(request),
    enabled: !!request.scheduleId && !!request.namespaceId,
  });
}

/**
 * Hook to list schedules
 */
export function useListSchedules(request: ListWorkflowSchedulesRequest) {
  return useTanstackQuery({
    queryKey: ["schedules", request.namespaceId, request.taskQueue],
    queryFn: () => scheduleClient.listWorkflowSchedules(request),
    enabled: !!request.namespaceId,
  });
}

// ============================================
// WORKER SERVICE HOOKS
// ============================================
// Note: Worker service is typically used by workers themselves,
// not by the admin UI, but included for completeness

/**
 * Export all service definitions for advanced use cases
 */
export const services = {
  workflow: WorkflowService,
  admin: AdminService,
  schedule: WorkflowScheduleService,
  worker: WorkerService,
};
