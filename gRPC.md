# Kagzi gRPC API Documentation

This document provides comprehensive details about all gRPC RPCs in the Kagzi workflow engine, including their implementation status, logic, and quality assessments.

## Table of Contents

- [Core Workflow APIs](#core-workflow-apis)
- [Step Management APIs](#step-management-apis) 
- [Worker APIs](#worker-apis)
- [Query & Management APIs](#query--management-apis)
- [Implementation Summary](#implementation-summary)

---

## Core Workflow APIs

### 1. StartWorkflow

**Purpose**: Initiates a new workflow execution instance.

**Request**: `StartWorkflowRequest`
```protobuf
message StartWorkflowRequest {
  string workflow_id = 1;    // Business ID (e.g., "order-123")
  string task_queue = 2;     // Target queue for workers
  string workflow_type = 3;  // Workflow function name
  bytes input = 4;           // JSON serialized input
  
  // Advanced options
  string namespace_id = 5;           // Multi-tenancy (default: "default")
  string idempotency_key = 6;       // Prevent duplicates
  bytes context = 7;                 // Metadata JSON
  google.protobuf.Timestamp deadline_at = 8;  // Execution deadline
  string version = 9;                // Workflow version
}
```

**Response**: `StartWorkflowResponse`
```protobuf
message StartWorkflowResponse {
  string run_id = 1;  // Generated UUID for this execution
}
```

**Implementation Status**: ✅ **100% COMPLETE**

**Logic Flow**:
1. Validate JSON input/context
2. Check idempotency key if provided
3. Insert into `workflow_runs` table with status 'PENDING'
4. Return generated `run_id`

**Quality Assessment**: 
- ✅ **Production Ready**
- ✅ Full validation and error handling
- ✅ Idempotency support
- ✅ Namespace support
- ✅ Proper timestamp handling

---

### 2. GetWorkflowRun

**Purpose**: Retrieve details and current status of a specific workflow run.

**Request**: `GetWorkflowRunRequest`
```protobuf
message GetWorkflowRunRequest {
  string run_id = 1;        // UUID of workflow run
  string namespace_id = 2;  // Namespace filter
}
```

**Response**: `GetWorkflowRunResponse`
```protobuf
message GetWorkflowRunResponse {
  WorkflowRun workflow_run = 1;  // Complete workflow details
}
```

**Implementation Status**: ❌ **0% COMPLETE**

**Current Behavior**: Returns `unimplemented` error

**Required Logic**:
1. Validate UUID format
2. Query `workflow_runs` table by run_id and namespace
3. Map database fields to `WorkflowRun` message
4. Handle not found case

**Quality Assessment**: 
- ❌ **Not Implemented**
- ❌ No error handling
- ❌ Missing database queries

---

### 3. ListWorkflowRuns

**Purpose**: List workflow runs with pagination and filtering.

**Request**: `ListWorkflowRunsRequest`
```protobuf
message ListWorkflowRunsRequest {
  int32 page_size = 1;           // Max results per page
  string page_token = 2;         // Pagination cursor
  string filter_status = 3;      // Filter by status (optional)
  string namespace_id = 4;       // Namespace filter
}
```

**Response**: `ListWorkflowRunsResponse`
```protobuf
message ListWorkflowRunsResponse {
  repeated WorkflowRun workflow_runs = 1;  // Results
  string next_page_token = 2;               // Forward pagination
  string prev_page_token = 3;               // Backward pagination
  bool has_more = 4;                        // More results available
}
```

**Implementation Status**: ⚠️ **10% COMPLETE**

**Current Behavior**: Returns empty response with default values

**Required Logic**:
1. Parse and validate pagination parameters
2. Build WHERE clause with optional status filter
3. Query with LIMIT/OFFSET or cursor-based pagination
4. Generate pagination tokens
5. Map results to `WorkflowRun` messages

**Quality Assessment**: 
- ❌ **Not Functional**
- ❌ No database queries
- ❌ Missing pagination logic
- ❌ No filtering implementation

---

### 4. CancelWorkflowRun

**Purpose**: Cancel a running or pending workflow.

**Request**: `CancelWorkflowRunRequest`
```protobuf
message CancelWorkflowRunRequest {
  string run_id = 1;        // UUID of workflow run
  string namespace_id = 2;  // Namespace filter
}
```

**Response**: `Empty`

**Implementation Status**: ❌ **0% COMPLETE**

**Current Behavior**: Returns empty response without any state changes

**Required Logic**:
1. Validate UUID format
2. Check current workflow status (can't cancel completed/failed)
3. Update status to 'CANCELLED'
4. Clear worker locks
5. Handle not found case

**Quality Assessment**: 
- ❌ **Not Implemented**
- ❌ No state changes
- ❌ No validation

---

## Step Management APIs

### 5. GetStepAttempt

**Purpose**: Retrieve details of a specific step attempt.

**Request**: `GetStepAttemptRequest`
```protobuf
message GetStepAttemptRequest {
  string step_attempt_id = 1;  // UUID of step attempt (now step_runs.attempt_id)
  string namespace_id = 2;     // Namespace filter
}
```

**Response**: `GetStepAttemptResponse`
```protobuf
message GetStepAttemptResponse {
  StepAttempt step_attempt = 1;  // Complete step details
}
```

**Implementation Status**: ⚠️ **70% COMPLETE**

**Logic Flow**:
1. ✅ Validate UUID format
2. ✅ Query `step_runs` table (simplified - no separate step_attempts table)
3. ✅ Map database fields to response
4. ❌ TODO: Map status string to enum (hardcoded to 0)
5. ❌ TODO: Store/fetch step kind (hardcoded to 0)
6. ❌ TODO: Fetch namespace_id from DB (hardcoded to "default")

**Database Design Note**: 
- ✅ **Simplified Architecture**: Uses `step_runs` table with `attempt_id` and `attempt_number`
- ✅ No separate `step_attempts` table needed
- ✅ `is_latest` flag identifies current attempt
- ✅ Full attempt history stored in single table

**Quality Assessment**: 
- ⚠️ **Partially Functional**
- ✅ Core database operations work
- ❌ Enum mapping incomplete
- ❌ Some fields hardcoded

---

### 6. ListStepAttempts

**Purpose**: List all step attempts for a workflow run.

**Request**: `ListStepAttemptsRequest`
```protobuf
message ListStepAttemptsRequest {
  string workflow_run_id = 1;  // UUID of workflow run
  string step_id = 2;          // Optional: filter by step ID
  int32 page_size = 3;         // Max results per page
  string page_token = 4;        // Pagination cursor
  string namespace_id = 5;     // Namespace filter
}
```

**Response**: `ListStepAttemptsResponse`
```protobuf
message ListStepAttemptsResponse {
  repeated StepAttempt step_attempts = 1;  // Results
  string next_page_token = 2;               // Pagination token
}
```

**Implementation Status**: ⚠️ **70% COMPLETE**

**Logic Flow**:
1. ✅ Validate workflow run UUID
2. ✅ Query `step_runs` table with LIMIT (simplified - no separate step_attempts table)
3. ✅ Map results to response
4. ❌ TODO: Same enum/status mapping issues as GetStepAttempt
5. ❌ TODO: Missing step_id filter implementation
6. ❌ TODO: Missing pagination token logic

**Database Design Note**: 
- ✅ **Simplified Query**: `SELECT * FROM step_runs WHERE workflow_run_id = $1 ORDER BY attempt_number`
- ✅ Single table contains all attempt history
- ✅ `step_id` filter can use existing index

**Quality Assessment**: 
- ⚠️ **Partially Functional**
- ✅ Basic listing works
- ❌ Incomplete filtering
- ❌ No proper pagination

---

## Worker APIs

### 7. PollActivity

**Purpose**: Long-polling endpoint for workers to fetch available work.

**Request**: `PollActivityRequest`
```protobuf
message PollActivityRequest {
  string task_queue = 1;     // Queue to poll from
  string worker_id = 2;      // Worker identifier
  string namespace_id = 3;   // Namespace filter
}
```

**Response**: `PollActivityResponse`
```protobuf
message PollActivityResponse {
  string run_id = 1;              // Workflow run UUID
  string workflow_type = 2;       // Workflow function name
  bytes workflow_input = 3;       // JSON serialized input
}
```

**Implementation Status**: ✅ **100% COMPLETE**

**Logic Flow**:
1. Start 60-second long poll loop
2. Use `FOR UPDATE SKIP LOCKED` to claim pending workflows
3. Update workflow status to 'RUNNING' with worker lock
4. Return claimed workflow details
5. Handle timeout with `DeadlineExceeded` status

**Quality Assessment**: 
- ✅ **Production Ready**
- ✅ Proper long polling implementation
- ✅ Database locking prevents race conditions
- ✅ Timeout handling
- ✅ Worker lock management

---

### 8. RecordHeartbeat

**Purpose**: Workers send periodic heartbeats to maintain workflow locks.

**Request**: `RecordHeartbeatRequest`
```protobuf
message RecordHeartbeatRequest {
  string run_id = 1;    // Workflow run UUID
  string worker_id = 2;  // Worker identifier
}
```

**Response**: `Empty`

**Implementation Status**: ❌ **0% COMPLETE**

**Current Behavior**: Returns empty response without extending locks

**Required Logic**:
1. Validate UUID format
2. Verify worker owns the workflow lock
3. Extend `locked_until` timestamp
4. Handle not found/stolen lock cases

**Quality Assessment**: 
- ❌ **Not Implemented**
- ❌ No lock extension
- ❌ Workers may lose workflows

---

### 9. BeginStep

**Purpose**: Check if a step has been executed before (memoization).

**Request**: `BeginStepRequest`
```protobuf
message BeginStepRequest {
  string run_id = 1;  // Workflow run UUID
  string step_id = 2; // Step identifier
}
```

**Response**: `BeginStepResponse`
```protobuf
message BeginStepResponse {
  bool should_execute = 1;  // Whether to run the step
  bytes cached_result = 2;  // Previous output if should_execute=false
}
```

**Implementation Status**: ✅ **100% COMPLETE**

**Logic Flow**:
1. Query `step_runs` table for latest completed step (`WHERE is_latest = true`)
2. If found and completed, return cached result
3. If not found or failed, indicate should_execute=true

**Database Design Note**: 
- ✅ **Efficient Query**: Uses `idx_step_runs_latest` index for fast lookup
- ✅ **Latest Attempt**: `is_latest = true` ensures we get current state
- ✅ **Attempt History**: All previous attempts preserved for debugging

**Quality Assessment**: 
- ✅ **Production Ready**
- ✅ Proper memoization logic
- ✅ Efficient database queries
- ✅ Correct caching behavior

---

### 10. CompleteStep

**Purpose**: Mark a step as successfully completed with its output.

**Request**: `CompleteStepRequest`
```protobuf
message CompleteStepRequest {
  string run_id = 1;  // Workflow run UUID
  string step_id = 2; // Step identifier
  bytes output = 3;   // JSON serialized output
}
```

**Response**: `Empty`

**Implementation Status**: ✅ **100% COMPLETE**

**Logic Flow**:
1. Validate JSON output format
2. Insert/update `step_runs` table with COMPLETED status
3. Store output and completion timestamp
4. Use UPSERT to handle retries

**Database Design Note**: 
- ✅ **Attempt Management**: Each completion creates new attempt record
- ✅ **History Tracking**: Previous attempts preserved for audit
- ✅ **Latest Flag**: New completion marked as `is_latest = true`

**Quality Assessment**: 
- ✅ **Production Ready**
- ✅ Proper UPSERT handling
- ✅ JSON validation
- ✅ Timestamp management

---

### 11. FailStep

**Purpose**: Mark a step as failed with error details.

**Request**: `FailStepRequest`
```protobuf
message FailStepRequest {
  string run_id = 1;  // Workflow run UUID
  string step_id = 2; // Step identifier
  string error = 3;   // Error message
}
```

**Response**: `Empty`

**Implementation Status**: ❌ **0% COMPLETE**

**Current Behavior**: Returns empty response without recording failure

**Required Logic**:
1. Validate UUID format
2. Create new attempt in `step_runs` with FAILED status
3. Store error message and timestamp
4. Update `is_latest` flag appropriately
5. Consider retry logic vs immediate failure

**Database Design Note**: 
- ✅ **Attempt Tracking**: Each failure creates new row with incremented `attempt_number`
- ✅ **History Preservation**: Previous attempts remain for audit trail
- ✅ **Latest Flag**: `is_latest = true` identifies current attempt

**Quality Assessment**: 
- ❌ **Not Implemented**
- ❌ No error recording
- ❌ No retry handling

---

### 12. CompleteWorkflow

**Purpose**: Mark an entire workflow as successfully completed.

**Request**: `CompleteWorkflowRequest`
```protobuf
message CompleteWorkflowRequest {
  string run_id = 1;  // Workflow run UUID
  bytes output = 2;   // JSON serialized final output
}
```

**Response**: `Empty`

**Implementation Status**: ✅ **100% COMPLETE**

**Logic Flow**:
1. Validate JSON output format
2. Update `workflow_runs` status to 'COMPLETED'
3. Store final output and completion timestamp
4. Clear worker locks

**Quality Assessment**: 
- ✅ **Production Ready**
- ✅ Proper state transitions
- ✅ Lock cleanup
- ✅ JSON validation

---

### 13. FailWorkflow

**Purpose**: Mark an entire workflow as failed.

**Request**: `FailWorkflowRequest`
```protobuf
message FailWorkflowRequest {
  string run_id = 1;  // Workflow run UUID
  string error = 2;   // Error message
}
```

**Response**: `Empty`

**Implementation Status**: ✅ **100% COMPLETE**

**Logic Flow**:
1. Validate UUID format
2. Update `workflow_runs` status to 'FAILED'
3. Store error message and completion timestamp
4. Clear worker locks

**Quality Assessment**: 
- ✅ **Production Ready**
- ✅ Proper error handling
- ✅ Lock cleanup
- ✅ State management

---

### 14. ScheduleSleep

**Purpose**: Put a workflow to sleep for a specified duration.

**Request**: `ScheduleSleepRequest`
```protobuf
message ScheduleSleepRequest {
  string run_id = 1;           // Workflow run UUID
  uint64 duration_seconds = 2; // Sleep duration
}
```

**Response**: `Empty`

**Implementation Status**: ✅ **100% COMPLETE**

**Logic Flow**:
1. Validate UUID format
2. Update `workflow_runs` status to 'SLEEPING'
3. Set `wake_up_at = NOW() + duration`
4. Clear worker locks
5. Background reaper will wake up workflow

**Quality Assessment**: 
- ✅ **Production Ready**
- ✅ Proper sleep scheduling
- ✅ Lock management
- ✅ Integration with reaper

---

## Implementation Summary

### ✅ **Production Ready** (7/14 APIs)
- `StartWorkflow` - Complete workflow initiation
- `PollActivity` - Core worker polling with proper locking
- `BeginStep` - Step memoization
- `CompleteStep` - Step completion
- `CompleteWorkflow` - Workflow completion
- `FailWorkflow` - Workflow failure
- `ScheduleSleep` - Sleep scheduling

### ⚠️ **Partially Implemented** (2/14 APIs)
- `GetStepAttempt` - 70% complete, uses enhanced `step_runs` schema, missing enum mapping
- `ListStepAttempts` - 70% complete, uses `attempt_number` ordering, missing filtering/pagination

### ❌ **Not Implemented** (5/14 APIs)
- `GetWorkflowRun` - Core observability missing
- `ListWorkflowRuns` - Management interface missing  
- `CancelWorkflowRun` - Workflow control missing
- `RecordHeartbeat` - Worker lifecycle missing (critical for production)
- `FailStep` - Step error handling missing (critical for retry logic)

### **🏗️ Architecture Simplification - COMPLETED**

**✅ Simplified Step Management**: 
- **Removed redundant `step_attempts` table** completely
- **Enhanced `step_runs` table** with comprehensive attempt tracking:
  - `attempt_id` (UUID primary key) - Unique attempt identifier
  - `attempt_number` (INTEGER) - Sequential attempt ordering (1, 2, 3...)
  - `is_latest` (BOOLEAN) - Current attempt flag for fast lookups
  - `input` (JSONB) - Step input data preservation
  - Full attempt history in single table with proper indexing

**Database Schema Benefits**:
- **Single source of truth** - No JOINs between step tables needed
- **Optimal performance** - Specialized indexes for common query patterns:
  - `idx_step_runs_latest` - Fast BeginStep lookups
  - `idx_step_runs_history` - Efficient attempt history queries
- **Complete audit trail** - Every step execution preserved with timestamps
- **Retry-ready** - Attempt numbering enables sophisticated retry logic

**Query Pattern Examples**:
```sql
-- Current step state (BeginStep)
SELECT * FROM step_runs WHERE run_id = $1 AND step_id = $2 AND is_latest = true;

-- Complete attempt history (ListStepAttempts)  
SELECT * FROM step_runs WHERE run_id = $1 ORDER BY attempt_number ASC;

-- Create new attempt (CompleteStep/FailStep)
UPDATE step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2;
INSERT INTO step_runs (run_id, step_id, attempt_number, is_latest, ...)
VALUES ($1, $2, (SELECT MAX(attempt_number)+1 FROM step_runs WHERE run_id = $1 AND step_id = $2), true, ...);
```

### **Overall Quality Assessment**

**Core Execution Flow**: ✅ **100% Functional**
- Workflows can be started, executed, and completed
- Step memoization works correctly
- Sleep/wake cycles function properly
- Worker polling and locking is robust

**Observability**: ⚠️ **30% Complete**
- Basic step attempt queries work
- Missing workflow run queries
- No proper filtering or pagination

**Management & Control**: ❌ **20% Complete**
- No workflow cancellation
- No worker heartbeat/lock extension
- No step-level failure handling

**Production Readiness**: ⚠️ **75% Complete**
- ✅ Core workflow execution is solid with simplified architecture
- ✅ Step attempt tracking and memoization working correctly
- ✅ Database schema optimized for performance
- ❌ Missing critical management features (heartbeat, cancellation)
- ❌ Need observability improvements (workflow queries)
- ❌ Step-level error handling incomplete

### **Priority Implementation Order**

1. **Critical Priority** (Production blockers)
   - `FailStep` - Essential for retry logic and error handling
   - `RecordHeartbeat` - Prevents workflow abandonment, extends worker locks
   - `GetWorkflowRun` - Basic observability for monitoring

2. **High Priority** (Production readiness)
   - `CancelWorkflowRun` - Workflow control and cleanup
   - `ListWorkflowRuns` - Management interface and monitoring
   - Complete enum mapping in step APIs (status, kind enums)

3. **Medium Priority** (Enhanced reliability)
   - Proper pagination in list APIs
   - Advanced filtering capabilities
   - Step retry logic with attempt limits

4. **Low Priority** (Performance optimizations)
   - Batch processing for reaper
   - Connection pool tuning
   - Query performance monitoring

### **Architecture Status**: ✅ **SOLID FOUNDATION**

The simplified single-table `step_runs` architecture provides:
- **Excellent performance** with specialized indexes
- **Complete audit trails** with attempt tracking
- **Retry-ready foundation** for sophisticated error handling
- **Production-grade database design** for scalability

The system has a **solid architectural foundation** and is ready for production deployment once critical management APIs are implemented.