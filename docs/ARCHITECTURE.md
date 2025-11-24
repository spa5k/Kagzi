# Kagzi V2 Architecture

This document provides a comprehensive overview of Kagzi's architecture, including V2 enhancements.

## Table of Contents
1. [System Overview](#system-overview)
2. [Component Architecture](#component-architecture)
3. [Data Flow](#data-flow)
4. [Database Schema](#database-schema)
5. [Worker Architecture](#worker-architecture)
6. [Retry Mechanism](#retry-mechanism)
7. [Parallel Execution](#parallel-execution)
8. [Workflow Versioning](#workflow-versioning)
9. [Health Monitoring](#health-monitoring)

## System Overview

Kagzi is a durable workflow engine that ensures workflows survive crashes, restarts, and long sleep periods. The system consists of three main components:

```
┌─────────────────────────────────────────────────────────────┐
│                     Your Application                        │
│                                                             │
│  ┌──────────────┐          ┌──────────────┐               │
│  │   Client     │          │   Worker     │               │
│  │              │          │              │               │
│  │ - Start      │          │ - Poll       │               │
│  │ - Query      │          │ - Execute    │               │
│  │ - Register   │          │ - Heartbeat  │               │
│  └──────┬───────┘          └──────┬───────┘               │
│         │                         │                        │
└─────────┼─────────────────────────┼────────────────────────┘
          │                         │
          ▼                         ▼
┌─────────────────────────────────────────────────────────────┐
│                      Kagzi SDK (Rust)                       │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ Client API                                          │   │
│  │ - Workflow registration & versioning                │   │
│  │ - Workflow execution                                │   │
│  │ - Worker creation & configuration                   │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ Workflow Execution                                  │   │
│  │ - WorkflowContext (step memoization)                │   │
│  │ - StepBuilder (retry policies)                      │   │
│  │ - ParallelExecutor (concurrent steps)               │   │
│  └─────────────────────────────────────────────────────┘   │
│                                                             │
│  ┌─────────────────────────────────────────────────────┐   │
│  │ Worker & Health                                     │   │
│  │ - WorkerBuilder (configuration)                     │   │
│  │ - HealthChecker (monitoring)                        │   │
│  │ - WorkflowRegistry (versioning)                     │   │
│  └─────────────────────────────────────────────────────┘   │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           ▼
┌─────────────────────────────────────────────────────────────┐
│                    PostgreSQL Database                      │
│                                                             │
│  Tables:                                                    │
│  • workflow_runs - Workflow execution state                 │
│  • step_runs - Step memoization & retry state               │
│  • workers - Worker registration & heartbeat                │
│  • worker_events - Worker lifecycle events                  │
│  • worker_leases - Workflow-to-worker assignment            │
└─────────────────────────────────────────────────────────────┘
```

## Component Architecture

### Core Components

```
Kagzi
├── kagzi-core                     (Database layer)
│   ├── Database                   - Connection pool
│   ├── Migrations                 - Schema management
│   ├── Models                     - Database models
│   │   ├── WorkflowRun
│   │   ├── StepRun
│   │   ├── Worker
│   │   └── WorkerLease
│   ├── Queries                    - Database operations
│   ├── RetryPolicy                - Retry configuration
│   └── ErrorKind                  - Error classification
│
├── kagzi                          (User-facing SDK)
│   ├── Client (Kagzi)             - Workflow management
│   │   ├── connect()
│   │   ├── register_workflow()
│   │   ├── register_workflow_version()
│   │   ├── start_workflow()
│   │   └── create_worker_builder()
│   │
│   ├── Context                    - Workflow execution
│   │   ├── WorkflowContext
│   │   │   ├── step()             - Basic step execution
│   │   │   ├── step_builder()     - Advanced step config
│   │   │   ├── sleep()            - Durable sleep
│   │   │   ├── parallel()         - Tuple parallel
│   │   │   ├── parallel_vec()     - Vec parallel
│   │   │   └── race()             - Racing parallel
│   │   │
│   │   └── StepBuilder
│   │       ├── retry_policy()     - Set retry config
│   │       ├── retry_predicate()  - Error classification
│   │       └── execute()          - Execute with retry
│   │
│   ├── Worker                     - Workflow execution
│   │   ├── Worker
│   │   ├── WorkerBuilder          - Configuration
│   │   └── WorkerConfig           - Settings
│   │
│   ├── Versioning                 - Version management
│   │   └── WorkflowRegistry
│   │       ├── register()         - Register version
│   │       ├── get_workflow()     - Lookup version
│   │       └── set_default_version()
│   │
│   ├── Parallel                   - Concurrent execution
│   │   ├── ParallelExecutor
│   │   ├── ParallelErrorStrategy
│   │   └── ParallelResult
│   │
│   └── Health                     - Monitoring
│       ├── HealthChecker
│       ├── HealthStatus
│       └── WorkerHealth
│
└── kagzi-cli                      (CLI tool)
    └── Command-line interface
```

## Data Flow

### Workflow Execution Flow

```
1. Client Starts Workflow
   │
   ├─→ Serialize input to JSON
   ├─→ Insert into workflow_runs table (status: PENDING)
   └─→ Return WorkflowHandle

2. Worker Polls for Workflows
   │
   ├─→ SELECT workflows WHERE status IN (PENDING, SLEEPING)
   ├─→ FOR UPDATE SKIP LOCKED (prevents conflicts)
   ├─→ Update status to RUNNING
   └─→ Create worker_lease

3. Worker Executes Workflow
   │
   ├─→ Load workflow function from registry
   ├─→ Deserialize input
   ├─→ Create WorkflowContext
   └─→ Execute workflow function
       │
       ├─→ For each step:
       │   │
       │   ├─→ Check step_runs for memoization
       │   │   ├─→ If found: return cached result
       │   │   └─→ If not: execute step
       │   │
       │   ├─→ Execute step logic
       │   │   ├─→ Success: store output in step_runs
       │   │   └─→ Failure: handle retry policy
       │   │       ├─→ Check if retryable
       │   │       ├─→ Calculate next retry time
       │   │       ├─→ Update step retry state
       │   │       └─→ Set workflow to SLEEPING
       │   │
       │   └─→ Continue to next step
       │
       └─→ Complete workflow
           ├─→ Store final output
           ├─→ Update status to COMPLETED
           └─→ Release worker_lease

4. Client Queries Result
   │
   ├─→ Poll workflow_runs for status
   └─→ Return output when COMPLETED
```

### Parallel Execution Flow

```
ParallelExecutor.execute_step()
│
├─→ 1. Check Memoization (Bulk Query)
│   │
│   ├─→ bulk_check_step_cache() for all step IDs
│   └─→ Return cached results immediately
│
├─→ 2. Execute Uncached Steps
│   │
│   ├─→ Spawn tokio tasks for each step
│   │   │
│   │   ├─→ Execute step logic
│   │   └─→ Store in step_runs with:
│   │       ├─→ parallel_group_id
│   │       ├─→ parent_step_id
│   │       └─→ status
│   │
│   └─→ Collect results with error strategy:
│       ├─→ FailFast: abort on first error
│       └─→ CollectAll: wait for all, collect errors
│
└─→ 3. Return Results
    │
    ├─→ Merge cached + newly computed results
    └─→ Return in original order
```

### Retry Mechanism Flow

```
StepBuilder.execute()
│
├─→ 1. Check Step State
│   │
│   ├─→ Query step_runs
│   │
│   ├─→ If COMPLETED: return cached result
│   │
│   ├─→ If FAILED with retry schedule:
│   │   ├─→ Check if retry time reached
│   │   ├─→ If yes: proceed to execution
│   │   └─→ If no: set workflow SLEEPING
│   │
│   └─→ If no record: proceed to execution
│
├─→ 2. Execute Step
│   │
│   └─→ Run step logic
│       ├─→ Success → Go to step 3
│       └─→ Failure → Go to step 4
│
├─→ 3. Handle Success
│   │
│   ├─→ Store output in step_runs
│   ├─→ Clear retry schedule
│   └─→ Return result
│
└─→ 4. Handle Failure
    │
    ├─→ Classify error (retryable?)
    │
    ├─→ Check retry policy
    │   ├─→ No policy: permanent failure
    │   ├─→ Max attempts reached: permanent failure
    │   └─→ Can retry: schedule next attempt
    │
    ├─→ Calculate next retry time
    │   ├─→ Exponential: delay × multiplier^attempt
    │   └─→ Fixed: constant delay
    │
    ├─→ Update step_runs with:
    │   ├─→ attempts (increment)
    │   ├─→ next_retry_at
    │   ├─→ retry_policy
    │   └─→ error
    │
    ├─→ Set workflow to SLEEPING
    │
    └─→ Return __RETRY__ signal
```

## Database Schema

### Core Tables

```sql
-- Workflow Runs
CREATE TABLE workflow_runs (
    id UUID PRIMARY KEY,
    workflow_name TEXT NOT NULL,
    workflow_version INTEGER,
    input JSONB NOT NULL,
    output JSONB,
    status TEXT NOT NULL,  -- PENDING, RUNNING, SLEEPING, COMPLETED, FAILED
    error JSONB,
    sleep_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- Step Runs (Memoization + Retry State)
CREATE TABLE step_runs (
    id UUID PRIMARY KEY,
    workflow_run_id UUID REFERENCES workflow_runs(id),
    step_id TEXT NOT NULL,
    input_hash TEXT,
    output JSONB,
    error JSONB,
    status TEXT NOT NULL,  -- PENDING, RUNNING, COMPLETED, FAILED

    -- Parallel execution tracking
    parent_step_id TEXT,
    parallel_group_id UUID,

    -- Retry state (V2)
    attempts INTEGER DEFAULT 0,
    next_retry_at TIMESTAMPTZ,
    retry_policy JSONB,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),

    UNIQUE(workflow_run_id, step_id)
);

-- Workers (V2)
CREATE TABLE workers (
    id UUID PRIMARY KEY,
    worker_name TEXT NOT NULL,
    hostname TEXT,
    process_id INTEGER,
    status TEXT NOT NULL,  -- RUNNING, SHUTTING_DOWN, STOPPED
    config JSONB,
    metadata JSONB,
    started_at TIMESTAMPTZ DEFAULT NOW(),
    last_heartbeat TIMESTAMPTZ DEFAULT NOW(),
    stopped_at TIMESTAMPTZ
);

-- Worker Events (V2)
CREATE TABLE worker_events (
    id UUID PRIMARY KEY,
    worker_id UUID REFERENCES workers(id),
    event_type TEXT NOT NULL,  -- STARTED, HEARTBEAT, STOPPING, STOPPED
    metadata JSONB,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Worker Leases
CREATE TABLE worker_leases (
    workflow_run_id UUID PRIMARY KEY REFERENCES workflow_runs(id),
    worker_id UUID REFERENCES workers(id),
    acquired_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL
);
```

### Indexes

```sql
-- Performance indexes
CREATE INDEX idx_workflow_runs_status ON workflow_runs(status);
CREATE INDEX idx_workflow_runs_sleep ON workflow_runs(sleep_until) WHERE status = 'SLEEPING';
CREATE INDEX idx_step_runs_workflow ON step_runs(workflow_run_id);
CREATE INDEX idx_step_runs_parallel ON step_runs(parallel_group_id) WHERE parallel_group_id IS NOT NULL;
CREATE INDEX idx_workers_status ON workers(status);
CREATE INDEX idx_workers_heartbeat ON workers(last_heartbeat);
CREATE INDEX idx_worker_leases_expires ON worker_leases(expires_at);
```

## Worker Architecture

### Worker Lifecycle

```
Worker Creation
│
├─→ 1. Initialize
│   ├─→ Parse WorkerConfig
│   ├─→ Generate worker_id
│   ├─→ Create semaphore (concurrency control)
│   └─→ Register in workers table
│
├─→ 2. Start
│   ├─→ Spawn heartbeat task
│   ├─→ Spawn polling task
│   └─→ Listen for shutdown signals
│
├─→ 3. Polling Loop
│   │
│   ├─→ Acquire semaphore permit
│   │   (blocks if max_concurrent_workflows reached)
│   │
│   ├─→ Query for available workflow
│   │   SELECT ... FOR UPDATE SKIP LOCKED
│   │
│   ├─→ If workflow found:
│   │   ├─→ Create worker_lease
│   │   ├─→ Spawn execution task
│   │   └─→ Release semaphore on completion
│   │
│   └─→ Sleep poll_interval_ms
│
├─→ 4. Heartbeat Loop
│   │
│   ├─→ Update workers.last_heartbeat
│   ├─→ Create HEARTBEAT event
│   └─→ Sleep heartbeat_interval_secs
│
└─→ 5. Shutdown
    │
    ├─→ Receive SIGTERM/SIGINT
    ├─→ Update status to SHUTTING_DOWN
    ├─→ Stop accepting new workflows
    ├─→ Wait for active workflows (with timeout)
    ├─→ Update status to STOPPED
    └─→ Exit
```

### Worker Concurrency Control

```
                        Worker
                          │
                          ▼
         ┌────────────────────────────────┐
         │   Semaphore                    │
         │   (max_concurrent_workflows)   │
         │                                │
         │   ┌─────┐ ┌─────┐ ┌─────┐    │
         │   │ ✓   │ │ ✓   │ │ ✓   │    │  Active
         │   └─────┘ └─────┘ └─────┘    │  Workflows
         │   ┌─────┐ ┌─────┐            │
         │   │     │ │     │  Available │
         │   └─────┘ └─────┘    Slots   │
         └────────────────────────────────┘
                          │
                          ▼
              Each permit represents
              one concurrent workflow
                          │
                          ▼
          ┌───────────────┴───────────────┐
          │                               │
          ▼                               ▼
    Workflow A                      Workflow B
    (holds permit)                  (holds permit)
          │                               │
          └───────────┬───────────────────┘
                      │
                      ▼
              On completion:
              release permit
```

## Retry Mechanism

### Retry Policy Types

```
1. Exponential Backoff
   │
   Delay = initial × multiplier^attempt
   │
   Example (initial=1s, multiplier=2):
   ├─→ Attempt 0: 1s
   ├─→ Attempt 1: 2s
   ├─→ Attempt 2: 4s
   ├─→ Attempt 3: 8s
   └─→ Attempt 4: 16s

   With jitter (±25%):
   ├─→ Attempt 0: 0.75s - 1.25s
   ├─→ Attempt 1: 1.5s - 2.5s
   └─→ ...

2. Fixed Interval
   │
   Delay = constant
   │
   Example (interval=5s):
   ├─→ Attempt 0: 5s
   ├─→ Attempt 1: 5s
   ├─→ Attempt 2: 5s
   └─→ ...

3. No Retry
   │
   └─→ Fail immediately
```

### Error Classification

```
ErrorKind
│
├─→ Retryable Errors
│   ├─→ NetworkError        (Connection timeout, etc.)
│   ├─→ Timeout             (Request timeout)
│   ├─→ DatabaseError       (Temporary DB issues)
│   ├─→ RateLimitExceeded   (API rate limits)
│   └─→ ServiceUnavailable  (503 errors)
│
└─→ Non-Retryable Errors
    ├─→ NotFound            (404 errors)
    ├─→ Unauthorized        (401/403 errors)
    ├─→ BadRequest          (400 errors)
    ├─→ ValidationError     (Invalid input)
    └─→ Unknown             (Unclassified errors)

Retry Predicates:
├─→ OnRetryableError  - Retry only retryable errors
├─→ OnAnyError        - Retry all errors
├─→ Never             - Never retry
└─→ OnErrorKind(...)  - Retry specific error kinds
```

## Parallel Execution

### Parallel Execution Model

```
WorkflowContext.parallel()
         │
         ├─→ Generate parallel_group_id
         │
         ├─→ Create ParallelExecutor
         │   ├─→ parallel_group_id
         │   ├─→ parent_step_id
         │   └─→ error_strategy
         │
         ├─→ Check Memoization (all steps at once)
         │   └─→ bulk_check_step_cache()
         │
         ├─→ Execute Uncached Steps
         │   │
         │   ├─→ Step 1 ─┐
         │   ├─→ Step 2 ─┼─→ tokio::spawn
         │   └─→ Step 3 ─┘
         │        │ │ │
         │        │ │ └─→ Parallel Execution
         │        │ │
         │        ▼ ▼ ▼
         │     ┌───────────┐
         │     │ Database  │
         │     │ step_runs │
         │     │           │
         │     │ • parallel_group_id = UUID
         │     │ • parent_step_id = "group"
         │     │ • status = COMPLETED/FAILED
         │     └───────────┘
         │
         └─→ Collect Results
             │
             ├─→ FailFast:
             │   └─→ Return on first error
             │
             └─→ CollectAll:
                 └─→ Wait for all, collect errors
```

### Memoization in Parallel Execution

```
Parallel Group: "fetch-user-data"
parallel_group_id: 550e8400-e29b-41d4-a716-446655440000

step_runs table:
┌─────────────┬────────────────┬─────────────────────────┐
│ step_id     │ parallel_group │ parent_step_id          │
├─────────────┼────────────────┼─────────────────────────┤
│ fetch-user  │ 550e8400...    │ fetch-user-data         │
│ fetch-posts │ 550e8400...    │ fetch-user-data         │
│ fetch-likes │ 550e8400...    │ fetch-user-data         │
└─────────────┴────────────────┴─────────────────────────┘

On workflow restart:
├─→ Query: SELECT * FROM step_runs
│          WHERE parallel_group_id = '550e8400...'
│
├─→ Returns all 3 steps in single query
│
└─→ Resume from cached results
    (no re-execution needed)
```

## Workflow Versioning

### Version Registry

```
WorkflowRegistry
│
├─→ workflows: HashMap<(name, version), WorkflowFn>
│   │
│   ├─→ ("process-order", 1) → process_order_v1
│   ├─→ ("process-order", 2) → process_order_v2
│   ├─→ ("process-order", 3) → process_order_v3
│   │
│   └─→ ("send-email", 1) → send_email_v1
│
└─→ default_versions: HashMap<name, version>
    │
    ├─→ "process-order" → 3
    └─→ "send-email" → 1

Workflow Execution:
│
├─→ start_workflow("process-order", input)
│   └─→ Uses version 3 (default)
│
├─→ start_workflow_version("process-order", 1, input)
│   └─→ Uses version 1 (explicit)
│
└─→ Running workflows continue on their version
```

### Version Deployment Strategy

```
Initial State:
├─→ v1 registered and running
│
Deploy v2:
├─→ Register v2 alongside v1
├─→ Test v2 with canary workflows
├─→ Set v2 as default
│   ├─→ New workflows use v2
│   └─→ Existing workflows continue on v1
│
Cleanup:
└─→ Wait for v1 workflows to complete
    └─→ Optionally unregister v1
```

## Health Monitoring

### Health Check System

```
HealthChecker
│
├─→ Worker Health Check
│   │
│   ├─→ Check heartbeat age
│   │   ├─→ < warning_secs: Healthy ✓
│   │   ├─→ < timeout_secs: Degraded ⚠
│   │   └─→ > timeout_secs: Unhealthy ✗
│   │
│   ├─→ Check worker status
│   │   ├─→ RUNNING: OK
│   │   ├─→ SHUTTING_DOWN: Degraded
│   │   └─→ STOPPED: Unhealthy
│   │
│   └─→ Check active workflows
│       └─→ Count from worker_leases
│
├─→ Database Health Check
│   │
│   └─→ Execute simple query
│       ├─→ Success: Healthy ✓
│       └─→ Failure: Unhealthy ✗
│
└─→ Comprehensive Check
    │
    ├─→ Check database
    ├─→ Check all workers
    └─→ Return aggregate status
```

### Health Status States

```
┌──────────────────────────────────────────────┐
│            Health Status                     │
├──────────────────────────────────────────────┤
│                                              │
│  Healthy ✓                                   │
│  ├─→ All systems operational                 │
│  ├─→ Recent heartbeats                       │
│  └─→ Database accessible                     │
│                                              │
│  Degraded ⚠                                  │
│  ├─→ Slow heartbeats                         │
│  ├─→ Some workers shutting down              │
│  └─→ Performance issues                      │
│                                              │
│  Unhealthy ✗                                 │
│  ├─→ Heartbeat timeout exceeded              │
│  ├─→ Workers stopped                         │
│  └─→ Database unreachable                    │
│                                              │
└──────────────────────────────────────────────┘
```

## Performance Characteristics

### Scalability

- **Horizontal Worker Scaling**: Add more workers to increase throughput
- **Workflow Isolation**: Each workflow runs independently
- **Parallel Execution**: Steps within a workflow can run concurrently
- **Database Connection Pooling**: Efficient resource utilization

### Durability Guarantees

- **At-Least-Once Execution**: Steps may execute multiple times on retry
- **Exactly-Once Result**: Memoization ensures only one result is stored
- **Crash Recovery**: Workflows resume from last completed step
- **Sleep Durability**: Sleep state persisted in database

### Latency Profile

```
Operation                Time Complexity    Latency
─────────────────────────────────────────────────────
Start Workflow           O(1)               1-5ms
Step Execution (cached)  O(1)               <1ms
Step Execution (new)     O(1) + work        varies
Parallel (n steps)       O(n) queries       ~same as 1
Worker Poll              O(1)               1-10ms
Heartbeat Update         O(1)               <1ms
Health Check             O(workers)         5-50ms
```

## Security Considerations

### Database Access

- Use connection pooling with connection limits
- Employ least-privilege database user
- Enable SSL for database connections
- Use prepared statements (SQLx handles this)

### Workflow Isolation

- Workflows cannot directly interfere with each other
- Worker leases prevent concurrent execution of same workflow
- Database transactions ensure atomicity

### Error Handling

- Errors are sanitized before storage
- Sensitive data should not be in error messages
- StepError provides structured error classification

## Best Practices

### Workflow Design

1. **Keep Steps Small**: Each step should be independently retryable
2. **Use Memoization**: Design for idempotency
3. **Avoid Side Effects in Steps**: Or make them idempotent
4. **Use Parallel Execution**: For independent operations
5. **Set Appropriate Retry Policies**: Not all errors should retry

### Worker Configuration

1. **Set Concurrency Limits**: Prevent resource exhaustion
2. **Configure Heartbeats**: Based on expected step duration
3. **Enable Graceful Shutdown**: Allow workflows to complete
4. **Monitor Health**: Set up alerting on health checks

### Database Maintenance

1. **Archive Completed Workflows**: Prevent table bloat
2. **Monitor Connection Pool**: Tune pool size as needed
3. **Index Optimization**: Add indexes for common queries
4. **Backup Strategy**: Regular backups of workflow state

## Conclusion

Kagzi V2 provides a robust, production-ready workflow engine with:
- Durable execution and memoization
- Automatic retries with exponential backoff
- Parallel step execution
- Workflow versioning
- Production-ready worker management
- Comprehensive health monitoring

This architecture ensures workflows are resilient, observable, and scalable.
