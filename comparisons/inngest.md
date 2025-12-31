Here is a comprehensive documentation README for the Inngest TypeScript SDK. This covers installation, core concepts, advanced features, and API references with extensive code examples.

---

# Inngest TypeScript SDK

The official TypeScript SDK for **Inngest** — the developer platform for durable background jobs, workflows, and event-driven systems.

Build complex, long-running workflows with retries, sleep, wait-for-event, and step-by-step execution using standard TypeScript code.

[![npm version](https://badge.fury.io/js/inngest.svg)](https://badge.fury.io/js/inngest)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

## Table of Contents

- [Inngest TypeScript SDK](#inngest-typescript-sdk)
  - [Table of Contents](#table-of-contents)
  - [Introduction](#introduction)
  - [Installation](#installation)
  - [Quick Start](#quick-start)
  - [Core Concepts](#core-concepts)
    - [1. The Client](#1-the-client)
    - [2. Sending Events](#2-sending-events)
    - [3. Defining Functions](#3-defining-functions)
      - [Scheduled Jobs (CRON)](#scheduled-jobs-cron)
  - [The Step API](#the-step-api)
    - [`step.run`](#steprun)
    - [`step.sleep`](#stepsleep)
    - [`step.waitForEvent`](#stepwaitforevent)
    - [`step.invoke`](#stepinvoke)
  - [Advanced Configuration](#advanced-configuration)
    - [Concurrency \& Throttling](#concurrency--throttling)
    - [Batching](#batching)
    - [Cancellation](#cancellation)
    - [Debouncing](#debouncing)
  - [Handling Failures](#handling-failures)
    - [Automatic Retries](#automatic-retries)
    - [`onFailure` Handler](#onfailure-handler)
  - [TypeScript Strict Typing](#typescript-strict-typing)
  - [Serving Your API](#serving-your-api)
    - [Next.js (App Router)](#nextjs-app-router)
    - [Express](#express)
    - [Cloudflare Workers / Hono](#cloudflare-workers--hono)

---

## Introduction

Inngest replaces traditional queues, scheduled jobs, and workflow engines. It allows you to write functions that can run for hours or days, persisting state between steps automatically.

**Key Features:**

- **Serverless:** Works on Vercel, Cloudflare, AWS Lambda, or simple Express/Fastify servers.
- **Durable:** Functions resume exactly where they left off if your server restarts or crashes.
- **Event-Driven:** Trigger functions via events, webhooks, or CRON schedules.

---

## Installation

```bash
npm install inngest
# or
yarn add inngest
# or
pnpm add inngest
```

---

## Quick Start

A minimal example of a reliable background job.

```typescript
import { Inngest } from "inngest";

// 1. Initialize the client
const inngest = new Inngest({ id: "my-app" });

// 2. Define a function
const helloWorld = inngest.createFunction(
  { id: "hello-world" },
  { event: "test/hello.world" },
  async ({ event, step }) => {
    await step.sleep("wait-a-sec", "1s");
    return { message: `Hello ${event.data.name}!` };
  },
);

// 3. Send an event to trigger it
// await inngest.send({ name: "test/hello.world", data: { name: "User" } });
```

---

## Core Concepts

### 1. The Client

The `Inngest` client is your entry point. It is used to send events and create functions.

```typescript
import { EventSchemas, Inngest } from "inngest";

// Define your event types for full type safety
type Events = {
  "user/signup": {
    data: { userId: string; email: string };
  };
  "billing/invoice.created": {
    data: { invoiceId: string; amount: number };
  };
};

export const inngest = new Inngest({
  id: "my-e-commerce-app",
  schemas: new EventSchemas().fromRecord<Events>(),
});
```

### 2. Sending Events

Events trigger your functions. You can send them from anywhere in your application (API routes, server actions, scripts).

```typescript
// Send a single event
await inngest.send({
  name: "user/signup",
  data: { userId: "123", email: "alex@example.com" },
});

// Send a batch of events (atomic)
await inngest.send([
  { name: "user/signup", data: { ... } },
  { name: "billing/invoice.created", data: { ... } },
]);
```

### 3. Defining Functions

A Function consists of three parts:

1. **Configuration**: ID, retries, concurrency, etc.
2. **Trigger**: Event name or Cron schedule.
3. **Handler**: The logic to execute.

```typescript
const processSignup = inngest.createFunction(
  { id: "process-signup", name: "Process User Signup" }, // Config
  { event: "user/signup" }, // Trigger
  async ({ event, step }) => {
    // Handler
    // Your business logic here
  },
);
```

#### Scheduled Jobs (CRON)

You can trigger functions on a schedule.

```typescript
const weeklyReport = inngest.createFunction(
  { id: "weekly-report" },
  { cron: "0 9 * * MON" }, // Every Monday at 9am
  async ({ step }) => {
    // Generate report
  },
);
```

---

## The Step API

The `step` object passed to your handler is what makes Inngest powerful. It allows you to break your function into durable chunks.

### `step.run`

Encapsulates a block of code. If the function fails and retries, **completed steps are not re-run**; their result is returned immediately from the memoized state.

```typescript
const result = await step.run("charge-card", async () => {
  return await stripe.charges.create({ ... });
});
// If the function fails AFTER this line, 'charge-card' will not run again.
// 'result' will be hydrated from the previous successful run.
```

### `step.sleep`

Pauses execution for a specific duration. This does not block your server; the function effectively "shuts down" and wakes up later.

```typescript
// Wait for 3 days
await step.sleep("wait-3-days", "3d");

// Wait for specific date
await step.sleep("wait-until-christmas", new Date("2025-12-25"));
```

### `step.waitForEvent`

Pauses execution until a specific event is received. Useful for human-in-the-loop flows (e.g., waiting for a user to verify their email).

```typescript
const verification = await step.waitForEvent("wait-for-verify", {
  event: "user/email.verified",
  timeout: "24h", // Stop waiting after 24 hours
  match: "data.userId", // Ensure the event matches the current flow's user
});

if (!verification) {
  // Timeout occurred
  await step.run("send-reminder", async () => sendEmail(...));
}
```

### `step.invoke`

Trigger another Inngest function directly and wait for its result. This allows you to compose functions like microservices.

```typescript
// Call another function and wait for the return value
const generatedPdf = await step.invoke("generate-invoice-pdf", {
  function: generatePdfFunction,
  data: { invoiceId: event.data.id },
});
```

---

## Advanced Configuration

### Concurrency & Throttling

Control how many functions run simultaneously.

```typescript
inngest.createFunction(
  {
    id: "ai-processing",
    concurrency: [
      {
        limit: 5, // Max 5 functions running at once
        key: "event.data.userId", // Per user!
      }
    ],
    // OR Global Throttling (rate limit execution)
    throttle: {
      limit: 10,
      period: "1m", // Max 10 runs per minute
    }
  },
  { event: "ai/generate.image" },
  async ({ event }) => { ... }
);
```

### Batching

Group events together to process them efficiently (e.g., bulk database inserts).

```typescript
inngest.createFunction(
  {
    id: "track-analytics",
    batchEvents: {
      maxSize: 100, // Process when we have 100 events...
      timeout: "5s", // ...or every 5 seconds, whichever comes first
    },
  },
  { event: "analytics/track" },
  async ({ events, step }) => {
    // Note: 'events' is now an array
    await step.run("bulk-insert", async () => {
      await db.insert(events.map((e) => e.data));
    });
  },
);
```

### Cancellation

Cancel a running function when a specific event occurs.

```typescript
inngest.createFunction(
  {
    id: "drip-campaign",
    cancelOn: [
      {
        event: "user/unsubscribed", // Cancel if user unsubscribes
        match: "data.userId", // Match based on the userId field
      }
    ]
  },
  { event: "marketing/start.drip" },
  async ({ step }) => {
    await step.sleep("wait-1", "1d");
    await step.run("send-email-1", ...);
    await step.sleep("wait-2", "3d");
    await step.run("send-email-2", ...); // Won't run if cancelled
  }
);
```

### Debouncing

Delay execution to group rapid-fire events (only run the last one).

```typescript
inngest.createFunction(
  {
    id: "update-index",
    debounce: {
      period: "10s",
      key: "event.data.documentId",
    },
  },
  { event: "doc/updated" },
  async ({ event }) => {
    // Runs 10s after the LAST "doc/updated" event for this documentId
  },
);
```

---

## Handling Failures

Inngest handles retries automatically.

### Automatic Retries

By default, functions retry 4 times with exponential backoff.

```typescript
inngest.createFunction(
  {
    id: "unstable-job",
    retries: 10, // Increase max retries
  },
  { event: "job/start" },
  async () => { ... }
);
```

### `onFailure` Handler

Execute specific logic when a function exhausts all retries (e.g., send an alert to Slack).

```typescript
inngest.createFunction(
  {
    id: "critical-payment",
    onFailure: async ({ event, error, step }) => {
      // Send alert
      await slack.send(`Payment failed for ${event.data.userId}: ${error.message}`);
    }
  },
  { event: "payment/process" },
  async () => { ... }
);
```

---

## TypeScript Strict Typing

To ensure full type safety across your codebase, define a master Event Map.

```typescript
// types.ts
import { EventSchemas } from "inngest";

type AppEvents = {
  "shop/cart.updated": {
    data: { cartId: string; items: string[] };
    user: { id: string };
  };
  "auth/login": {
    data: { userId: string; method: "google" | "password" };
  };
};

// Create a typed client instance
export const inngest = new Inngest({
  id: "my-app",
  schemas: new EventSchemas().fromRecord<AppEvents>(),
});
```

Now, `inngest.send` will require correct data shapes, and `createFunction` handlers will infer `event.data` types automatically.

---

## Serving Your API

Inngest works by communicating with your application via an HTTP endpoint. You must serve your functions.

### Next.js (App Router)

```typescript
// app/api/inngest/route.ts
import { inngest } from "@/inngest/client";
import { fn1, fn2 } from "@/inngest/functions";
import { serve } from "inngest/next";

export const { GET, POST, PUT } = serve({
  client: inngest,
  functions: [fn1, fn2],
});
```

### Express

```typescript
import express from "express";
import { serve } from "inngest/express";
import { functions, inngest } from "./inngest";

const app = express();

app.use("/api/inngest", serve({ client: inngest, functions }));

app.listen(3000);
```

### Cloudflare Workers / Hono

```typescript
import { Hono } from "hono";
import { serve } from "inngest/hono";

const app = new Hono();

app.on(
  ["GET", "PUT", "POST"],
  "/api/inngest",
  serve({ client: inngest, functions }),
);

export default app;
```
