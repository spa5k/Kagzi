# Kagzi Server

A production-grade gRPC server for the Kagzi workflow orchestration system, built with Rust and PostgreSQL.

## Overview

Kagzi Server provides distributed workflow execution with:

- **Durable task queues** with PostgreSQL-backed persistence
- **Cron-based scheduling** with catch-up support
- **Idempotent workflows** guaranteed at-least-once execution
- **Retry policies** with exponential backoff
- **Sleep steps** for delayed execution
- **Worker pool management** with heartbeat monitoring
- **Graceful shutdown** with drain mode

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     gRPC Server                           │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │ Workflow     │  │ Workflow     │  │ Worker       │  │
│  │ Service      │  │ Schedule     │  │ Service      │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
│  ┌──────────────┐  ┌──────────────────────────────────┐  │
│  │ Admin       │  │   Background Tasks              │  │
│  │ Service     │  │  ┌─────────┐  ┌─────────────┐  │  │
│  └──────────────┘  │  │Scheduler│  │ Watchdog    │  │  │
│                    │  └─────────┘  └─────────────┘  │  │
└────────────────────┼──────────────────────────────────┼──┘
                     │                                  │
              ┌──────▼──────┐                  ┌──────▼──────┐
              │ PostgreSQL  │                  │ LISTEN/     │
              │ (Persistence│                  │ NOTIFY      │
              │  & Locking) │                  │  Channel    │
              └─────────────┘                  └─────────────┘
```

### Components

- **gRPC Services**: Expose CRUD operations for workflows, schedules, workers, and administrative queries
- **Scheduler**: Wakes sleeping workflows and fires cron schedules on interval (default: 5s)
- **Watchdog**: Processes retries, recovers orphaned workflows, marks stale workers (default: 1s)
- **PostgreSQL**: Stores all workflow state, implements optimistic locking, and provides pub/sub notifications

### Data Flow

1. **Workflow Creation**: Client → WorkflowService → PostgreSQL
2. **Task Distribution**: Workers poll via WorkerService → PostgreSQL LISTEN/NOTIFY → Workers notified
3. **Execution**: Worker claims workflow, executes, reports completion/failure
4. **Monitoring**: AdminService queries for health, worker status, step history
5. **Recovery**: Watchdog detects stale workers, recovers orphaned workflows

## gRPC Services

### WorkflowService

Manages workflow lifecycle and state queries.

#### Methods

##### StartWorkflow

Creates a new workflow or returns existing workflow if one with the same `external_id` and namespace exists.

**Fields:**

- `external_id` (string, required): User-provided identifier for idempotency
- `task_queue` (string, required): Queue for worker assignment
- `workflow_type` (string, required): Workflow type identifier
- `namespace_id` (string): Namespace (default: "default")
- `version` (string): Workflow version (default: "1")
- `input` (Payload): Initial workflow input data
- `retry_policy` (RetryPolicy): Retry configuration (max_attempts, intervals, backoff)

**Returns:**

- `run_id` (string): UUID of created/existing workflow
- `already_exists` (bool): Whether workflow already existed

**Idempotency:** Duplicate `external_id` + `namespace_id` returns same `run_id`

##### GetWorkflow

Retrieves workflow details by ID.

**Fields:**

- `run_id` (string, required): UUID of workflow
- `namespace_id` (string): Namespace (default: "default")

**Returns:** Complete workflow state including status, input/output, timestamps

##### ListWorkflows

Lists workflows with pagination and filtering.

**Fields:**

- `namespace_id` (string): Namespace (default: "default")
- `status_filter` (WorkflowStatus): Filter by status (PENDING, RUNNING, SLEEPING, COMPLETED, FAILED, CANCELLED)
- `page.page_size` (int32): Page size (1-100, default: 20)
- `page.page_token` (string): Base64-encoded cursor for pagination
- `page.include_total_count` (bool): Include total count in response

**Returns:** Paginated list of workflows with pagination metadata

##### CancelWorkflow

Cancels a workflow if in a cancellable state.

**Fields:**

- `run_id` (string, required): UUID of workflow
- `namespace_id` (string): Namespace (default: "default")

**Preconditions:** Workflow must be PENDING, RUNNING, or SLEEPING

### WorkflowScheduleService

Manages cron-based workflow scheduling with catch-up support.

#### Methods

##### CreateWorkflowSchedule

Creates a new schedule that fires workflows according to a cron expression.

**Fields:**

- `task_queue` (string, required): Queue for spawned workflows
- `workflow_type` (string, required): Workflow type to spawn
- `cron_expr` (string, required): Cron expression (e.g., "0 0 * * * *" for daily midnight)
- `namespace_id` (string): Namespace (default: "default")
- `enabled` (bool): Enable schedule (default: true)
- `max_catchup` (int32): Max missed fires to catch up (default: 100)
- `version` (string): Workflow version (default: "1")
- `input` (Payload): Input data for spawned workflows

**Behavior:**

- Calculates `next_fire_at` from cron expression
- Scheduler fires workflow when current time >= `next_fire_at`
- On server restart, catches up missed fires up to `max_catchup`

##### GetWorkflowSchedule

Retrieves schedule details by ID.

##### ListWorkflowSchedules

Lists schedules with pagination, optional task_queue filter.

##### UpdateWorkflowSchedule

Updates schedule configuration. Can modify cron, enabled state, max_catchup, input.

**Behavior:** Recalculates `next_fire_at` if cron expression changes

##### DeleteWorkflowSchedule

Permanently deletes a schedule.

### WorkerService

Worker lifecycle and task distribution.

#### Methods

##### Register

Registers a worker to receive tasks.

**Fields:**

- `workflow_types` (repeated string): Types this worker can execute
- `task_queue` (string): Queue to poll from
- `namespace_id` (string): Namespace (default: "default")
- `hostname` (string): Worker hostname
- `pid` (uint32): Worker process ID
- `version` (string): Worker version
- `max_concurrent` (int32): Max concurrent tasks (default: 10)
- `queue_concurrency_limit` (int32): Per-queue concurrency limit (max: 10000)
- `workflow_type_concurrency` (repeated WorkflowTypeConcurrency): Per-type limits (max: 10000 each)

**Returns:**

- `worker_id` (string): UUID of registered worker
- `heartbeat_interval_secs` (int32): Required heartbeat interval

##### Heartbeat

Updates worker status, maintains liveness, extends workflow locks.

**Fields:**

- `worker_id` (string, required)
- `active_count` (int32): Currently executing workflows
- `completed_delta` (int32): Workflow completions since last heartbeat
- `failed_delta` (int32): Workflow failures since last heartbeat

**Returns:**

- `accepted` (bool): Whether heartbeat was accepted
- `should_drain` (bool): Whether worker should drain and stop accepting work

**Behavior:**

- Extends locks for workflows held by this worker (30s extension)
- Returns `should_drain=true` if worker marked for draining

##### Deregister

Removes worker registration.

**Fields:**

- `worker_id` (string, required)
- `drain` (bool): If true, marks worker as draining; if false, immediately deregisters

**Behavior:**

- Drain: Worker stops accepting new work, completes current workflows
- Immediate: Worker marked offline, workflows recovered as orphans

##### PollTask

Long-poll for available work. Uses PostgreSQL LISTEN/NOTIFY for efficiency.

**Fields:**

- `worker_id` (string, required)
- `workflow_types` (repeated string): Subset of registered types to poll for
- `task_queue` (string): Queue to poll from
- `namespace_id` (string): Namespace (default: "default")

**Returns:**

- `run_id` (string): Workflow to execute, or empty if timeout
- `workflow_type` (string): Type of workflow
- `input` (Payload): Workflow input data

**Behavior:**

- Immediately returns available workflow if present
- Waits on PostgreSQL channel for new work notification (up to `poll_timeout_secs`)
- Uses jitter (0-500ms) on notification to prevent thundering herd
- Decrements worker active count on timeout

##### BeginStep

Starts a step within a workflow.

**Fields:**

- `run_id` (string, required)
- `step_id` (string, required): Step identifier
- `kind` (StepKind): STEP or SLEEP
- `input` (Payload): Step input data
- `retry_policy` (RetryPolicy): Step-specific retry policy (merges with workflow policy)

**Returns:**

- `step_id` (string): Echoed step ID
- `should_execute` (bool): Whether step should execute (idempotency check)
- `cached_output` (Payload): Output from previous execution if idempotent

**Behavior:**

- If step already completed with same input, returns `should_execute=false` with cached output
- For SLEEP steps, lazily completes immediately and returns `should_execute=false`

##### CompleteStep

Marks step as completed.

**Fields:**

- `run_id` (string, required)
- `step_id` (string, required)
- `output` (Payload): Step output data

**Returns:** Updated step state

##### FailStep

Marks step as failed, potentially scheduling retry.

**Fields:**

- `run_id` (string, required)
- `step_id` (string, required)
- `error` (ErrorDetail): Error details (code, message, non_retryable, retry_after_ms)

**Returns:**

- `scheduled_retry` (bool): Whether retry was scheduled
- `retry_at` (Timestamp): When retry will occur

**Behavior:**

- If `non_retryable=true`, step marked permanently failed
- If `retry_after_ms>0`, retry scheduled at specific time
- Otherwise, applies retry policy with exponential backoff

##### CompleteWorkflow

Marks workflow as completed.

**Fields:**

- `run_id` (string, required)
- `output` (Payload): Final workflow output

**Preconditions:** Workflow must not be in terminal state

**Behavior:** Decrements worker active count if workflow was locked by a worker

##### FailWorkflow

Marks workflow as failed.

**Fields:**

- `run_id` (string, required)
- `error` (ErrorDetail): Error details

**Behavior:** Decrements worker active count if workflow was locked by a worker

##### Sleep

Schedules a workflow to sleep for a duration.

**Fields:**

- `run_id` (string, required)
- `duration` (Duration): Sleep duration (max 30 days)

**Behavior:**

- Workflow marked SLEEPING
- Scheduler wakes workflow after duration expires

### AdminService

Administrative operations and health checks.

#### Methods

##### ListWorkers

Lists workers with pagination and filtering.

**Fields:**

- `namespace_id` (string): Namespace (default: "default")
- `task_queue` (string): Filter by task queue
- `status_filter` (WorkerStatus): Filter by status (ONLINE, DRAINING, OFFLINE)
- `page.page_size` (int32): Page size (1-100, default: 20)
- `page.page_token` (string): Cursor for pagination
- `page.include_total_count` (bool): Include total count

**Returns:** Paginated list of workers with metadata

##### GetWorker

Retrieves worker details by ID.

##### GetStep

Retrieves step details by ID.

##### ListSteps

Lists steps for a workflow run.

**Fields:**

- `run_id` (string, required): Workflow run ID
- `step_name` (string): Filter by step name
- `page.page_size` (int32): Page size (1-100, default: 50)
- `page.page_token` (string): Cursor for pagination

**Returns:** Paginated list of steps with attempt history

##### HealthCheck

Health check endpoint.

**Returns:**

- `status` (ServingStatus): SERVING if database healthy, NOT_SERVING otherwise
- `message` (string): Status description
- `timestamp` (Timestamp): Check timestamp

##### GetServerInfo

Returns server metadata.

**Returns:**

- `version` (string): Server version
- `api_version` (string): API version
- `supported_features` (repeated string): Enabled features
- `min_sdk_version` (string): Minimum compatible SDK version

## Scheduler

The scheduler runs as a background task at configurable intervals (default: 5 seconds).

### Responsibilities

1. **Wake Sleeping Workflows**
   - Transitions SLEEPING workflows back to RUNNING when sleep duration expires
   - Batch size: configurable via `scheduler.batch_size` (default: 100)

2. **Fire Cron Schedules**
   - Queries schedules where `next_fire_at <= now`
   - Creates workflows for each due schedule
   - Handles catch-up for missed fires (server restart, downtime)

### Idempotency

Scheduled workflows are idempotent using:

- `external_id` = `{schedule_id}:{fire_time_rfc3339}`

This ensures each fire creates a unique workflow while preventing duplicate fires at the same timestamp.

### Catch-Up Logic

For schedules with `max_catchup > 0`:

- Calculates missed fires from `last_fired_at` to `now`
- Limits to `max_catchup` fires per scheduler tick
- Also limited by `max_workflows_per_tick` (default: 1000)

Example: Schedule fires every hour, server down for 24 hours, `max_catchup=10` → 10 workflows created, 14 missed

### Configuration

See [Configuration](#configuration) for scheduler settings.

## Watchdog

The watchdog runs as a background task at configurable intervals (default: 1 second).

### Responsibilities

1. **Process Pending Retries**
   - Transitions steps from RETRY_PENDING to PENDING when retry time arrives
   - Allows workers to re-poll and retry failed steps

2. **Find Orphaned Workflows**
   - Detects workflows with `locked_by` pointing to non-existent or offline workers
   - Schedules retry with exponential backoff based on workflow retry policy
   - If retries exhausted, marks workflow as FAILED

3. **Mark Stale Workers**
   - Marks workers as OFFLINE if no heartbeat within `worker_stale_threshold_secs` (default: 30s)
   - Recovers workflows from offline workers (treats as orphaned)

### Recovery Logic

For orphaned workflows:

1. Check retry policy: `should_retry(attempts)`
2. If true and `should_retry(attempts + 1)` → schedule retry with backoff
3. If true but `!should_retry(attempts + 1)` → mark FAILED with "exhausted retries"
4. If false → mark FAILED with "no retries"

### Configuration

See [Configuration](#configuration) for watchdog settings.

## Configuration

Configuration is loaded from environment variables with the `KAGZI_` prefix.

### Environment Variables

| Variable                                     | Description                      | Default         |
| -------------------------------------------- | -------------------------------- | --------------- |
| `KAGZI_DB_URL`                               | PostgreSQL connection URL        | Required        |
| `KAGZI_SERVER_HOST`                          | gRPC server bind address         | `0.0.0.0`       |
| `KAGZI_SERVER_PORT`                          | gRPC server port                 | `50051`         |
| `KAGZI_SERVER_DB_MAX_CONNECTIONS`            | Database max connections         | `50`            |
| `KAGZI_SCHEDULER_INTERVAL_SECS`              | Scheduler tick interval          | `5`             |
| `KAGZI_SCHEDULER_BATCH_SIZE`                 | Max workflows per batch          | `100`           |
| `KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK`     | Max workflows per scheduler tick | `1000`          |
| `KAGZI_WATCHDOG_INTERVAL_SECS`               | Watchdog tick interval           | `1`             |
| `KAGZI_WATCHDOG_WORKER_STALE_THRESHOLD_SECS` | Time before worker marked stale  | `30`            |
| `KAGZI_WORKER_POLL_TIMEOUT_SECS`             | PollTask timeout                 | `60`            |
| `KAGZI_WORKER_HEARTBEAT_INTERVAL_SECS`       | Required heartbeat interval      | `10`            |
| `KAGZI_PAYLOAD_WARN_THRESHOLD_BYTES`         | Payload size warning threshold   | `1048576` (1MB) |
| `KAGZI_PAYLOAD_MAX_SIZE_BYTES`               | Payload max size                 | `2097152` (2MB) |

### Configuration File

The server can also load configuration from a config file (TOML, YAML, JSON, etc.) via the `config` crate, but environment variables take precedence.

### Database Connection Pool

The server uses a connection pool with configurable size:

- Minimum: 10 connections (internal)
- Maximum: `KAGZI_SERVER_DB_MAX_CONNECTIONS`
- Connections are shared across all services and background tasks

## Deployment

### Prerequisites

- PostgreSQL 14+ with LISTEN/NOTIFY support
- Rust 1.70+ (for building from source)

### Database Schema

Run migrations automatically on startup:

```bash
# Migrations are located in ../../migrations
# Server runs migrations before accepting connections
```

Key tables:

- `workflow_runs`: Workflow instances
- `workflow_payloads`: Workflow input/output (bytea)
- `step_runs`: Step execution attempts
- `workers`: Worker registrations
- `concurrency_configs`: Per-queue/type concurrency limits
- `workflow_schedules`: Cron schedules

### Docker Deployment

```dockerfile
FROM postgres:16-alpine
# ... database setup ...

FROM rust:1.80-slim
RUN cargo install --git https://github.com/kagzi/kagzi kagzi-server
ENV KAGZI_DB_URL=postgres://user:pass@db:5432/kagzi
CMD ["kagzi-server"]
```

### Docker Compose

```yaml
version: "3.8"
services:
  kagzi-server:
    image: kagzi-server:latest
    ports:
      - "50051:50051"
    environment:
      KAGZI_DB_URL: postgres://kagzi:password@db:5432/kagzi
      KAGZI_SCHEDULER_INTERVAL_SECS: 5
      KAGZI_WATCHDOG_INTERVAL_SECS: 1
    depends_on:
      - db

  db:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: kagzi
      POSTGRES_USER: kagzi
      POSTGRES_PASSWORD: password
    volumes:
      - postgres_data:/var/lib/postgresql/data

volumes:
  postgres_data:
```

### Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kagzi-server
spec:
  replicas: 1
  selector:
    matchLabels:
      app: kagzi-server
  template:
    metadata:
      labels:
        app: kagzi-server
    spec:
      containers:
        - name: kagzi-server
          image: kagzi-server:latest
          ports:
            - containerPort: 50051
          env:
            - name: KAGZI_DB_URL
              valueFrom:
                secretKeyRef:
                  name: kagzi-secrets
                  key: db-url
            - name: KAGZI_SCHEDULER_INTERVAL_SECS
              value: "5"
            - name: KAGZI_WATCHDOG_INTERVAL_SECS
              value: "1"
          livenessProbe:
            grpc:
              port: 50051
            initialDelaySeconds: 30
            periodSeconds: 10
          readinessProbe:
            grpc:
              port: 50051
            initialDelaySeconds: 5
            periodSeconds: 5
```

### Health Checks

The server exposes a gRPC health check:

```bash
grpcurl -plaintext localhost:50051 \
  kagzi.kagzi.AdminService/HealthCheck
```

Response:

```json
{
  "status": "SERVING",
  "message": "Service is healthy and serving requests",
  "timestamp": {
    "seconds": 1704067200,
    "nanos": 0
  }
}
```

## Operational Considerations

### Monitoring

#### Metrics to Monitor

- **Workflow throughput**: Workflows started/completed/failed per interval
- **Queue depth**: PENDING workflows per task queue
- **Worker pool**: Online/offline/draining workers, active_count trends
- **Retry behavior**: Step retry rate, average retry delays
- **Scheduler lag**: Time between `next_fire_at` and actual fire
- **Watchdog activity**: Orphaned workflow detection rate

#### Logging

The server uses structured logging with `tracing`:

- All requests include `correlation_id` and `trace_id`
- Log format: JSON (configurable)
- Key log levels:
  - `INFO`: Normal operations (workflow creation, worker registration)
  - `WARN`: Recoverable issues (orphaned workflow, stale worker)
  - `ERROR`: Unrecoverable errors (database failures)

### Performance Tuning

#### Database

- **Connection pool**: Match `KAGZI_SERVER_DB_MAX_CONNECTIONS` to expected concurrency
- **Indexes**: Ensure partial indexes on status, task_queue are present
- **Query performance**: Monitor `workflow_runs` table size, consider partitioning

#### Scheduler

- **Interval**: Decrease `KAGZI_SCHEDULER_INTERVAL_SECS` for lower latency (higher CPU)
- **Batch size**: Increase `KAGZI_SCHEDULER_BATCH_SIZE` for high-throughput schedules
- **Max workflows per tick**: Increase `KAGZI_SCHEDULER_MAX_WORKFLOWS_PER_TICK` for catch-up scenarios

#### Worker Polling

- **Poll timeout**: Adjust `KAGZI_WORKER_POLL_TIMEOUT_SECS` based on task frequency
- **Heartbeat interval**: Workers must heartbeat within `KAGZI_WORKER_HEARTBEAT_INTERVAL_SECS`

### Scaling

#### Horizontal Scaling

**Multiple server instances:**

- Supported for multiple gRPC server replicas (same database)
- **Do not run multiple schedulers** - only one scheduler instance per database
- **Do not run multiple watchdogs** - only one watchdog instance per database

**Deployment pattern:**

```
┌──────────────┐  ┌──────────────┐
│ Server A     │  │ Server B     │
│ (gRPC only)  │  │ (gRPC only)  │
└──────────────┘  └──────────────┘
       │                 │
       └────────┬────────┘
                │
┌───────────────▼────────────────┐
│  Server C                      │
│  (gRPC + Scheduler + Watchdog) │
└────────────────────────────────┘
                │
       ┌────────▼────────┐
       │  PostgreSQL    │
       └────────────────┘
```

#### Vertical Scaling

- Increase database connection pool size
- Add CPU cores for higher gRPC request throughput
- Add memory for larger connection pools

### Backup and Recovery

#### Database Backup

```bash
pg_dump -U kagzi -h localhost -d kagzi > kagzi_backup.sql
```

#### Recovery Considerations

- **Orphaned workflows**: After restore, watchdog detects and recovers workflows from workers that no longer exist
- **Schedule catch-up**: Scheduler catches up missed fires based on `max_catchup`
- **Worker re-registration**: Workers must re-register after server restart

### Graceful Shutdown

The server handles graceful shutdown via SIGINT/SIGTERM:

1. Cancels scheduler and watchdog tasks
2. Stops accepting new gRPC connections
3. Allows in-flight requests to complete
4. Workers detect drain and finish current workflows

Worker drain process:

1. Mark worker as DRAINING via Deregister
2. Worker stops accepting new work (PollTask returns precondition_failed)
3. Worker completes in-flight workflows
4. Worker calls Deregister again (or process exits)

### Troubleshooting

#### Workers Not Receiving Tasks

1. **Check worker status:**
   ```bash
   grpcurl -plaintext localhost:50051 \
     -d '{"namespace_id":"default","status_filter":1}' \
     kagzi.kagzi.AdminService/ListWorkers
   ```

2. **Verify workflow is PENDING:**
   ```bash
   grpcurl -plaintext localhost:50051 \
     -d '{"run_id":"<uuid>","namespace_id":"default"}' \
     kagzi.kagzi.WorkflowService/GetWorkflow
   ```

3. **Check queue and type match:**
   - Worker must be registered for the workflow's `task_queue`
   - Worker must have the workflow's `workflow_type` in its list

#### Orphaned Workflow Detection

If workers crash:

1. Watchdog marks worker OFFLINE after `worker_stale_threshold_secs`
2. Watchdog recovers workflows as orphans
3. Workflows rescheduled with retry policy backoff

#### Schedule Not Firing

1. **Check schedule is enabled:**
   ```bash
   grpcurl -plaintext localhost:50051 \
     -d '{"schedule_id":"<uuid>","namespace_id":"default"}' \
     kagzi.kagzi.WorkflowScheduleService/GetWorkflowSchedule
   ```

2. **Verify cron expression:**
   - Use cron expression validator
   - Check `next_fire_at` is in the past

3. **Check scheduler logs:**
   ```
   Scheduler woke up sleeping workflows: <count>
   ```

#### Database Connection Issues

1. **Verify connection pool:**
   - Check `KAGZI_SERVER_DB_MAX_CONNECTIONS` is sufficient
   - Monitor PostgreSQL `max_connections`

2. **Check database health:**
   ```bash
   grpcurl -plaintext localhost:50051 \
     kagzi.kagzi.AdminService/HealthCheck
   ```

3. **Review database logs:**
   - Check for deadlocks or connection exhaustion

## Security Considerations

- **Authentication**: Not implemented in core server (use reverse proxy for auth)
- **Authorization**: Not implemented (all clients have full access)
- **TLS**: Enable TLS for production deployments (configure gRPC server)
- **Input validation**: All inputs validated, payload size limited
- **SQL injection**: Uses parameterized queries via sqlx
- **Secrets management**: Store database credentials in secret management (e.g., HashiCorp Vault)

## Development

### Building

```bash
cargo build -p kagzi-server
```

### Running Locally

```bash
export KAGZI_DB_URL="postgres://kagzi:password@localhost:5432/kagzi"
cargo run -p kagzi-server --bin kagzi-server
```

### Testing

```bash
# Unit tests
cargo test -p kagzi-server --lib

# Integration tests (requires Docker)
KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests -- --test-threads=1
```

### Linting

```bash
cargo clippy -p kagzi-server --all-targets --all-features -- -D warnings
```

## License

[License information from workspace]

## Contributing

[Contributing guidelines from workspace]
