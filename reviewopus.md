[4 tools called]

Let me also look at the current Kagzi implementation to understand the existing patterns before providing a comprehensive review.

[5 tools called]

Now I have a complete picture. Let me provide a detailed review and ask clarifying questions.

---

## Plan Review: Kagzi API Redesign

### Overall Assessment

The plan is well-structured and addresses real pain points. The proposed changes align well with industry standards (Temporal, Inngest, Hatchet). However, there are several areas that need clarification before finalizing.

---

## Detailed Analysis

### 1. Worker API Simplification ✅ Mostly Good

**Proposed:**

```rust
Worker::new("http://localhost:50051")
    .namespace("tenant-123")
    .queue("orders")
    .max_concurrent(50)
    .retries(3)
    .workflows([...])
    .build()
```

**Observations:**

- Moving `task_queue` from constructor to builder method `.queue()` is cleaner
- Bundling workflows in builder vs. separate `register()` calls is a style choice

**Concern:** The `.workflows([...])` pattern requires all workflows be known at builder time. Current pattern allows dynamic registration:

```rust
// Current - allows conditional registration
worker.register("order-v1", order_v1);
if feature_flag {
    worker.register("order-v2", order_v2);
}
```

-- my review
hmm tbf the user should always define all workflows in restart, and I want it to be in the builder area so it stays connected tbf.

---

### 2. Retry API ✅ Good

**Current:**

```rust
RetryPolicy {
    maximum_attempts: Some(3),
    initial_interval: Some(Duration::from_secs(1)),
    backoff_coefficient: Some(2.0),
    maximum_interval: Some(Duration::from_secs(60)),
    non_retryable_errors: vec!["InvalidArgument".to_string()],
}
```

**Proposed:**

```rust
.retries(3)
// or
.retry(Retry::exponential(3).initial("1s").max("60s"))
```

This is a significant DX improvement. Matches Inngest/Hatchet patterns. The presets (`Retry::none()`, `Retry::forever()`, `Retry::linear(5, "2s")`) are excellent.

**Question:** For human-readable duration parsing (`"1s"`, `"5m"`, `"1d"`), are you planning to:

- (A) Use an existing crate like `humantime` or `duration-str`?
- (B) Roll your own parser?

I'd recommend (A) for standardization and fewer edge cases.

-- my review
Use the existing humantime crate for this. It's a standard library and is well-maintained.

---

### 3. Step Execution (Single Builder) ✅ Excellent

**Proposed:**

```rust
ctx.step("validate-order")
    .retry(Retry::exponential(10))
    .timeout("5m")
    .run(|| validate(&input))
    .await?;
```

This is superior to the current triple method variants (`run`, `run_with_input`, `run_with_input_with_retry`). The builder pattern is extensible without API breakage.

**Question:** How should input be passed? The plan shows:

```rust
.run(|| validate(&input))  // captures `input` from outer scope
```

But current API explicitly passes input:

```rust
ctx.run_with_input("step", &input, async { ... })
```

The explicit input matters because Kagzi serializes it for replay/memoization. If using closure capture, how do you ensure the input is serialized to the step log?

Possible solutions:

- (A) `.input(&data).run(|| ...)` - explicit on builder
- (B) `.run(|input| ...)` - passed to closure (like Hatchet)
- (C) Capture analysis (complex, error-prone)

-- my review
Im not sure, what do you suggest we should do.

---

### 4. Sleep API ✅ Good (Minor Suggestion)

**Proposed:**

```rust
ctx.sleep("wait-cooldown", "30s").await?;
```

Named sleeps are excellent for observability (matches Inngest exactly).

**Question:** The current implementation uses an auto-incrementing counter for sleep IDs:

```rust
pub(crate) sleep_counter: u32,
```

With named sleeps, what happens if the user:

1. Accidentally reuses a name?
2. Forgets to provide a unique name in a loop?

```rust
for item in items {
    ctx.sleep("wait", "1s").await?;  // Same name each iteration - problem?
}
```

Options:

- (A) Auto-append index: `"wait_0"`, `"wait_1"`, etc.
- (B) Error on duplicate name
- (C) Allow same name (rely on user discipline)

Inngest uses (A) implicitly with `step.sleep("name", ...)` allowing duplicates in loops.

-- my review
Name is name, I dont think theres any need for it to be unique since its more for tracking purposes.

---

### 5. Step State Tracking Middleware ⚠️ Needs Discussion

The middleware pattern for tracking step states is architecturally clean, but I'm unclear on the actual problem being solved.

**Current Step Flow:**

```
BeginStep → (sets RUNNING) → execute → CompleteStep/FailStep → (sets COMPLETED/FAILED)
```

The state transitions are already atomic SQL operations in `crates/kagzi-store/src/postgres/step.rs`.

**Question:** Is the concern:

- (A) Too much code duplication across service methods?
- (B) Want to add cross-cutting concerns (metrics, tracing) without touching each method?
- (C) Want to decouple business logic from persistence?

If (B), this is a good abstraction. If (A), it might be over-engineering since the transitions are inherently tied to the DB operations.

-- my review
I'm also not sure, should we change, should we rewrite it, should we just optimize it if needed etc, What do you suggest?

---

### 6. Client API ✅ Good

**Proposed:**

```rust
client.start("process-order")
    .namespace("tenant-123")
    .queue("orders")
    .input(OrderInput { ... })
    .retries(5)
    .id("order-123")
    .await?;
```

Clean and consistent with the worker API. The `.id()` for idempotency is smart.

---

## Key Questions on Queue Control

This is the central question you raised. Let me break it down:

### Current Model

| Concept                | Purpose          | User Controls                                                            |
| ---------------------- | ---------------- | ------------------------------------------------------------------------ |
| **Namespace**          | Tenant isolation | Set per worker/workflow                                                  |
| **Queue (task_queue)** | Work routing     | Set per worker/workflow                                                  |
| **Concurrency**        | Resource limits  | `max_concurrent`, `queue_concurrency_limit`, `workflow_type_concurrency` |

### What Other Products Expose

| Product      | Queue Exposure                   | Concurrency Control                                       |
| ------------ | -------------------------------- | --------------------------------------------------------- |
| **Temporal** | Task Queue (mandatory, explicit) | None at SDK level, infra-managed                          |
| **Inngest**  | Hidden (event routing)           | Per-function `concurrency: { limit, key }` + `throttle`   |
| **Hatchet**  | Hidden (event routing)           | Per-workflow `concurrency: { limit, key, limitStrategy }` |
| **Vercel**   | Hidden (no queue concept)        | None                                                      |

### Clarifying Questions

**Q1: What is `queue_concurrency_limit` intended to do?**

Looking at your current code, I see three concurrency knobs:

1. `max_concurrent` - worker-local limit
2. `queue_concurrency_limit` - stored in DB per worker registration
3. `workflow_type_concurrency` - per-workflow-type limit

But in `PgWorkflowRepository::poll_workflow`, I don't see these limits being enforced at poll time. Is `queue_concurrency_limit` intended for:

- (A) Server-side enforcement (not yet implemented)?
- (B) Client-side advisory information?
- (C) Future use with distributed rate limiting?

-- my review
Hmm I was not intending to have server side limits, but keep it on the worker themselves, how does other products do it? How should we handle it? If we implement it server side, will it be easy or too complex? Since from what I see, all the 3 temporal, hatchet and inngest do it on the worker side. Should we keep the queue hidden? and do it the event way like inngest? and the internal queue can create various queues according to event?

**Q2: Do you want to support per-key concurrency?**

Inngest and Hatchet allow:

```typescript
concurrency: {
    limit: 5,
    key: "event.data.userId"  // Max 5 per user
}
```

This is powerful for fairness (e.g., "max 5 concurrent AI generations per tenant"). Is this in scope for Kagzi, or should `namespace` + `queue` be sufficient for isolation?

-- my review
For concurrency, I was thinking we should do it at the worker level, like how many parallel workflows it can run etc, lets use some stragtegy or some library for algorithm not sure if possible.

And then at the workflow level, but it will be depndennt inside the worker probably, not server side. hmm how does others handle it?

**Q3: Throttling vs. Concurrency - do you need both?**

- **Concurrency**: How many can run simultaneously
- **Throttling**: How many can start per time window

Inngest has both:

```typescript
concurrency: { limit: 5 },           // Max 5 running at once
throttle: { limit: 10, period: "1m" } // Max 10 new runs per minute
```

Is throttling in scope?

-- my review
Not sure, how does others handle it? How should we do it.

**Q4: Should queues be explicit or implicit?**

Current (Temporal-style):

```rust
Worker::new(addr, "orders")  // queue is required
client.workflow("process", "orders", input)  // queue is required
```

Inngest-style (no queues, just events):

```typescript
inngest.createFunction({ id: "process-order" }, { event: "order.created" }, ...)
inngest.send({ name: "order.created", data })
```

The Inngest model is simpler for users but reduces routing control. Do you want to:

- (A) Keep queues explicit (current, Temporal-like)?
- (B) Make queues optional with a default?
- (C) Hide queues entirely (event-based routing)?

-- my review
I think we should keep it hidden, can make it on event based or something?

**Q5: What's the relationship between namespace and queue for polling?**

Current poll logic:

```sql
WHERE namespace_id = $1 AND task_queue = $2
```

Both are required. Is this the intended model, or should workers be able to:

- Poll all queues in a namespace?
- Poll same queue across namespaces (unlikely, breaks isolation)?

-- my review
Namespace is always isolated.

---

## Summary of Open Questions

1. **Workflow registration**: Builder-time vs. dynamic `register()` - which pattern?
2. **Duration parsing**: Use `humantime` crate or custom?
3. **Step input**: Explicit `.input()` or closure capture?
4. **Duplicate sleep names**: Auto-index, error, or allow?
5. **Middleware motivation**: What specific problem does it solve?
6. **Queue concurrency limit**: Server-side enforcement or advisory?
7. **Per-key concurrency**: In scope (like Inngest/Hatchet)?
8. **Throttling**: In scope?
9. **Queue explicitness**: Keep required, default, or hide?
10. **Namespace-queue relationship**: Strict pairing or flexible?

Please clarify these and I'll help refine the implementation plan accordingly.

-- my review
Ultimately my aim is to keep things simple, and not become another temporal with too much complexity.
