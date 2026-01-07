// Re-export all protobuf types for convenience
export type { Workflow, WorkflowStatus as WorkflowStatusEnum } from "@/gen/workflow_pb";
export type {
  Worker,
  WorkerStatus as WorkerStatusEnum,
  Step,
  StepKind as StepKindEnum,
  StepStatus as StepStatusEnum,
} from "@/gen/worker_pb";
export type { WorkflowSchedule } from "@/gen/workflow_schedule_pb";
export type {
  ServingStatus as ServingStatusEnum,
  WorkflowTypeInfo,
  GetStatsResponse,
  GetQueueDepthResponse,
  DrainWorkerResponse,
} from "@/gen/admin_pb";

// Import and export the proto enums as values
import { WorkflowStatus as ProtoWorkflowStatus } from "@/gen/workflow_pb";
import { WorkerStatus as ProtoWorkerStatus, StepStatus, StepKind } from "@/gen/worker_pb";
import { ServingStatus } from "@/gen/admin_pb";

// Export the proto enums
export const WorkflowStatus = ProtoWorkflowStatus;
export const WorkerStatus = ProtoWorkerStatus;
export { StepStatus, StepKind, ServingStatus };

// Label mapping for workflow statuses (includes all enum values)
export const WorkflowStatusLabel: Record<number, string> = {
  [ProtoWorkflowStatus.UNSPECIFIED]: "Unspecified",
  [ProtoWorkflowStatus.PENDING]: "Pending",
  [ProtoWorkflowStatus.RUNNING]: "Running",
  [ProtoWorkflowStatus.SLEEPING]: "Sleeping",
  [ProtoWorkflowStatus.COMPLETED]: "Completed",
  [ProtoWorkflowStatus.FAILED]: "Failed",
  [ProtoWorkflowStatus.CANCELLED]: "Cancelled",
  [ProtoWorkflowStatus.SCHEDULED]: "Scheduled",
  [ProtoWorkflowStatus.PAUSED]: "Paused",
};

// Label mapping for worker statuses
export const WorkerStatusLabel: Record<number, string> = {
  [ProtoWorkerStatus.UNSPECIFIED]: "Unspecified",
  [ProtoWorkerStatus.ONLINE]: "Online",
  [ProtoWorkerStatus.DRAINING]: "Draining",
  [ProtoWorkerStatus.OFFLINE]: "Offline",
};

// Label mapping for step statuses
export const StepStatusLabel: Record<number, string> = {
  [StepStatus.UNSPECIFIED]: "Unspecified",
  [StepStatus.PENDING]: "Pending",
  [StepStatus.RUNNING]: "Running",
  [StepStatus.COMPLETED]: "Completed",
  [StepStatus.FAILED]: "Failed",
};

// Label mapping for step kinds
export const StepKindLabel: Record<number, string> = {
  [StepKind.UNSPECIFIED]: "Unspecified",
  [StepKind.FUNCTION]: "Function",
  [StepKind.SLEEP]: "Sleep",
  [StepKind.CHILD_WORKFLOW]: "Child Workflow",
};

// Label mapping for serving status
export const ServingStatusLabel: Record<number, string> = {
  [ServingStatus.UNSPECIFIED]: "Unspecified",
  [ServingStatus.SERVING]: "Serving",
  [ServingStatus.NOT_SERVING]: "Not Serving",
};

// Namespace type (not in protobufs, used for mock data)
export interface Namespace {
  id: string;
  name: string;
  description?: string;
  createdAt: string;
}

// Type aliases for enum values
export type WorkflowStatusType = ProtoWorkflowStatus;
export type WorkerStatusType = ProtoWorkerStatus;
export type StepStatusType = StepStatus;
export type StepKindType = StepKind;
