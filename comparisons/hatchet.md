This is a comprehensive documentation README for the Hatchet TypeScript SDK. It covers installation, core concepts, and advanced features with copy-pasteable code examples.

---

# Hatchet TypeScript SDK

The official TypeScript SDK for [Hatchet](https://hatchet.run), a distributed, fault-tolerant task queue and workflow engine. Replace legacy queues and pub/sub with durable, code-first workflows.

## Table of Contents

- [Hatchet TypeScript SDK](#hatchet-typescript-sdk)
  - [Table of Contents](#table-of-contents)
  - [Features](#features)
  - [Installation](#installation)
  - [Prerequisites](#prerequisites)
  - [Quick Start](#quick-start)
    - [1. Define the Workflow (`workflow.ts`)](#1-define-the-workflow-workflowts)
    - [2. Trigger the Workflow (`trigger.ts`)](#2-trigger-the-workflow-triggerts)
  - [Core Concepts](#core-concepts)
    - [Workflows \& Steps](#workflows--steps)
    - [Workers](#workers)
    - [Triggering Runs](#triggering-runs)
  - [Advanced Usage](#advanced-usage)
    - [Concurrency \& Rate Limiting](#concurrency--rate-limiting)
    - [Retries \& Error Handling](#retries--error-handling)
    - [Cron Schedules](#cron-schedules)
    - [Child Workflows (Fan-out)](#child-workflows-fan-out)
    - [Timeouts](#timeouts)
    - [Streaming \& Logging](#streaming--logging)
  - [API Reference](#api-reference)
    - [`Hatchet.init(config?)`](#hatchetinitconfig)
    - [`worker.registerWorkflow(workflow)`](#workerregisterworkflowworkflow)
    - [`ctx` (Context Object)](#ctx-context-object)
    - [`hatchet.event.push(eventName, payload)`](#hatcheteventpusheventname-payload)
    - [`hatchet.admin.run_workflow(workflowId, input)`](#hatchetadminrun_workflowworkflowid-input)

---

## Features

- **Durable Execution**: Steps are durably logged. If a worker crashes, Hatchet resumes execution exactly where it left off.
- **Low Latency**: Sub-25ms task dispatch for high-throughput workloads.
- **Smart Retries**: automatic retries with exponential backoff for transient failures.
- **Concurrency Control**: Global and per-key rate limiting (e.g., "max 5 concurrent runs per user").
- **Cron Scheduling**: Built-in distributed scheduler for recurring tasks.
- **Type Safety**: Full TypeScript support with type inference for inputs and outputs.
- **DAG Support**: specific dependencies between steps (Directed Acyclic Graphs).

---

## Installation

```bash
npm install @hatchet-dev/typescript-sdk
# or
pnpm add @hatchet-dev/typescript-sdk
# or
yarn add @hatchet-dev/typescript-sdk
```

## Prerequisites

1. **Hatchet Engine**: You need a running Hatchet instance (Hatchet Cloud or Self-Hosted).
2. **API Token**: Generate a token from your Hatchet dashboard (`Settings` -> `API Keys`).
3. **Environment Variables**: Create a `.env` file in your project root:

```env
HATCHET_CLIENT_TOKEN="<your-api-key>"
# If self-hosting, add your instance URL (defaults to cloud)
# HATCHET_CLIENT_TLS_STRATEGY=none (if not using SSL locally)
```

---

## Quick Start

This example creates a simple workflow with one step that runs when manually triggered.

### 1. Define the Workflow (`workflow.ts`)

```typescript
import Hatchet from "@hatchet-dev/typescript-sdk";

const hatchet = Hatchet.init();

const workflow = {
  id: "simple-workflow",
  description: "My first Hatchet workflow",
  on: {
    event: "user:create", // Trigger on this event name
  },
  steps: [
    {
      name: "send-welcome-email",
      run: async (ctx) => {
        const input = ctx.workflowInput();
        console.log(`Sending welcome email to ${input.email}`);
        return { sent: true };
      },
    },
  ],
};

async function main() {
  const worker = hatchet.worker("example-worker");
  await worker.registerWorkflow(workflow);
  worker.start();
}

main();
```

### 2. Trigger the Workflow (`trigger.ts`)

```typescript
import Hatchet from "@hatchet-dev/typescript-sdk";

const hatchet = Hatchet.init();

async function run() {
  // Push an event to trigger the workflow
  await hatchet.admin.run_workflow("simple-workflow", {
    email: "test@example.com",
  });

  console.log("Workflow triggered!");
}

run();
```

---

## Core Concepts

### Workflows & Steps

A **Workflow** is a collection of steps. It is defined by a unique `id` and a trigger (`on`).
**Steps** are individual units of logic. They can return data that is passed to subsequent steps.

```typescript
const detailedWorkflow = {
  id: "data-pipeline",
  on: { event: "data:process" },
  steps: [
    {
      name: "fetch-data",
      run: async (ctx) => {
        return { rawData: [1, 2, 3] };
      },
    },
    {
      name: "process-data",
      parents: ["fetch-data"], // This step waits for 'fetch-data'
      run: async (ctx) => {
        // Access output from parent step
        const { rawData } = ctx.stepOutput("fetch-data");
        return { processed: rawData.map((x) => x * 2) };
      },
    },
  ],
};
```

### Workers

A **Worker** is a long-running process that listens for tasks assigned to it. You can run workers on any infrastructure (AWS, Vercel, Railway, localhost).

```typescript
const worker = hatchet.worker("data-worker", {
  maxRuns: 10, // Max concurrent workflow runs on this worker
});

await worker.registerWorkflow(detailedWorkflow);
worker.start(); // Blocking call
```

### Triggering Runs

You can trigger workflows via **Events** or **Direct Execution**.

**1. Event-based (Decoupled):**
Triggers any workflow listening for `user:signup`.

```typescript
await hatchet.event.push("user:signup", { userId: "123" });
```

**2. Direct Run (RPC-style):**
Triggers a specific workflow ID.

```typescript
const runId = await hatchet.admin.run_workflow("data-pipeline", {
  date: "2023-10-01",
});
```

---

## Advanced Usage

### Concurrency & Rate Limiting

Limit how many workflow instances can run simultaneously to protect your resources or external APIs.

**Global Limit:**
Only 1 instance of this workflow runs at a time globally.

```typescript
const workflow = {
  id: "db-migration",
  concurrency: {
    limit: 1,
  },
  // ...
};
```

**Key-based Limit (Fairness):**
Allow max 5 concurrent runs _per user_.

```typescript
const workflow = {
  id: "ai-generation",
  concurrency: {
    name: "per-user-limit",
    maxRuns: 5,
    limitStrategy: "CANCEL_IN_PROGRESS", // or GROUP_ROUND_ROBIN
    key: (ctx) => ctx.workflowInput().userId, // Dynamic key from input
  },
  // ...
};
```

### Retries & Error Handling

Automatically retry steps on failure.

```typescript
steps: [
  {
    name: "unreliable-api-call",
    retries: 3, // Retry up to 3 times
    timeout: "60s", // Timeout after 60 seconds
    run: async (ctx) => {
      // If this throws, Hatchet catches it and schedules a retry
      await fetch("https://flaky-api.com");
    },
  },
];
```

To stop retrying immediately (non-retriable error), throw a specific error type (implementation depends on your error logic, usually plain errors trigger retries by default until exhausted).

### Cron Schedules

Schedule workflows to run automatically at specific intervals using standard cron syntax.

```typescript
const cleanupWorkflow = {
  id: "nightly-cleanup",
  on: {
    cron: "0 0 * * *", // Runs every day at midnight
  },
  steps: [
    {
      name: "delete-old-logs",
      run: async (ctx) => console.log("Cleaning up..."),
    },
  ],
};
```

### Child Workflows (Fan-out)

A step can spawn other workflows and wait for their results. This is useful for "fan-out/fan-in" patterns.

```typescript
{
  name: 'spawn-child-jobs',
  run: async (ctx) => {
    const items = [1, 2, 3, 4, 5];

    // Spawn 5 workflows in parallel
    const runPromises = items.map(item =>
      ctx.spawnWorkflow('process-item-workflow', { itemId: item })
    );

    // Wait for all to complete and get results
    const results = await Promise.all(runPromises.map(r => r.result()));

    return { results };
  },
}
```

### Timeouts

Ensure a step doesn't hang forever.

```typescript
{
  name: 'long-computation',
  timeout: '5m', // 5 minutes
  run: async (ctx) => {
    // ...
  }
}
```

### Streaming & Logging

Hatchet supports streaming logs from your worker to the dashboard in real-time.

```typescript
{
  name: 'logger-step',
  run: async (ctx) => {
    ctx.log('Starting the process...'); // Visible in Hatchet Dashboard

    // Stream data back to the caller (if needed)
    ctx.putStream('Keep alive signal...');
  }
}
```

---

## API Reference

### `Hatchet.init(config?)`

Initializes the Hatchet client.

- `config`: Optional object to override env vars (token, host, etc).

### `worker.registerWorkflow(workflow)`

Registers a workflow definition.

- `workflow`: Object containing `id`, `on`, `steps`, etc.

### `ctx` (Context Object)

Passed to every step function.

- `ctx.workflowInput()`: Get input passed to the workflow.
- `ctx.stepOutput(stepName)`: Get result from a parent step.
- `ctx.trigger(eventName, data)`: Trigger a new event.
- `ctx.sleep(ms)`: Durable sleep (survives worker restarts).
- `ctx.playground(key, defaultValue)`: Define inputs editable in the UI for testing/retries.

### `hatchet.event.push(eventName, payload)`

Ingests an event into Hatchet.

### `hatchet.admin.run_workflow(workflowId, input)`

Directly executes a workflow.

---

For more details, visit the [Hatchet Documentation](https://docs.hatchet.run).
