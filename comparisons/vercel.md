Here is a comprehensive documentation README for **Vercel Workflow** (powered by the Workflow Development Kit), written in a professional, developer-focused style.

---

# Vercel Workflow SDK

> **Note**: Vercel Workflow is currently in **Beta**.

The **Vercel Workflow SDK** (powered by the Workflow Development Kit) allows you to build durable, reliable, and observable workflows directly in your TypeScript/JavaScript applications. It extends standard async/await with durability, enabling functions to pause for days, survive deployments, and retry seamlessly upon failure without managing queues or infrastructure.

## 🚀 Features

- **Code-First Durability**: Just add `'use workflow'` and `'use step'` directives to your functions.
- **Zero-Config Infrastructure**: Vercel manages the queues, state persistence, and orchestration scaling automatically.
- **Long-Running Processes**: Workflows can sleep for seconds, hours, or months without consuming compute time.
- **Automatic Retries**: Steps automatically retry on failure with exponential backoff.
- **Observability**: Built-in visual dashboard for inspecting execution history, inputs, outputs, and errors.
- **Framework Agnostic**: Optimized for Next.js but compatible with other frameworks deploying to Vercel.

---

## 📦 Installation

Install the `workflow` package in your project:

```bash
npm install workflow
# or
pnpm add workflow
# or
yarn add workflow
```

---

## ⚡ Quick Start

### 1. Define your steps

Create functions that perform actual work (API calls, DB operations). Mark them with `'use step'`.

```typescript
// app/workflows/steps.ts
import { db } from "@/lib/db";

export async function getUser(userId: string) {
  "use step"; // <--- Marks this as a durable step
  return await db.user.findUnique({ where: { id: userId } });
}

export async function sendEmail(email: string, subject: string) {
  "use step";
  // If this fails (e.g., API down), it automatically retries
  await fetch("https://api.resend.com/emails", {
    method: "POST",
    body: JSON.stringify({ to: email, subject }),
    // headers...
  });
}
```

### 2. Define your workflow

Compose your steps into a workflow function. Mark it with `'use workflow'`.

```typescript
// app/workflows/onboarding.ts
import { sleep } from "workflow";
import { getUser, sendEmail } from "./steps";

export async function onboardingWorkflow(userId: string) {
  "use workflow"; // <--- Marks this as a durable workflow

  // 1. Fetch user data (results are memoized)
  const user = await getUser(userId);

  if (!user) throw new Error("User not found");

  // 2. Send welcome email
  await sendEmail(user.email, "Welcome to the platform!");

  // 3. Sleep for 3 days (consumes 0 compute resources)
  await sleep("3d");

  // 4. Send follow-up email
  await sendEmail(user.email, "Checking in...");

  return { status: "completed", userId };
}
```

### 3. Trigger the workflow

Call the workflow function from anywhere in your app, such as a Next.js Server Action or Route Handler.

```typescript
// app/api/signup/route.ts
import { onboardingWorkflow } from "@/app/workflows/onboarding";
import { NextResponse } from "next/server";

export async function POST(request: Request) {
  const { userId } = await request.json();

  // This starts the workflow asynchronously.
  // The function returns immediately with a handle (or void depending on config),
  // while the workflow executes in the background.
  await onboardingWorkflow(userId);

  return NextResponse.json({ success: true, message: "Onboarding started" });
}
```

---

## 📖 Core Concepts

### Directives

Vercel Workflow relies on two simple string directives to transform your code into a durable execution graph.

#### `'use workflow'`

- **Role**: Orchestrator.
- **Behavior**:
  - Execution is **deterministic**.
  - Logic is replayed from the beginning on resume/crash, skipping already completed steps (memoization).
  - **Do not** perform side effects (DB writes, API calls) directly here; use steps instead.
  - Can contain control flow logic (if/else, loops, switch).

#### `'use step'`

- **Role**: Worker unit.
- **Behavior**:
  - Performs side effects (API calls, DB mutations).
  - Results are persisted to the event log.
  - Automatically retries on failure (default: exponential backoff).
  - Must be idempotent where possible.

### Sleeping & Delays

You can pause a workflow for any duration using the `sleep` utility. During this time, the workflow is "suspended" and costs nothing.

```typescript
import { sleep } from "workflow";

export async function dripCampaign() {
  "use workflow";

  await sendEmail("Day 1");
  await sleep("24h"); // Waits 24 hours

  await sendEmail("Day 2");
  await sleep("1 week"); // Waits 7 days

  await sendEmail("Day 9");
}
```

**Supported formats**: `'10s'`, `'5m'`, `'24h'`, `'3d'`, `'1 month'`, or milliseconds (number).

---

## 🛠 API Usage & Patterns

### Error Handling & Retries

Steps retry automatically. You can wrap steps in `try/catch` within the workflow to handle permanent failures or run compensating logic.

```typescript
// app/workflows/payment.ts
export async function processSubscription(userId: string) {
  "use workflow";

  try {
    await chargeCard(userId); // 'use step' function
  } catch (error) {
    // If retries are exhausted, this block runs
    await sendPaymentFailedEmail(userId);
    await downgradeAccount(userId);
    throw error; // Fail the workflow
  }
}
```

### Waiting for External Events (Human-in-the-Loop)

Workflows can pause until they receive an external signal (e.g., a webhook or user click).

```typescript
import { defineHook } from "workflow";

// Define a hook to wait for
const approvalHook = defineHook("approval");

export async function expenseWorkflow(expenseId: string) {
  "use workflow";

  await sendManagerApprovalRequest(expenseId);

  // Pause execution until the 'approval' event is received for this instance
  const event = await approvalHook.waitForEvent({
    timeout: "7d",
  });

  if (event.approved) {
    await payExpense(expenseId);
  } else {
    await rejectExpense(expenseId);
  }
}
```

### Control Flow (Loops)

You can use standard JavaScript loops. The SDK ensures the loop state is preserved.

```typescript
export async function pollExternalSystem(jobId: string) {
  "use workflow";

  let status = "pending";

  while (status !== "completed") {
    status = await checkJobStatus(jobId); // 'use step'

    if (status === "completed") break;

    await sleep("1m"); // Wait before polling again
  }

  await notifyUser(jobId);
}
```

---

## 📊 Observability

Deploying to Vercel automatically enables the **Workflows Dashboard**.

1. Navigate to your Vercel Project.
2. Click on the **Logs** or **Workflows** tab (depending on beta UI).
3. Inspect:
   - **Run History**: See all active, completed, and failed runs.
   - **Visual Trace**: A timeline view of every step execution, sleep, and error.
   - **State Inspection**: View the input and output JSON of every step.

---

## ⚙️ Configuration

You can configure workflow behavior via `workflow.config.ts` (if supported in your version) or step-level options.

### Customizing Retries (Step Level)

_Note: Currently, retry behavior is managed by the platform defaults, but explicit configuration APIs are coming soon._

For now, standard behavior is:

- **Transient Errors**: Retried automatically (Network errors, timeouts).
- **Permanent Errors**: Fail immediately if thrown (e.g., validation errors).

---

## 🧩 FAQ

**Q: Can I use `Math.random()` or `Date.now()` in a workflow?**
A: **No**, not directly inside `'use workflow'`. Because the workflow function replays to rebuild state, it must be deterministic.

- **Bad**: `const id = Math.random();` (changes on every replay)
- **Good**: Pass it as input or generate it inside a `'use step'` function.

**Q: Where is the state stored?**
A: Vercel manages a secure, hidden storage layer for your project that persists the event history and variables of your workflow functions.

**Q: What happens if I redeploy while a workflow is sleeping?**
A: The workflow is durable. When the sleep timer expires, the workflow resumes using the _new_ version of your code, while retaining the state of steps that already completed.

---

## 🔗 Resources

- [Official Vercel Documentation](https://vercel.com/docs)
- [WDK GitHub Repository](https://github.com/vercel/workflow)
- [Examples & Templates](https://vercel.com/templates)
