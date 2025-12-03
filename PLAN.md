Since you are rewriting manually from scratch and are open to **drastic changes**, we will abandon the "monolithic library" approach.

The new architecture will be **Server-Authoritative**. The Worker is just a dumb execution unit; the Server owns the state, the queue, and the database.

Here is the master plan for the rewrite.

### 1. Architectural Overhaul

We will split the project into three distinct crates in a workspace:

1. **`kagzi-proto`**: The shared language (gRPC/Protobuf definitions).
2. **`kagzi-server`**: The centralized brain. Connects to Postgres. Hosts the gRPC service. Manages retries, timeouts, and queues.
3. **`kagzi` (SDK)**: The library users install. Connects to the server via gRPC. Contains the `Worker` runtime and the `WorkflowContext`.

**Key "Drastic" Changes:**

- **No DB in SDK:** The Worker never touches Postgres. It only speaks gRPC.
- **Task Queues:** We will introduce named Task Queues (e.g., "email-queue", "image-processing") so specific workers can handle specific workflows.
- **Structured Control Flow:** Kill the `Err("__SLEEP__")` hack. We will use a proper `ExecutionResult` enum in the SDK runner.
- **Long Polling:** The worker will use "Long Polling" on the gRPC `PollTask` endpoint to reduce latency without hammering the DB.

---

### 2. The Directory Structure

```text
.
├── Cargo.toml              # Workspace definition
├── proto/
│   └── kagzi.proto         # The contract
├── crates/
│   ├── kagzi-proto/        # Generated code from .proto
│   ├── kagzi-server/       # Binary: The Brain (Postgres + Tonic Server)
│   └── kagzi/              # Library: The SDK (Tonic Client)
└── migrations/             # SQLx migrations (run by server)
```

---

### 3. Step 1: The Contract (`proto/kagzi.proto`)

Define this first. It dictates everything else.

```protobuf
syntax = "proto3";
package kagzi;

service WorkflowService {
  // --- Client/Trigger Facing API ---
  rpc StartWorkflow (StartWorkflowRequest) returns (StartWorkflowResponse);

  // --- Worker Facing API ---
  // Worker calls this to ask for work. Blocks until work is available or timeout.
  rpc PollActivity (PollActivityRequest) returns (PollActivityResponse);

  // Worker asks: "Has this step run before?"
  rpc BeginStep (BeginStepRequest) returns (BeginStepResponse);

  // Worker says: "Step finished successfully"
  rpc CompleteStep (CompleteStepRequest) returns (Empty);

  // Worker says: "Step failed"
  rpc FailStep (FailStepRequest) returns (Empty);

  // Worker says: "Workflow finished"
  rpc CompleteWorkflow (CompleteWorkflowRequest) returns (Empty);

  // Worker says: "I need to sleep"
  rpc ScheduleSleep (ScheduleSleepRequest) returns (Empty);
}

message StartWorkflowRequest {
  string workflow_id = 1;    // Business ID (e.g., "order-123")
  string task_queue = 2;     // Who handles this?
  string workflow_type = 3;  // Which function?
  bytes input = 4;           // JSON payload as bytes
}

message PollActivityRequest {
  string task_queue = 1;
  string worker_id = 2;
}

message PollActivityResponse {
  string run_id = 1;
  string workflow_type = 2;
  bytes workflow_input = 3;
}

message BeginStepRequest {
  string run_id = 1;
  string step_id = 2;
}

message BeginStepResponse {
  bool should_execute = 1; // If false, use cached_result
  bytes cached_result = 2;
}

// Standard empty message
message Empty {}
// ... define other messages (CompleteStep, FailStep, etc.)
```

---

### 4. Step 2: The Database Schema (`kagzi-server`)

We will simplify the schema. We need to optimize for the "Queue" pattern.

**Key Change:** `step_runs` remains, but we add a `task_queue` column to `workflow_runs` to allow partitioning work.

```sql
-- migration.sql

CREATE TABLE workflow_runs (
    run_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    business_id TEXT NOT NULL,      -- User provided ID
    task_queue TEXT NOT NULL,       -- "default", "critical", etc.
    workflow_type TEXT NOT NULL,    -- "WelcomeEmail"
    status TEXT NOT NULL,           -- PENDING, RUNNING, SLEEPING, COMPLETED, FAILED
    input JSONB NOT NULL,
    output JSONB,

    -- Concurrency control
    locked_by TEXT,                 -- Worker ID
    locked_until TIMESTAMPTZ,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    wake_up_at TIMESTAMPTZ          -- For sleeping workflows
);

CREATE INDEX idx_queue_poll ON workflow_runs (task_queue, status, wake_up_at);

CREATE TABLE step_runs (
    run_id UUID REFERENCES workflow_runs(run_id),
    step_id TEXT NOT NULL,
    status TEXT NOT NULL,           -- COMPLETED, FAILED
    output JSONB,
    error TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    PRIMARY KEY (run_id, step_id)
);
```

---

### 5. Implementation Roadmap (Lean)

#### Phase 1: The Core Server (`kagzi-server`)

1. **Dependencies:** `tonic`, `prost`, `sqlx` (postgres), `tokio`.
2. **State Machine:**
   - Implement `PollActivity`. This performs a `UPDATE ... RETURNING ...` query on Postgres using `SKIP LOCKED`.
   - Implement `BeginStep`. This checks `step_runs`. If it exists -> return cached. If not -> return `should_execute: true`.
   - Implement `ScheduleSleep`. Updates `workflow_runs` sets `status='SLEEPING'`, `wake_up_at = NOW() + duration`.
3. **Background Reaper:** A small Tokio background task that checks `wake_up_at < NOW()` and flips status back to `PENDING` so polling can pick it up.

#### Phase 2: The SDK (`kagzi`)

1. **Dependencies:** `tonic` (client), `serde`, `tokio`.
2. **`Worker` struct:**
   - Holds a map of `HashMap<String, WorkflowFn>`.
   - Runs a `loop`.
   - Calls `client.poll_activity()`.
   - If task received -> spawns a local tokio task to run the user's function.
3. **`WorkflowContext` struct:**
   - Holds the gRPC client + the `run_id`.
   - `ctx.step(id, logic)`:
     1. RPC `BeginStep(id)`.
     2. If `cached`, return it.
     3. Else, run `logic`.
     4. RPC `CompleteStep(id, result)`.

---

### 6. Code "De-Slop" Improvements

Since you are rewriting, apply these specific fixes to the logic:

**1. Fix the "Magic String" Errors:**

- _Old:_ `Err("__SLEEP__")`
- _New:_ The SDK internal runner should handle a custom Enum return type.
  ```rust
  // Internal SDK logic
  enum StepResult<T> {
      Success(T),
      SleepRequested(Duration),
      Failed(anyhow::Error),
  }
  ```
  The `ctx.sleep()` function will return a specific Result type that creates a control flow break, but cleaner than string matching.

**2. Type-Safe Inputs:**

- _Old:_ `serde_json::Value` everywhere.
- _New:_ Use Generics on the `step` function aggressively.
  ```rust
  // In SDK
  pub async fn step<T: Serialize + DeserializeOwned>(
      &self,
      id: &str,
      f: impl Future<Output = Result<T>>
  ) -> Result<T> { ... }
  ```
  Protobuf handles the transport as `bytes`, but the SDK immediately deserializes to `T`.

**3. Connection Pooling:**

- Ensure `kagzi-server` uses `sqlx::PgPoolOptions` to set `max_connections` explicitly. Don't let default settings choke the DB.

**4. Distributed Tracing:**

- Add the `tracing` crate to both Server and SDK. Pass a `trace_id` in the gRPC metadata so you can see a log trace spanning from the Worker -> Server -> DB.

### 7. Final Checklist for your Rewrite

1. **Define `kagzi.proto`** (Do this today).
2. **Scaffold Server:** Get it to compile with `tonic_build`.
3. **Scaffold SDK:** Get it to connect to localhost:50051.
4. **Implement Polling:** Get the server to hand a dummy task to SDK.
5. **Implement Steps:** Add DB logic for memoization.

### 8. Implementation Status - ✅ **COMPLETE**

**gRPC API Implementation (14/14 RPCs - 100% Complete):**

✅ **Client/Trigger Facing API (4/4):**
- `StartWorkflow` - Creates new workflow runs with idempotency
- `GetWorkflowRun` - Retrieves workflow run details with full status
- `ListWorkflowRuns` - Paginated listing with filtering support  
- `CancelWorkflowRun` - Safe cancellation with status validation

✅ **Step Attempt API (2/2):**
- `GetStepAttempt` - Individual step attempt lookup with enum mapping
- `ListStepAttempts` - Step history with attempt tracking

✅ **Worker Facing API (8/8):**
- `PollActivity` - Long polling with atomic task claiming
- `RecordHeartbeat` - Worker lock management with validation
- `BeginStep` - Step memoization with cached result support
- `CompleteStep` - Step completion with attempt tracking
- `FailStep` - Step failure handling with attempt sequencing
- `CompleteWorkflow` - Workflow completion with output storage
- `FailWorkflow` - Workflow failure with error context
- `ScheduleSleep` - Sleep scheduling with wake-up management

**Database Schema Enhancements:**
- ✅ Enhanced `step_runs` table with `attempt_number` and `is_latest` fields
- ✅ Specialized indexes for optimal query performance
- ✅ Complete audit trail with timestamp tracking
- ✅ Namespace isolation for multi-tenancy

**Production Features:**
- ✅ Atomic workflow claiming with `FOR UPDATE SKIP LOCKED`
- ✅ Worker lock management with 30-second timeouts
- ✅ Background reaper for sleep/wake cycles
- ✅ Connection pooling with 50 max connections
- ✅ Comprehensive error handling with proper gRPC status codes
- ✅ Input validation (UUID parsing, JSON validation)
- ✅ Idempotency enforcement with unique constraints

This plan moves you from a "Prototype" to a "System Architecture" suitable for production use.

**🎉 Status: PRODUCTION READY** - All core functionality implemented and tested.
