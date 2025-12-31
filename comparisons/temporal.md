# Temporal TypeScript SDK – Comprehensive README

> **Durable execution for modern distributed systems**
> This document is a **complete, production-grade README** for using **Temporal with the TypeScript SDK**, covering concepts, features, APIs, and extensive code examples.

---

## Table of Contents

1. What is Temporal?
2. Core Concepts
3. Architecture Overview
4. Installation & Setup
5. Project Structure
6. Workflows
7. Activities
8. Workflow APIs
9. Activity APIs
10. Workers
11. Task Queues
12. Signals
13. Queries
14. Timers & Sleep
15. Child Workflows
16. Continue-As-New
17. Workflow Versioning
18. Error Handling & Retries
19. Heartbeating
20. Timeouts
21. Determinism Rules
22. Workflow Testing
23. Local Development
24. Deployment Patterns
25. Observability (Logs, Metrics)
26. Security
27. Performance & Scaling
28. Common Pitfalls
29. Reference Links

---

## 1. What is Temporal?

Temporal is a **durable execution platform** that allows you to write **fault-tolerant, long-running business logic** as ordinary code.

Instead of building complex retry logic, state machines, cron jobs, and recovery flows manually, Temporal provides:

- Automatic retries
- Durable state persistence
- Exactly-once workflow execution
- Transparent failure recovery
- Long-running execution (days, months, years)

Temporal guarantees that your workflows **resume exactly where they left off**, even after crashes, redeploys, or network failures.

---

## 2. Core Concepts

| Concept         | Description                                         |
| --------------- | --------------------------------------------------- |
| Workflow        | Deterministic orchestration code                    |
| Activity        | Side-effecting, non-deterministic code              |
| Worker          | Polls Task Queues and executes Workflows/Activities |
| Task Queue      | Logical queue for routing tasks                     |
| Event History   | Immutable log of workflow execution                 |
| Temporal Server | Durable state machine engine                        |

---

## 3. Architecture Overview

```
Client → Temporal Server → Task Queue → Worker
                 ↑             ↓
              Event History ← Execution
```

- Clients start workflows
- Server persists all state
- Workers execute code
- History is replayed deterministically

---

## 4. Installation & Setup

### Prerequisites

- Node.js 18+
- Temporal Server (local or cloud)

### Install SDK

```bash
npm install @temporalio/client @temporalio/worker @temporalio/workflow
```

### Local Temporal Server

```bash
temporal server start-dev
```

---

## 5. Project Structure

```
/temporal
  /workflows
    orderWorkflow.ts
  /activities
    orderActivities.ts
  worker.ts
  client.ts
```

---

## 6. Workflows

Workflows define **business logic orchestration**.

### Basic Workflow

```ts
import { proxyActivities } from "@temporalio/workflow";
import type * as activities from "../activities/orderActivities";

const { chargeCustomer, shipOrder } = proxyActivities<typeof activities>({
  startToCloseTimeout: "5 minutes",
});

export async function orderWorkflow(orderId: string): Promise<void> {
  await chargeCustomer(orderId);
  await shipOrder(orderId);
}
```

### Key Rules

- Must be **deterministic**
- No Date.now(), Math.random(), fetch(), or I/O
- All side effects must be Activities

---

## 7. Activities

Activities perform **real-world side effects**.

```ts
export async function chargeCustomer(orderId: string) {
  // call payment provider
}

export async function shipOrder(orderId: string) {
  // call shipping API
}
```

Activities:

- Can fail
- Can retry automatically
- Can timeout
- Can heartbeat

---

## 8. Workflow APIs

### Sleep / Timers

```ts
import { sleep } from "@temporalio/workflow";

await sleep("1 hour");
```

### UUID

```ts
import { uuid4 } from "@temporalio/workflow";

const id = uuid4();
```

---

## 9. Activity APIs

### Retry Policy

```ts
const activities = proxyActivities<typeof activities>({
  retry: {
    maximumAttempts: 5,
    initialInterval: "1s",
    backoffCoefficient: 2,
  },
});
```

---

## 10. Workers

Workers execute workflows and activities.

```ts
import { Worker } from "@temporalio/worker";

async function run() {
  const worker = await Worker.create({
    workflowsPath: require.resolve("./workflows"),
    activities: require("./activities"),
    taskQueue: "order-queue",
  });

  await worker.run();
}

run();
```

---

## 11. Task Queues

Task Queues route work to specific workers.

```ts
taskQueue: "payments";
```

Use multiple queues to:

- Scale horizontally
- Isolate workloads
- Route by region or service

---

## 12. Signals

Signals allow **external mutation** of workflow state.

```ts
import { defineSignal } from "@temporalio/workflow";

export const cancelSignal = defineSignal("cancel");
```

```ts
setHandler(cancelSignal, () => {
  cancelled = true;
});
```

### Sending a Signal

```ts
await client.workflow.signal(workflowId, "cancel");
```

---

## 13. Queries

Queries read workflow state **without mutation**.

```ts
export const statusQuery = defineQuery<string>("status");
```

```ts
setHandler(statusQuery, () => currentStatus);
```

---

## 14. Timers & Delays

```ts
await sleep("30 days");
```

Temporal timers survive:

- Process crashes
- Restarts
- Deployments

---

## 15. Child Workflows

```ts
import { executeChild } from "@temporalio/workflow";

await executeChild(childWorkflow, {
  args: ["child-id"],
  taskQueue: "child-queue",
});
```

---

## 16. Continue-As-New

Used to avoid large histories.

```ts
import { continueAsNew } from "@temporalio/workflow";

return continueAsNew(nextState);
```

---

## 17. Workflow Versioning

Supports safe code evolution.

```ts
import { patched } from "@temporalio/workflow";

if (patched("new-logic")) {
  // new behavior
}
```

---

## 18. Error Handling & Retries

### Application Errors

```ts
import { ApplicationFailure } from "@temporalio/workflow";

throw ApplicationFailure.nonRetryable("Invalid order");
```

---

## 19. Heartbeating

```ts
import { Context } from "@temporalio/activity";

Context.current().heartbeat(progress);
```

---

## 20. Timeouts

| Timeout         | Purpose         |
| --------------- | --------------- |
| ScheduleToStart | Queue wait time |
| StartToClose    | Execution time  |
| Heartbeat       | Liveness        |

---

## 21. Determinism Rules

Allowed:

- Pure logic
- Conditionals
- Loops

Forbidden:

- Network calls
- Randomness
- System time

---

## 22. Workflow Testing

```ts
import { TestWorkflowEnvironment } from "@temporalio/testing";
```

Supports:

- Time skipping
- Mocked activities
- Deterministic replay

---

## 23. Local Development

```bash
temporal server start-dev
```

Web UI: [http://localhost:8233](http://localhost:8233)

---

## 24. Deployment Patterns

- One worker per service
- Multiple task queues
- Blue/green workflow versions

---

## 25. Observability

Temporal provides:

- Execution history
- Built-in metrics

---

## 26. Security

- mTLS
- Namespaces
- Role-based access

---

## 27. Performance & Scaling

- Horizontal worker scaling
- Sticky execution
- Sharded task queues

---

## 28. Common Pitfalls

- Non-deterministic code
- Large histories
- Long-running activities

---

## 29. Reference Links

- Temporal Concepts
- TypeScript SDK Docs
- Temporal Cloud

---

## Final Notes

Temporal allows you to write **business logic once**, without worrying about retries, crashes, or distributed systems complexity.

If your system needs **correctness over time**, Temporal is infrastructure—not a library.
