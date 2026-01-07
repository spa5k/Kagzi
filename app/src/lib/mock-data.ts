import type { Namespace } from "@/types";
// Note: The other mock functions use `any` to avoid protobuf type complexity
// They're not currently used in the application
import { StepKind, StepStatus, WorkerStatus, WorkflowStatus } from "@/types";

const uuid = () => crypto.randomUUID();

const timestamp = (hoursAgo = 0) => {
  const date = new Date();
  date.setHours(date.getHours() - hoursAgo);
  return date.toISOString();
};

export const mockNamespaces: Namespace[] = [
  { id: "default", name: "default", createdAt: timestamp(720) },
  { id: "production", name: "production", createdAt: timestamp(2160) },
  { id: "staging", name: "staging", createdAt: timestamp(1440) },
];

const workflowTypes = [
  "order_workflow",
  "payment_workflow",
  "notification_workflow",
  "report_generator",
  "data_pipeline",
  "user_onboarding",
];

const taskQueues = ["default", "orders", "payments", "background"];

export function generateMockWorkflows(count: number): any[] {
  const statuses = [
    WorkflowStatus.COMPLETED,
    WorkflowStatus.COMPLETED,
    WorkflowStatus.COMPLETED,
    WorkflowStatus.RUNNING,
    WorkflowStatus.FAILED,
    WorkflowStatus.PENDING,
    WorkflowStatus.SLEEPING,
    WorkflowStatus.CANCELLED,
  ];

  return Array.from({ length: count }, (_, i) => {
    const status = statuses[i % statuses.length] ?? WorkflowStatus.PENDING;
    const hoursAgo = Math.random() * 48;
    const workflowType = workflowTypes[i % workflowTypes.length] ?? "workflow";
    const taskQueue = taskQueues[i % taskQueues.length] ?? "default";

    const isTerminal =
      status === WorkflowStatus.COMPLETED ||
      status === WorkflowStatus.FAILED ||
      status === WorkflowStatus.CANCELLED;

    return {
      runId: uuid(),
      externalId: `${workflowType}-${Date.now()}-${i}`,
      namespaceId: "default",
      taskQueue,
      workflowType,
      status,
      input: {
        data: btoa(JSON.stringify({ orderId: `ORD-${1000 + i}`, userId: `USR-${100 + i}` })),
        metadata: { "content-type": "application/json" },
      },
      output:
        status === WorkflowStatus.COMPLETED
          ? {
              data: btoa(JSON.stringify({ success: true, processedAt: timestamp() })),
              metadata: { "content-type": "application/json" },
            }
          : null,
      error:
        status === WorkflowStatus.FAILED
          ? {
              code: 7,
              message: "Payment gateway timeout after 3 retries",
              nonRetryable: false,
              retryAfterMs: 5000,
              subject: "step",
              subjectId: "process-payment",
              metadata: {},
            }
          : null,
      attempts: status === WorkflowStatus.FAILED ? 3 : 1,
      createdAt: timestamp(hoursAgo),
      startedAt: status !== WorkflowStatus.PENDING ? timestamp(hoursAgo - 0.01) : null,
      finishedAt: isTerminal ? timestamp(hoursAgo - 0.1) : null,
      wakeUpAt: status === WorkflowStatus.SLEEPING ? timestamp(-2) : null,
      workerId: status === WorkflowStatus.RUNNING ? `worker-${uuid().slice(0, 8)}` : null,
      version: "1.0.0",
      parentStepId: null,
      cronExpr: null,
      scheduleId: null,
    };
  });
}

export const mockWorkflows = generateMockWorkflows(25);

export function generateMockSteps(runId: string): any[] {
  const steps = [
    { name: "validate-input", durationMs: 50 },
    { name: "check-inventory", durationMs: 200 },
    { name: "process-payment", durationMs: 1500 },
    { name: "reserve-items", durationMs: 300 },
    { name: "send-confirmation", durationMs: 100 },
    { name: "await-fulfillment", durationMs: 0, isSleep: true },
  ];

  let currentTime = Date.now() - 5000;

  return steps.map((step, i) => {
    const startTime = currentTime;
    const endTime = step.isSleep ? null : currentTime + step.durationMs;
    currentTime = endTime ?? currentTime + 86400000;

    const isLast = i === steps.length - 1;
    const status = step.isSleep
      ? StepStatus.RUNNING
      : isLast
        ? StepStatus.PENDING
        : StepStatus.COMPLETED;

    return {
      stepId: uuid(),
      runId,
      namespaceId: "default",
      name: step.name,
      kind: step.isSleep ? StepKind.SLEEP : StepKind.FUNCTION,
      status,
      attemptNumber: 1,
      input: {
        data: btoa(JSON.stringify({ step: step.name })),
        metadata: { "content-type": "application/json" },
      },
      output:
        status === StepStatus.COMPLETED
          ? {
              data: btoa(JSON.stringify({ success: true })),
              metadata: { "content-type": "application/json" },
            }
          : null,
      error: null,
      createdAt: new Date(startTime).toISOString(),
      startedAt: new Date(startTime).toISOString(),
      finishedAt: endTime ? new Date(endTime).toISOString() : null,
      childRunId: null,
    };
  });
}

export const mockSchedules: any[] = [
  {
    scheduleId: uuid(),
    namespaceId: "default",
    taskQueue: "background",
    workflowType: "daily_report",
    cronExpr: "0 9 * * *",
    input: { data: btoa(JSON.stringify({ type: "daily" })), metadata: {} },
    enabled: true,
    maxCatchup: 3,
    nextFireAt: timestamp(-8),
    lastFiredAt: timestamp(16),
    version: "1.0.0",
    createdAt: timestamp(720),
    updatedAt: timestamp(24),
  },
  {
    scheduleId: uuid(),
    namespaceId: "default",
    taskQueue: "background",
    workflowType: "cleanup_workflow",
    cronExpr: "0 0 * * 0",
    input: { data: btoa(JSON.stringify({ daysToKeep: 30 })), metadata: {} },
    enabled: true,
    maxCatchup: 1,
    nextFireAt: timestamp(-120),
    lastFiredAt: timestamp(168),
    version: "1.0.0",
    createdAt: timestamp(2160),
    updatedAt: timestamp(168),
  },
  {
    scheduleId: uuid(),
    namespaceId: "default",
    taskQueue: "default",
    workflowType: "health_check",
    cronExpr: "*/5 * * * *",
    input: null,
    enabled: false,
    maxCatchup: 0,
    nextFireAt: null,
    lastFiredAt: timestamp(1),
    version: "1.0.0",
    createdAt: timestamp(48),
    updatedAt: timestamp(1),
  },
];

export const mockWorkers: any[] = [
  {
    workerId: uuid(),
    namespaceId: "default",
    taskQueue: "default",
    status: WorkerStatus.ONLINE,
    hostname: "worker-node-1.internal",
    pid: 12345,
    version: "1.0.0",
    workflowTypes: ["order_workflow", "payment_workflow", "notification_workflow"],
    registeredAt: timestamp(24),
    lastHeartbeatAt: timestamp(0.01),
    labels: { region: "us-east-1", instance: "c5.xlarge" },
    queueConcurrencyLimit: 10,
  },
  {
    workerId: uuid(),
    namespaceId: "default",
    taskQueue: "default",
    status: WorkerStatus.ONLINE,
    hostname: "worker-node-2.internal",
    pid: 12346,
    version: "1.0.0",
    workflowTypes: ["order_workflow", "payment_workflow"],
    registeredAt: timestamp(20),
    lastHeartbeatAt: timestamp(0.02),
    labels: { region: "us-east-1", instance: "c5.xlarge" },
    queueConcurrencyLimit: 10,
  },
  {
    workerId: uuid(),
    namespaceId: "default",
    taskQueue: "background",
    status: WorkerStatus.ONLINE,
    hostname: "worker-bg-1.internal",
    pid: 54321,
    version: "1.0.0",
    workflowTypes: ["report_generator", "data_pipeline", "cleanup_workflow"],
    registeredAt: timestamp(48),
    lastHeartbeatAt: timestamp(0.01),
    labels: { region: "us-west-2", instance: "m5.large" },
    queueConcurrencyLimit: 5,
  },
  {
    workerId: uuid(),
    namespaceId: "default",
    taskQueue: "orders",
    status: WorkerStatus.DRAINING,
    hostname: "worker-orders-old.internal",
    pid: 11111,
    version: "0.9.0",
    workflowTypes: ["order_workflow"],
    registeredAt: timestamp(168),
    lastHeartbeatAt: timestamp(0.5),
    labels: { region: "us-east-1", deprecated: "true" },
    queueConcurrencyLimit: 5,
  },
];
