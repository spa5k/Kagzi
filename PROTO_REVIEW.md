# Proto File API Structure Review

## Comparison with Inngest REST API v2 & OpenWorkflow

### Current State Analysis

Your proto file defines a gRPC API (not REST) with:

- **Client-facing**: Start, Get, List, Cancel workflow runs
- **Worker-facing**: Poll, Heartbeat, Step management, Workflow completion
- **Core entities**: WorkflowRun, Step runs (embedded in workflow)

---

## ✅ What You Have (Good)

### Core Workflow Operations

- ✅ Start workflow (`StartWorkflow`)
- ✅ Get workflow run (`GetWorkflowRun`)
- ✅ List workflow runs (`ListWorkflowRuns`)
- ✅ Cancel workflow run (`CancelWorkflowRun`)
- ✅ Poll for work (`PollActivity`)
- ✅ Step execution tracking (`BeginStep`, `CompleteStep`, `FailStep`)
- ✅ Workflow completion (`CompleteWorkflow`, `FailWorkflow`)
- ✅ Sleep functionality (`ScheduleSleep`)
- ✅ Heartbeat mechanism (`RecordHeartbeat`)

### Data Model

- ✅ WorkflowRun with status, input, output, error
- ✅ Timestamps (created_at, started_at, finished_at, wake_up_at)
- ✅ Attempts tracking
- ✅ Task queue support

---

## ❌ Missing from Database Schema (Already in DB, Not in Proto)

Your database schema has these fields that aren't exposed in the proto:

### 1. **Namespace/Environment Support**

```sql
namespace_id TEXT NOT NULL DEFAULT 'default'
```

- **Inngest**: Uses `X-Inngest-Env` header for environment scoping
- **OpenWorkflow**: Has `namespaceId` on all resources
- **Your DB**: Has `namespace_id` but proto doesn't expose it
- **Recommendation**: Add `namespace_id` to requests/responses for multi-tenancy

### 2. **Idempotency Key**

```sql
idempotency_key TEXT
```

- **Inngest**: Supports idempotency for function invocations
- **OpenWorkflow**: Has `idempotencyKey` in `CreateWorkflowRunParams`
- **Your DB**: Has unique constraint on `(namespace_id, idempotency_key)`
- **Recommendation**: Add `idempotency_key` to `StartWorkflowRequest`

### 3. **Context Field**

```sql
context JSONB  -- Tracing/Metadata
```

- **OpenWorkflow**: Has `context: JsonValue | null` for runtime metadata
- **Your DB**: Has `context` column
- **Recommendation**: Add `context` to `WorkflowRun` message and `StartWorkflowRequest`

### 4. **Deadline/Timeout**

```sql
deadline_at TIMESTAMPTZ
```

- **OpenWorkflow**: Has `deadlineAt: Date | null` for workflow timeouts
- **Your DB**: Has `deadline_at` column
- **Recommendation**: Add `deadline_at` to `WorkflowRun` and `StartWorkflowRequest`

### 5. **Worker ID Tracking**

```sql
locked_by TEXT  -- Worker ID
locked_until TIMESTAMPTZ
```

- **OpenWorkflow**: Has `workerId: string | null` on WorkflowRun
- **Your DB**: Tracks `locked_by` (worker ID) and `locked_until`
- **Recommendation**: Add `worker_id` to `WorkflowRun` message (read-only, from server)

---

## 🚫 Missing Features (Not in DB or Proto)

### 1. **Step Attempts as Separate Entity**

**OpenWorkflow Pattern:**

```typescript
interface StepAttempt {
  namespaceId: string;
  id: string;
  workflowRunId: string;
  stepName: string;
  kind: StepKind; // "function" | "sleep"
  status: StepAttemptStatus;
  config: JsonValue;
  context: StepAttemptContext | null;
  output: JsonValue | null;
  error: JsonValue | null;
  childWorkflowRunId: string | null;
  startedAt: Date | null;
  finishedAt: Date | null;
  createdAt: Date;
  updatedAt: Date;
}
```

**Your Current Approach:**

- Steps are tracked in `step_runs` table but not exposed as first-class entities
- No API to list step attempts for a workflow run
- No step attempt IDs (only composite key: run_id + step_id)

**Recommendation:**

- Add `GetStepAttempt` and `ListStepAttempts` RPCs
- Add `StepAttempt` message type
- Consider adding `step_attempt_id` to step_runs table (or use ULID)

### 2. **Workflow Versioning**

**OpenWorkflow Pattern:**

```typescript
version: string | null; // Workflow version
```

**Inngest Pattern:**

- Functions have versions for deployment management

**Recommendation:**

- Add `version` field to `WorkflowRun` and `StartWorkflowRequest`
- Useful for A/B testing, gradual rollouts, and debugging

### 3. **Parent/Child Workflow Relationships**

**OpenWorkflow Pattern:**

```typescript
parentStepAttemptNamespaceId: string | null;
parentStepAttemptId: string | null;
childWorkflowRunNamespaceId: string | null;
childWorkflowRunId: string | null;
```

**Use Case:**

- Sub-workflows spawned from steps
- Workflow composition

**Recommendation:**

- Add parent/child relationship fields to `WorkflowRun`
- Add `child_workflow_run_id` to `step_runs` table

### 4. **Replay Functionality**

**Inngest Pattern:**

```bash
POST /v2/runs/{runId}/replay  # Replay run
```

**Use Case:**

- Debugging failed workflows
- Testing workflow logic
- Recovering from transient failures

**Recommendation:**

- Add `ReplayWorkflowRun` RPC
- Takes `run_id` and optional `from_step_id` parameter

### 5. **Events System**

**Inngest Pattern:**

```bash
POST /v2/events              # Send event
GET  /v2/events               # List events
GET  /v2/events/{eventId}     # Get event details
GET  /v2/events/{eventId}/runs  # Get runs triggered by event
```

**Use Case:**

- Event-driven workflow triggers
- Event sourcing
- Audit trail

**Recommendation:**

- Consider adding events as separate service/entity
- Or keep current direct invocation model (simpler)

### 6. **Function/Workflow Definition Management**

**Inngest Pattern:**

```bash
GET    /v2/functions              # List functions
GET    /v2/functions/{functionId} # Get function details
PUT    /v2/functions/{functionId} # Update function
DELETE /v2/functions/{functionId} # Delete function
POST   /v2/functions/{functionId}/invoke  # Trigger function
POST   /v2/functions/{functionId}/pause   # Pause function
POST   /v2/functions/{functionId}/resume   # Resume function
```

**Your Current Approach:**

- Workflows are implicit (defined in worker code)
- No workflow definition management

**Recommendation:**

- **Option A**: Keep current approach (simpler, workflows are code)
- **Option B**: Add workflow definition management (more complex, but enables dynamic workflows)

---

## 🔧 API Design Improvements

### 1. **Pagination Strategy**

**Current:**

```protobuf
message ListWorkflowRunsRequest {
  int32 page_size = 1;
  string page_token = 2;
}
message ListWorkflowRunsResponse {
  repeated WorkflowRun workflow_runs = 1;
  string next_page_token = 2;
}
```

**Inngest Pattern (Cursor-based):**

```json
{
  "page": {
    "cursor": "01hp1zx8m3ng9vp6qn0xk7j4cz",
    "hasMore": true,
    "limit": 50
  }
}
```

**OpenWorkflow Pattern:**

```typescript
{
  pagination: {
    next: string | null;
    prev: string | null;
  }
}
```

**Recommendation:**

- Keep `page_token` approach (works well with gRPC)
- Add `has_more` boolean to response
- Consider bidirectional pagination (next/prev) like OpenWorkflow

### 2. **Error Handling**

**Current:**

- Uses gRPC status codes (standard, but not structured)

**Inngest Pattern:**

```json
{
  "errors": [
    {
      "code": "workflow_not_found",
      "message": "Workflow 'abc123' not found"
    }
  ]
}
```

**Recommendation:**

- For gRPC, use `google.rpc.Status` with error details
- Define error codes enum (e.g., `WORKFLOW_NOT_FOUND`, `STEP_ALREADY_COMPLETED`)
- Add structured error details in `google.rpc.ErrorInfo`

### 3. **Response Envelope**

**Inngest Pattern:**

```json
{
  "data": {...},
  "metadata": {
    "fetchedAt": "2025-08-11T10:30:00Z",
    "cachedUntil": null
  }
}
```

**Recommendation:**

- For gRPC, this is less critical (no caching headers needed)
- Consider adding `metadata` to responses if you add caching later

### 4. **Field Naming Consistency**

**Current:**

- Uses `snake_case` (standard for protobuf)

**Inngest Pattern:**

- Uses `camelCase` for JSON (REST API)

**Recommendation:**

- Keep `snake_case` for protobuf (standard)
- If you add REST gateway, use `camelCase` in JSON (protobuf-to-JSON mapping handles this)

### 5. **ID Formats**

**Current:**

- Uses UUIDs for `run_id`

**Inngest Pattern:**

- Events/Runs use ULIDs (sortable, time-ordered)
- Functions use composite IDs (`app-slug:function-slug`)

**OpenWorkflow Pattern:**

- Uses string IDs (could be ULIDs or UUIDs)

**Recommendation:**

- Consider ULIDs for workflow runs (better for time-ordered queries)
- Keep UUIDs if you need random, non-guessable IDs
- Current UUID approach is fine

### 6. **Timestamp Precision**

**Current:**

- Uses `google.protobuf.Timestamp` (nanosecond precision)

**Inngest Pattern:**

- RFC 3339 with millisecond precision

**Recommendation:**

- Keep `google.protobuf.Timestamp` (standard, high precision)
- When converting to JSON (REST gateway), use RFC 3339 format

---

## 📋 Recommended Additions to Proto

### High Priority

1. **Add missing DB fields to proto:**
   - `namespace_id` to requests/responses
   - `idempotency_key` to `StartWorkflowRequest`
   - `context` to `WorkflowRun` and `StartWorkflowRequest`
   - `deadline_at` to `WorkflowRun` and `StartWorkflowRequest`
   - `worker_id` to `WorkflowRun` (read-only, from server)

2. **Add Step Attempts API:**
   ```protobuf
   rpc GetStepAttempt (GetStepAttemptRequest) returns (GetStepAttemptResponse);
   rpc ListStepAttempts (ListStepAttemptsRequest) returns (ListStepAttemptsResponse);

   message StepAttempt {
     string step_attempt_id = 1;
     string workflow_run_id = 2;
     string step_id = 3;
     StepKind kind = 4;
     StepAttemptStatus status = 5;
     bytes config = 6;
     bytes context = 7;
     bytes output = 8;
     bytes error = 9;
     google.protobuf.Timestamp started_at = 10;
     google.protobuf.Timestamp finished_at = 11;
     google.protobuf.Timestamp created_at = 12;
     google.protobuf.Timestamp updated_at = 13;
   }
   ```

3. **Add version support:**
   - `version` field to `WorkflowRun` and `StartWorkflowRequest`

4. **Improve pagination:**
   ```protobuf
   message ListWorkflowRunsResponse {
     repeated WorkflowRun workflow_runs = 1;
     string next_page_token = 2;
     string prev_page_token = 3;  // Add bidirectional
     bool has_more = 4;            // Add has_more
   }
   ```

### Medium Priority

5. **Add replay functionality:**
   ```protobuf
   rpc ReplayWorkflowRun (ReplayWorkflowRunRequest) returns (ReplayWorkflowRunResponse);

   message ReplayWorkflowRunRequest {
     string run_id = 1;
     string from_step_id = 2;  // Optional: replay from specific step
   }
   ```

6. **Add parent/child relationships:**
   - `parent_step_attempt_id` to `WorkflowRun`
   - `child_workflow_run_id` to `StepAttempt`

### Low Priority (Consider for Future)

7. **Events system** (if you want event-driven workflows)
8. **Function/workflow definition management** (if you want dynamic workflows)
9. **Workflow pause/resume** (if you want workflow lifecycle management)

---

## 🗑️ Things to Consider Removing

### Nothing Critical

- Current API is lean and focused
- All endpoints serve a purpose
- No obvious bloat

---

## 📊 Summary Matrix

| Feature                | Inngest | OpenWorkflow | Your Proto | DB Schema | Priority   |
| ---------------------- | ------- | ------------ | ---------- | --------- | ---------- |
| Namespace/Environment  | ✅      | ✅           | ❌         | ✅        | **HIGH**   |
| Idempotency Key        | ✅      | ✅           | ❌         | ✅        | **HIGH**   |
| Context Field          | ❌      | ✅           | ❌         | ✅        | **HIGH**   |
| Deadline/Timeout       | ❌      | ✅           | ❌         | ✅        | **HIGH**   |
| Worker ID Tracking     | ❌      | ✅           | ❌         | ✅        | **HIGH**   |
| Step Attempts API      | ❌      | ✅           | ❌         | ✅        | **HIGH**   |
| Workflow Version       | ✅      | ✅           | ❌         | ❌        | **MEDIUM** |
| Parent/Child Workflows | ❌      | ✅           | ❌         | ❌        | **MEDIUM** |
| Replay                 | ✅      | ❌           | ❌         | ❌        | **MEDIUM** |
| Events System          | ✅      | ❌           | ❌         | ❌        | **LOW**    |
| Function Management    | ✅      | ❌           | ❌         | ❌        | **LOW**    |

---

## 🎯 Action Items

### Immediate (High Priority)

1. ✅ Add `namespace_id` to all relevant requests/responses
2. ✅ Add `idempotency_key` to `StartWorkflowRequest`
3. ✅ Add `context` to `WorkflowRun` and `StartWorkflowRequest`
4. ✅ Add `deadline_at` to `WorkflowRun` and `StartWorkflowRequest`
5. ✅ Add `worker_id` to `WorkflowRun` (read-only)
6. ✅ Add Step Attempts API (`GetStepAttempt`, `ListStepAttempts`)

### Short Term (Medium Priority)

7. Add `version` field to workflow runs
8. Add `ReplayWorkflowRun` RPC
9. Improve pagination (add `has_more`, `prev_page_token`)

### Long Term (Low Priority)

10. Consider events system
11. Consider function/workflow definition management
12. Consider parent/child workflow relationships

---

## 💡 Design Philosophy Notes

### gRPC vs REST

- Your API is gRPC (good for internal services, streaming, type safety)
- Inngest uses REST (good for public APIs, webhooks, HTTP caching)
- **Recommendation**: Keep gRPC for worker API, consider REST gateway for client API if needed

### Simplicity vs Features

- Your current API is simpler than Inngest (good for MVP)
- OpenWorkflow is more feature-complete but more complex
- **Recommendation**: Add high-priority features first, keep API lean

### Database vs Proto Mismatch

- Your DB has more fields than proto exposes
- **Recommendation**: Expose all DB fields in proto (no reason to hide them)
