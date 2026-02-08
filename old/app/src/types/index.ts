export const WorkflowStatus = {
  UNSPECIFIED: 0,
  PENDING: 1,
  RUNNING: 2,
  SLEEPING: 3,
  COMPLETED: 4,
  FAILED: 5,
  CANCELLED: 6,
  SCHEDULED: 7,
  PAUSED: 8,
} as const;

export type WorkflowStatusType = (typeof WorkflowStatus)[keyof typeof WorkflowStatus];

export const WorkflowStatusLabel: Record<WorkflowStatusType, string> = {
  [WorkflowStatus.UNSPECIFIED]: "Unknown",
  [WorkflowStatus.PENDING]: "Pending",
  [WorkflowStatus.RUNNING]: "Running",
  [WorkflowStatus.SLEEPING]: "Sleeping",
  [WorkflowStatus.COMPLETED]: "Completed",
  [WorkflowStatus.FAILED]: "Failed",
  [WorkflowStatus.CANCELLED]: "Cancelled",
  [WorkflowStatus.SCHEDULED]: "Scheduled",
  [WorkflowStatus.PAUSED]: "Paused",
};

export const StepStatus = {
  UNSPECIFIED: 0,
  PENDING: 1,
  RUNNING: 2,
  COMPLETED: 3,
  FAILED: 4,
} as const;

export type StepStatusType = (typeof StepStatus)[keyof typeof StepStatus];

export const StepStatusLabel: Record<StepStatusType, string> = {
  [StepStatus.UNSPECIFIED]: "Unknown",
  [StepStatus.PENDING]: "Pending",
  [StepStatus.RUNNING]: "Running",
  [StepStatus.COMPLETED]: "Completed",
  [StepStatus.FAILED]: "Failed",
};

export const StepKind = {
  UNSPECIFIED: 0,
  FUNCTION: 1,
  SLEEP: 2,
} as const;

export type StepKindType = (typeof StepKind)[keyof typeof StepKind];

export const WorkerStatus = {
  UNSPECIFIED: 0,
  ONLINE: 1,
  DRAINING: 2,
  OFFLINE: 3,
} as const;

export type WorkerStatusType = (typeof WorkerStatus)[keyof typeof WorkerStatus];

export const WorkerStatusLabel: Record<WorkerStatusType, string> = {
  [WorkerStatus.UNSPECIFIED]: "Unknown",
  [WorkerStatus.ONLINE]: "Online",
  [WorkerStatus.DRAINING]: "Draining",
  [WorkerStatus.OFFLINE]: "Offline",
};

export interface Payload {
  data: string;
  metadata: Record<string, string>;
}

export interface ErrorDetail {
  code: number;
  message: string;
  nonRetryable: boolean;
  retryAfterMs: number;
  subject: string;
  subjectId: string;
  metadata: Record<string, string>;
}

export interface Workflow {
  runId: string;
  externalId: string;
  namespaceId: string;
  taskQueue: string;
  workflowType: string;
  status: WorkflowStatusType;
  input: Payload | null;
  output: Payload | null;
  error: ErrorDetail | null;
  attempts: number;
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  wakeUpAt: string | null;
  workerId: string | null;
  version: string;
  parentStepId: string | null;
  cronExpr: string | null;
  scheduleId: string | null;
}

export interface Step {
  stepId: string;
  runId: string;
  namespaceId: string;
  name: string;
  kind: StepKindType;
  status: StepStatusType;
  attemptNumber: number;
  input: Payload | null;
  output: Payload | null;
  error: ErrorDetail | null;
  createdAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  childRunId: string | null;
}

export interface WorkflowSchedule {
  scheduleId: string;
  namespaceId: string;
  taskQueue: string;
  workflowType: string;
  cronExpr: string;
  input: Payload | null;
  enabled: boolean;
  maxCatchup: number;
  nextFireAt: string | null;
  lastFiredAt: string | null;
  version: string;
  createdAt: string;
  updatedAt: string;
}

export interface Worker {
  workerId: string;
  namespaceId: string;
  taskQueue: string;
  status: WorkerStatusType;
  hostname: string;
  pid: number;
  version: string;
  workflowTypes: string[];
  registeredAt: string;
  lastHeartbeatAt: string;
  labels: Record<string, string>;
  queueConcurrencyLimit: number | null;
}

export interface Namespace {
  id: string;
  name: string;
}

export interface PageInfo {
  nextPageToken: string;
  hasMore: boolean;
  totalCount: number;
}

export interface PageRequest {
  pageSize: number;
  pageToken?: string;
  includeTotalCount?: boolean;
}
