# Kagzi Queue v2 Plan (Work-Signal Bus + DB Claim)

## Summary

We want Kagzi workers to “connect to a queue” (Kafka / NATS JetStream / RabbitMQ / Postgres / later SQLite) and have the queue be responsible for _distribution_ of wakeups, while Kagzi remains correct and durable.

The core rule that keeps this correct across crashes, redeliveries, and outages:

> **A message delivery is never a claim. Postgres is the source of truth and the DB lease/claim is the only gateway to execution.**

So the external queue becomes a **work-signal bus** (notification + coarse distribution), and Postgres remains the **authoritative queue state** (runnable state + locking + retries + timers + orphan recovery).

---

## Goals

- Workers can subscribe to a queue backend and get notified when a `{namespace, task_queue}` may have runnable work.
- Backends are pluggable: default Postgres-based signaling for easy startup; optional Kafka/NATS/Rabbit for production scale; later SQLite for local/dev.
- Maintain Kagzi correctness:
  - at-least-once execution with idempotent replay
  - durable retries / backoff
  - durable sleep/timers (`available_at`)
  - orphan recovery via visibility timeout lease
- Reduce DB polling load and reduce gRPC long-polling complexity.

## Non-goals (for this phase)

- “Exactly once” execution guarantees (not realistic with arbitrary long tasks).
- Making Kafka/NATS/Rabbit the authoritative workflow store (would duplicate orchestration logic across backends).
- Sophisticated placement / smart worker selection (capabilities/labels-based routing) beyond task-queue partitioning.

---

## Reality-based problems we must solve (and how we solve them)

### 1) Lost wakeups can stall work

**Problem:** Publishing `WorkAvailable` might fail (broker outage, server crash). If workers depend solely on messages, due work can get stuck.

**Fix:** Always have a durable fallback:

- Server-side periodic **due-work enqueuer**: query Postgres for _distinct_ `{namespace, task_queue}` that currently have due work and publish `WorkAvailable`.
- Worker-side periodic **try-claim tick** (belt-and-suspenders): even if no signals arrive, occasionally call `ClaimTask`.

Either one alone can work; having both is ideal.

### 2) Broker timeouts / redelivery storms if ack is tied to execution

**Problem:** If workers only ack/commit after finishing workflows, brokers will redeliver (JetStream `ack_wait`, Rabbit consumer timeout, Kafka rebalances).

**Fix:** Ack/commit the broker message immediately after attempting claim/drain-claim. Execution is governed by Postgres lease/heartbeat, not broker visibility.

### 3) Thundering herd and DB contention on hot queues

**Problem:** A hot queue can trigger bursts of claim queries.

**Fix:**

- Use competing-consumer/consumer-group semantics for external backends (one signal is delivered to one worker in a group).
- Worker uses bounded **drain-claim** (cap N claims per signal).
- Add jitter/backoff on repeated empty claims.
- Ensure DB indexing matches the claim query.

### 4) Routing mismatches (workers without capability get signals)

**Problem:** If workers subscribe too broadly, they waste cycles receiving signals and failing claims.

**Fix:**

- Keep “who should see signals” defined by `{namespace, task_queue}` first.
- Keep `workflow_types` filtering enforced in the DB claim (as it is today).
- Later: optionally partition by additional routing keys (capability queues) if needed.

### 5) Double execution risk if DB-claim is bypassed

**Problem:** Any path that executes without the atomic DB claim will double-execute during crash/rebalance scenarios.

**Fix:** Make DB claim the only gateway to execution. All modes (Postgres bus, Kafka/NATS/Rabbit, SQLite) must go through the same `ClaimTask`.

---

## Target architecture

### Authoritative plane (Postgres)

- Keeps workflow state and timing:
  - `status`, `available_at`, `locked_by`, `attempts`, etc.
- Owns lock/lease semantics:
  - claim via atomic query (e.g. `FOR UPDATE SKIP LOCKED` + update lease)
  - extend lease on heartbeat
  - orphan recovery when `available_at` passes while status `RUNNING`

### Distribution plane (Work-signal bus)

- Emits a lossy signal:
  - `WorkAvailable { namespace, task_queue }`
- Semantics:
  - “There may be runnable work in this queue; go try claim.”
  - Duplicate signals are normal and safe.

---

## Deployment reality: multiple `kagzi-server` replicas + broker proxying

If `kagzi-server` runs more than one instance and workers connect to arbitrary instances behind a load balancer, **server-proxy broker mode has a routing problem**:

- If the server consumes Kafka/NATS/Rabbit using competing-consumer semantics (consumer group / queue group), a given `WorkAvailable` signal will be received by only one server instance.
- A worker that should wake up might be connected to a different server instance and never see the signal.

Decision: for external brokers we will use **worker direct-subscribe** as the primary model. This avoids the routing problem entirely and keeps multi-server deployments simple.

### Chosen strategy: worker direct-subscribe (external brokers)

- Workers subscribe to Kafka/NATS/Rabbit directly and call `ClaimTask` on signal.
- Scaling is clean: broker load scales with workers; server replicas are decoupled.
- Keep gRPC `SubscribeWork` backed by Postgres LISTEN/NOTIFY as the “easy start” default.

### Optional future strategies (only if we ever want server-proxying)

- Fanout consumption per server replica (expensive for Kafka at scale)
- Sticky routing / consistent hashing (operationally complex)

---

## API changes (breaking allowed)

### Worker-facing gRPC: replace long-poll `PollTask` with subscribe + claim

#### ✅ New: `SubscribeWork`

- Type: server-streaming RPC
- Purpose: workers wait efficiently for signals without holding a long-poll request.
- Scope: **Postgres (LISTEN/NOTIFY) “easy start” mode**. External broker mode uses worker direct-subscribe and does not require this RPC.
- Request:
  - `namespace`
  - `task_queue`
  - `worker_id` (required)
  - (optional) `workflow_types` (allows server to reject obviously incompatible subscriptions early)
- Response stream:
  - `WorkAvailable` (or empty event payload; the stream event is the wakeup)

Notes:

- The server should validate:
  - worker exists and is ONLINE
  - worker is registered for `{namespace, task_queue}`
  - worker is not draining
- Including `worker_id` also reduces information leakage (prevents arbitrary clients from inferring queue activity).

#### ✅ New: `ClaimTask`

- Type: unary RPC
- Purpose: attempt exactly one DB claim and return a task if claimed.
- Request:
  - `namespace`
  - `task_queue`
  - `worker_id`
  - `workflow_types` (requested subset; server-enforced capability filtering)
- Response:
  - Avoid “empty strings mean none”.
  - Prefer `oneof` (or explicit `has_task`):
    - `ClaimedTask { run_id, workflow_type, input }`
    - `NoTask {}`

Authorization / correctness checks (must enforce in server):

- Worker exists and is ONLINE.
- Worker is **not draining** (draining workers must not claim new tasks).
- Worker is registered for `{namespace, task_queue}`.
- Worker type filtering is server-authoritative:
  - Treat request `workflow_types` as a _requested subset_.
  - Intersect it with the worker’s registered `workflow_types`.
  - Reject if the intersection is empty (worker must not claim tasks it is not registered for, even if it received a signal).

Post-claim side effects (must preserve current behavior):

- After a successful claim, do the same best-effort actions currently performed in `PollTask`:
  - complete pending sleep steps (if applicable)
  - record `WorkflowStarted` lifecycle event (best-effort)

#### ❌ Removed/Deprecated: `PollTask`

- Current behavior:
  - server blocks, internally subscribes to Postgres notifier, loops until deadline, then claims in DB and returns a task.
- New model:
  - worker blocks on `SubscribeWork` stream and triggers bounded `ClaimTask` calls.

---

## Worker SDK behavior (new loop)

### Registration (unchanged)

- Worker registers once (same as today): namespace + task_queue + workflow_types.
- Worker heartbeats to extend leases (same as today).

### Waiting + claiming

- Worker opens a `SubscribeWork` stream for `{namespace, task_queue}`.
- On each signal, worker does bounded drain-claim:
  - Stop conditions (in this order):
    1. no permits left
    2. claim budget exhausted
    3. empty claim returned (`NoTask`)
  - While it has permits and claim budget remaining:
    - call `ClaimTask(...)`
    - if empty: stop draining
    - if claimed: spawn execution
  - If `ClaimTask` fails with “not registered / draining / offline / wrong queue” precondition errors, treat as a hard stop for that queue and back off (do not spin).

### Fallback

- Add a periodic timer (e.g. every 5–30s) to call `ClaimTask` once even without signals.

### Backoff / jitter

- If a worker repeatedly receives signals but gets `NoTask` (due to races), apply a small randomized backoff before the next claim burst.
- Keep the claim burst bounded by available concurrency (do not spam claims when no permits are available).

### Ack semantics (external brokers)

- Ack/commit the broker message immediately after triggering drain-claim attempt.
- Never keep broker messages “in flight” during workflow execution.

---

## Server-side “due-work enqueuer” (portable delayed readiness)

We cannot rely on delayed-message semantics across Kafka/NATS/Rabbit consistently. `available_at` is the canonical scheduler.

Implement a background task that:

- Every `T` (e.g. 1–5s), queries Postgres for **distinct `{namespace, task_queue}`** that currently have due work:
  - status in `PENDING`, `SLEEPING`, and `RUNNING` where lease expired (orphan recovery)
  - `available_at <= now`
- Publishes one `WorkAvailable` per queue (coalesce to avoid spamming).
- Rate limits and adds jitter to avoid floods.

This provides:

- wakeups for sleep timers
- wakeups for retries that become due
- recovery after missed broker signals

### Performance considerations (important)

The “distinct `{namespace, task_queue}`” query can become expensive on large `workflow_runs` tables.

Mitigations:

- Put this into the existing coordinator/watchdog loop so cadence and rate limiting are controlled.
- Add a hard per-tick cap (publish at most `N` queues per tick; rely on next tick + duplicates).
- If scale requires it: introduce a small **queue-state table** (one row per `{namespace, task_queue}` with `next_due_at`), updated transactionally when workflows are created/updated. Then the enqueuer scans that table instead of scanning `workflow_runs`.

---

## Backends (pluggable work-signal bus)

### Default “easy start”

- **Postgres LISTEN/NOTIFY** (already exists as a notifier).
- Server uses it to implement `SubscribeWork`.
- Workers only need gRPC credentials (no DB creds).

### Optional production backends

- **NATS JetStream**
- **Kafka**
- **RabbitMQ**

Primary implementation mode (chosen):

- Workers subscribe directly to the broker and call `ClaimTask` on signal.
- Server publishes `WorkAvailable` into the broker (from StartWorkflow / coordinator / enqueuer).

### SQLite (later)

- SQLite has no cross-process LISTEN/NOTIFY.
- Use in-memory signals + due-work enqueuer + worker-side periodic claim tick.
- Still goes through the same `ClaimTask` path (the “DB claim” becomes SQLite-compatible claim logic).

---

## Implementation checklist (high level)

### A) Contracts and traits

- [x] Define `WorkAvailable { namespace, task_queue }` type (crate-shared, `kagzi-queue`).
- [x] Replace/rename `QueueNotifier` to `WorkSignalBus` (work-signal bus semantics).
- [x] Keep Postgres LISTEN/NOTIFY backend as the default implementation.

### B) gRPC and proto

- [ ] Add worker RPC: `SubscribeWork` (server-streaming).
- [ ] Add worker RPC: `ClaimTask` (unary).
- [ ] Remove or deprecate `PollTask` and update server + SDK accordingly.
- [ ] Ensure `SubscribeWork` validates worker registration/authorization (include `worker_id`).
- [ ] Ensure `ClaimTask` response uses `oneof` (or explicit `has_task`) rather than “empty fields”.
- [ ] Ensure `ClaimTask` validates worker is ONLINE, not draining, and registered for `{namespace, task_queue}`.
- [ ] Ensure `ClaimTask` enforces workflow type authorization by intersecting requested `workflow_types` with the worker’s registered `workflow_types` (reject empty intersection).

### C) Server changes (`kagzi-server`)

- [ ] Implement `ClaimTask` by calling the existing DB claim (`poll_workflow`) once.
- [ ] Preserve post-claim side effects currently done by `PollTask` (sleep completion + lifecycle event).
- [ ] Implement `SubscribeWork` by bridging work-signal bus subscriptions into a stream.
- [ ] Publish `WorkAvailable` on:
  - [ ] `StartWorkflow` when `available_at <= now`
  - [ ] schedule fire when instances are created
  - [ ] immediate retries (if any)
  - [ ] admin/manual transitions that make work runnable now (e.g. retry-now / resume-now)
- [ ] Add periodic due-work enqueuer:
  - [ ] DB query for distinct due queues
  - [ ] publish with coalescing + rate limiting
  - [ ] hard cap queues-per-tick to bound DB and broker load
  - [ ] (optional) queue-state table design if DISTINCT query becomes a bottleneck

### D) Worker SDK changes (`kagzi-rs`)

- [ ] Replace poll loop with:
  - [ ] subscribe stream
  - [ ] bounded drain-claim with concurrency permits
  - [ ] periodic fallback claim tick
- [ ] Keep heartbeat/lease extension behavior (as-is).
- [ ] Add optional direct-subscribe adapters for external brokers (Kafka/NATS/Rabbit) that trigger drain-claim on `WorkAvailable`.

### E) DB hardening (for contention + performance)

- [ ] Confirm indexes match claim query:
  - [ ] `(namespace, task_queue, available_at)` partial on relevant statuses
- [ ] Add jitter/backoff behavior on repeated empty claims (worker side).
- [ ] Add claim budget per signal to avoid stampedes.

### F) Tests (target real failure modes)

- [ ] Lost signal: due work exists, no signals published → enqueuer/fallback still processes.
- [ ] Duplicate signals: multiple signals → only one execution (DB claim).
- [ ] Hot queue: many signals → bounded drain-claim prevents runaway DB load.
- [ ] Orphan recovery: worker crashes → lease expires → another worker can claim.
- [ ] Draining worker: worker enters draining state → `ClaimTask` rejects new claims.
- [ ] Authorization: worker receives signal for a queue/type it is not registered for → `ClaimTask` rejects.
- [ ] (If server-proxy broker mode is implemented later) Multi-server signal routing: worker connected to server A still wakes when signals are consumed by server B (or explicitly document proxy requirements).
- [ ] Bounded stall guarantee: with only fallback tick enabled, worst-case “work starts within ≤ X seconds + jitter” (assert in an integration-ish test).

### G) Documentation

- [ ] Update `ARCHITECTURE.md` to reflect: “bus distributes signals, DB delegates ownership”.
- [ ] Document ack semantics and at-least-once behavior.
- [ ] Add quickstart guidance:
  - [ ] Postgres-only default (zero extra infra)
  - [ ] Optional NATS/Kafka/Rabbit configuration

---

## Recommended configuration defaults

- Work-signal backend: Postgres LISTEN/NOTIFY (default).
- Due-work enqueuer interval: 1–5s (configurable).
- Worker drain-claim cap per signal: 50–200 (configurable).
- Worker fallback claim tick: 5–30s (configurable).
- Jitter: small random delay before drain on signal (configurable).

---

## Open questions (to decide before coding)

- External brokers: **worker direct-subscribe** (chosen primary model). `SubscribeWork` remains for Postgres “easy start”.
- Multi-tenancy encoding: avoid topic-per-queue explosion.
  - Kafka: single topic (global or per-namespace), record key = `namespace:task_queue` to keep per-queue ordering while consumer group load-balances.
  - NATS JetStream: subject `kagzi.work.<namespace>.<task_queue>` with queue group semantics.
  - RabbitMQ: queue per `{namespace, task_queue}` is acceptable; keep payload for observability.
- Publishing strategy: **hybrid**
  - Publish `WorkAvailable` immediately when work becomes runnable **now** (start workflow, schedule fire, immediate retry).
  - Do not rely on delayed broker messages for `available_at`.
  - Rely on the periodic due-work enqueuer for `available_at` transitions, missed publishes, and outages.
  - Add per-queue debounce/coalescing to prevent spam (e.g. max 1 publish per queue per 100–500ms).
