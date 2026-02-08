import {
  AdminService,
  type DrainWorkerRequest,
  type GetQueueDepthRequest,
  type GetServerInfoRequest,
  GetServerInfoRequestSchema,
  type GetStatsRequest,
  GetStatsRequestSchema,
  type GetStepRequest,
  type GetWorkerRequest,
  type HealthCheckRequest,
  HealthCheckRequestSchema,
  type ListStepsRequest,
  type ListWorkersRequest,
  type ListWorkflowTypesRequest,
} from "@/gen/admin_pb";
import {
  NamespaceService,
  type CreateNamespaceRequest,
  type DisableNamespaceRequest,
  type EnableNamespaceRequest,
  type GetNamespaceRequest,
  type ListNamespacesRequest,
  ListNamespacesRequestSchema,
  type UpdateNamespaceRequest,
} from "@/gen/namespace_pb";
import { WorkerService } from "@/gen/worker_pb";
import {
  WorkflowService,
  type GetWorkflowByExternalIdRequest,
  type GetWorkflowRequest,
  type ListWorkflowsRequest,
  type RetryWorkflowRequest,
  type TerminateWorkflowRequest,
} from "@/gen/workflow_pb";
import {
  WorkflowScheduleService,
  type GetWorkflowScheduleRequest,
  type ListScheduleRunsRequest,
  type ListWorkflowSchedulesRequest,
  type PauseWorkflowScheduleRequest,
  type ResumeWorkflowScheduleRequest,
  type TriggerWorkflowScheduleRequest,
} from "@/gen/workflow_schedule_pb";
import { getGrpcTransport } from "@/lib/grpc-client";
import { createClient } from "@connectrpc/connect";
import { create } from "@bufbuild/protobuf";
import {
  useMutation as useTanstackMutation,
  useQuery as useTanstackQuery,
  useQueryClient,
} from "@tanstack/react-query";

// Create clients for each service
const transport = getGrpcTransport();
const workflowClient = createClient(WorkflowService, transport);
const adminClient = createClient(AdminService, transport);
const scheduleClient = createClient(WorkflowScheduleService, transport);
const namespaceClient = createClient(NamespaceService, transport);

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
    queryKey: ["workflow", request.runId, request.namespace],
    queryFn: () => workflowClient.getWorkflow(request),
    enabled: !!request.runId && !!request.namespace,
  });
}

/**
 * Hook to list workflows
 */
export function useListWorkflows(request: ListWorkflowsRequest) {
  return useTanstackQuery({
    queryKey: ["workflows", request.namespace, request.statusFilter],
    queryFn: () => workflowClient.listWorkflows(request),
    enabled: !!request.namespace,
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

/**
 * Hook to get a workflow by external_id (idempotency key)
 */
export function useGetWorkflowByExternalId(request: GetWorkflowByExternalIdRequest) {
  return useTanstackQuery({
    queryKey: ["workflow", "externalId", request.externalId, request.namespace],
    queryFn: () => workflowClient.getWorkflowByExternalId(request),
    enabled: !!request.externalId && !!request.namespace,
  });
}

/**
 * Hook to retry a failed workflow
 */
export function useRetryWorkflow() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: RetryWorkflowRequest) => workflowClient.retryWorkflow(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["workflow", variables.runId] });
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
  });
}

/**
 * Hook to terminate a workflow forcefully
 */
export function useTerminateWorkflow() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: TerminateWorkflowRequest) => workflowClient.terminateWorkflow(request),
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
    queryFn: () => adminClient.healthCheck(request || create(HealthCheckRequestSchema)),
  });
}

/**
 * Hook to get server info
 */
export function useGetServerInfo(request?: GetServerInfoRequest) {
  return useTanstackQuery({
    queryKey: ["serverInfo"],
    queryFn: () => adminClient.getServerInfo(request || create(GetServerInfoRequestSchema)),
  });
}

/**
 * Hook to list workers
 */
export function useListWorkers(request: ListWorkersRequest) {
  return useTanstackQuery({
    queryKey: ["workers", request.namespace, request.taskQueue],
    queryFn: () => adminClient.listWorkers(request),
    enabled: !!request.namespace,
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
    queryKey: ["step", request.stepId, request.namespace],
    queryFn: () => adminClient.getStep(request),
    enabled: !!request.stepId && !!request.namespace,
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

/**
 * Hook to get aggregate statistics about workflows and workers
 */
export function useGetStats(request?: GetStatsRequest) {
  return useTanstackQuery({
    queryKey: ["stats", request?.namespace],
    queryFn: () => adminClient.getStats(request || create(GetStatsRequestSchema)),
    refetchInterval: 30000, // Refresh every 30 seconds
  });
}

/**
 * Hook to get queue depth (pending/running task counts)
 */
export function useGetQueueDepth(request: GetQueueDepthRequest) {
  return useTanstackQuery({
    queryKey: ["queueDepth", request.namespace, request.taskQueue],
    queryFn: () => adminClient.getQueueDepth(request),
    enabled: !!request.namespace,
    refetchInterval: 10000, // Refresh every 10 seconds
  });
}

/**
 * Hook to list workflow types with statistics
 */
export function useListWorkflowTypes(request: ListWorkflowTypesRequest) {
  return useTanstackQuery({
    queryKey: ["workflowTypes", request.namespace],
    queryFn: () => adminClient.listWorkflowTypes(request),
    enabled: !!request.namespace,
  });
}

/**
 * Hook to drain a worker (graceful shutdown)
 */
export function useDrainWorker() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: DrainWorkerRequest) => adminClient.drainWorker(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["worker", variables.workerId] });
      queryClient.invalidateQueries({ queryKey: ["workers"] });
    },
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
    queryKey: ["schedule", request.scheduleId, request.namespace],
    queryFn: () => scheduleClient.getWorkflowSchedule(request),
    enabled: !!request.scheduleId && !!request.namespace,
  });
}

/**
 * Hook to list schedules
 */
export function useListSchedules(request: ListWorkflowSchedulesRequest) {
  return useTanstackQuery({
    queryKey: ["schedules", request.namespace, request.taskQueue],
    queryFn: () => scheduleClient.listWorkflowSchedules(request),
    enabled: !!request.namespace,
  });
}

/**
 * Hook to trigger a schedule immediately
 */
export function useTriggerSchedule() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: TriggerWorkflowScheduleRequest) =>
      scheduleClient.triggerWorkflowSchedule(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["schedule", variables.scheduleId] });
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
      queryClient.invalidateQueries({ queryKey: ["workflows"] });
    },
  });
}

/**
 * Hook to pause a schedule
 */
export function usePauseSchedule() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: PauseWorkflowScheduleRequest) =>
      scheduleClient.pauseWorkflowSchedule(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["schedule", variables.scheduleId] });
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
    },
  });
}

/**
 * Hook to resume a paused schedule
 */
export function useResumeSchedule() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: ResumeWorkflowScheduleRequest) =>
      scheduleClient.resumeWorkflowSchedule(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["schedule", variables.scheduleId] });
      queryClient.invalidateQueries({ queryKey: ["schedules"] });
    },
  });
}

/**
 * Hook to list workflow runs triggered by a schedule
 */
export function useListScheduleRuns(request: ListScheduleRunsRequest) {
  return useTanstackQuery({
    queryKey: ["scheduleRuns", request.scheduleId, request.namespace],
    queryFn: () => scheduleClient.listScheduleRuns(request),
    enabled: !!request.scheduleId && !!request.namespace,
  });
}

// ============================================
// WORKER SERVICE HOOKS
// ============================================
// Note: Worker service is typically used by workers themselves,
// not by the admin UI, but included for completeness

// ============================================
// NAMESPACE SERVICE HOOKS
// ============================================

/**
 * Hook to list all namespaces
 */
export function useListNamespaces(request?: ListNamespacesRequest) {
  return useTanstackQuery({
    queryKey: ["namespaces"],
    queryFn: () => namespaceClient.listNamespaces(request || create(ListNamespacesRequestSchema)),
  });
}

/**
 * Hook to get a namespace by identifier
 */
export function useGetNamespace(request: GetNamespaceRequest) {
  return useTanstackQuery({
    queryKey: ["namespace", request.namespace],
    queryFn: () => namespaceClient.getNamespace(request),
    enabled: !!request.namespace,
  });
}

/**
 * Hook to create a new namespace
 */
export function useCreateNamespace() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: CreateNamespaceRequest) => namespaceClient.createNamespace(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["namespaces"] });
    },
  });
}

/**
 * Hook to update a namespace
 */
export function useUpdateNamespace() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: UpdateNamespaceRequest) => namespaceClient.updateNamespace(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["namespace", variables.namespace] });
      queryClient.invalidateQueries({ queryKey: ["namespaces"] });
    },
  });
}

/**
 * Hook to enable a namespace
 */
export function useEnableNamespace() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: EnableNamespaceRequest) => namespaceClient.enableNamespace(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["namespace", variables.namespace] });
      queryClient.invalidateQueries({ queryKey: ["namespaces"] });
    },
  });
}

/**
 * Hook to disable a namespace
 */
export function useDisableNamespace() {
  const queryClient = useQueryClient();

  return useTanstackMutation({
    mutationFn: (request: DisableNamespaceRequest) => namespaceClient.disableNamespace(request),
    onSuccess: (_, variables) => {
      queryClient.invalidateQueries({ queryKey: ["namespace", variables.namespace] });
      queryClient.invalidateQueries({ queryKey: ["namespaces"] });
    },
  });
}

/**
 * Export all service definitions for advanced use cases
 */
export const services = {
  workflow: WorkflowService,
  admin: AdminService,
  schedule: WorkflowScheduleService,
  worker: WorkerService,
  namespace: NamespaceService,
};
