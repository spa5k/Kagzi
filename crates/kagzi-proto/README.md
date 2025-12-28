# Kagzi Protocol Buffers

This crate contains the Protocol Buffer definitions and generated gRPC code for the Kagzi workflow engine. It provides type-safe, versioned APIs for workflow orchestration, worker communication, and administrative operations.

## Overview

The `kagzi-proto` crate:

- Defines all gRPC service interfaces via Protocol Buffers
- Generates idiomatic Rust client and server code using `tonic` and `prost`
- Provides a file descriptor set for gRPC reflection
- Supports backward compatibility through versioned package paths (`kagzi.v1`)

## Table of Contents

- [gRPC Services](#grpc-services)
  - [WorkflowService](#workflowservice)
  - [WorkerService](#workerservice)
  - [AdminService](#adminservice)
  - [WorkflowScheduleService](#workflowscheduleservice)
- [Message Types](#message-types)
- [Common Types](#common-types)
- [Enums](#enums)
- [File Descriptor Set](#file-descriptor-set)
- [Build Process](#build-process)
- [Regenerating Code](#regenerating-code)
- [Usage Examples](#usage-examples)

## gRPC Services

### WorkflowService

Manages workflow lifecycle operations including starting, querying, and cancelling workflows.

**Methods:**

| Method           | Request                 | Response                | Description                                                  |
| ---------------- | ----------------------- | ----------------------- | ------------------------------------------------------------ |
| `StartWorkflow`  | `StartWorkflowRequest`  | `StartWorkflowResponse` | Start a new workflow execution with optional idempotency key |
| `GetWorkflow`    | `GetWorkflowRequest`    | `GetWorkflowResponse`   | Retrieve workflow details by run_id                          |
| `ListWorkflows`  | `ListWorkflowsRequest`  | `ListWorkflowsResponse` | List workflows with optional status filtering and pagination |
| `CancelWorkflow` | `CancelWorkflowRequest` | `google.protobuf.Empty` | Cancel a running workflow                                    |

**Example:**

```rust
use kagzi_proto::kagzi::v1::{workflow_service_client::WorkflowServiceClient, StartWorkflowRequest};

let mut client = WorkflowServiceClient::connect("http://localhost:50051").await?;

let request = StartWorkflowRequest {
    external_id: Some("unique-idempotency-key".to_string()),
    namespace_id: "default".to_string(),
    task_queue: "main".to_string(),
    workflow_type: "process-order".to_string(),
    input: Some(Payload {
        data: serde_json::to_vec(&input_data)?,
        metadata: Default::default(),
    }),
    ..Default::default()
};

let response = client.start_workflow(request).await?;
```

### WorkerService

Handles worker registration, lifecycle management, and task execution.

**Methods:**

| Category      | Method             | Request                   | Response                   | Description                                                  |
| ------------- | ------------------ | ------------------------- | -------------------------- | ------------------------------------------------------------ |
| **Lifecycle** | `Register`         | `RegisterRequest`         | `RegisterResponse`         | Register worker and receive worker_id and heartbeat interval |
|               | `Heartbeat`        | `HeartbeatRequest`        | `HeartbeatResponse`        | Send heartbeat with activity stats                           |
|               | `Deregister`       | `DeregisterRequest`       | `google.protobuf.Empty`    | Unregister worker (optionally drain)                         |
| **Execution** | `PollTask`         | `PollTaskRequest`         | `PollTaskResponse`         | Poll for next available task                                 |
|               | `BeginStep`        | `BeginStepRequest`        | `BeginStepResponse`        | Begin step execution, receive step_id and cached output      |
|               | `CompleteStep`     | `CompleteStepRequest`     | `CompleteStepResponse`     | Complete a step with output                                  |
|               | `FailStep`         | `FailStepRequest`         | `FailStepResponse`         | Report step failure with optional retry scheduling           |
|               | `CompleteWorkflow` | `CompleteWorkflowRequest` | `CompleteWorkflowResponse` | Complete workflow execution                                  |
|               | `FailWorkflow`     | `FailWorkflowRequest`     | `FailWorkflowResponse`     | Report workflow failure                                      |
|               | `Sleep`            | `SleepRequest`            | `google.protobuf.Empty`    | Put workflow to sleep for duration                           |

**Example:**

```rust
use kagzi_proto::kagzi::v1::{
    worker_service_client::WorkerServiceClient,
    RegisterRequest, PollTaskRequest, BeginStepRequest, CompleteStepRequest,
    step_kind::Kind as StepKindEnum, step_kind,
};

// Register worker
let mut client = WorkerServiceClient::connect("http://localhost:50051").await?;
let register_response = client.register(RegisterRequest {
    namespace_id: "default".to_string(),
    task_queue: "main".to_string(),
    workflow_types: vec!["process-order".to_string()],
    hostname: "worker-1".to_string(),
    pid: std::process::id() as i32,
    version: "1.0.0".to_string(),
    max_concurrent: 10,
    ..Default::default()
}).await?;
let worker_id = register_response.into_inner().worker_id;

// Poll for tasks
loop {
    let task = client.poll_task(PollTaskRequest {
        worker_id: worker_id.clone(),
        namespace_id: "default".to_string(),
        task_queue: "main".to_string(),
        workflow_types: vec!["process-order".to_string()],
    }).await?;

    // Begin step
    let begin_response = client.begin_step(BeginStepRequest {
        run_id: task.run_id,
        step_name: "validate".to_string(),
        kind: Some(step_kind::Kind::Function(Empty {})),
        input: task.input,
        ..Default::default()
    }).await?;

    let step_id = begin_response.into_inner().step_id;

    // Complete step
    let result = execute_step(&step_id).await?;
    client.complete_step(CompleteStepRequest {
        run_id: task.run_id,
        step_id,
        output: Some(Payload {
            data: serde_json::to_vec(&result)?,
            metadata: Default::default(),
        }),
    }).await?;
}
```

### AdminService

Provides administrative capabilities for monitoring workers, steps, and server health.

**Methods:**

| Category      | Method          | Request                | Response                | Description                          |
| ------------- | --------------- | ---------------------- | ----------------------- | ------------------------------------ |
| **Discovery** | `ListWorkers`   | `ListWorkersRequest`   | `ListWorkersResponse`   | List workers with optional filtering |
|               | `GetWorker`     | `GetWorkerRequest`     | `GetWorkerResponse`     | Get detailed worker information      |
|               | `GetStep`       | `GetStepRequest`       | `GetStepResponse`       | Retrieve step details                |
|               | `ListSteps`     | `ListStepsRequest`     | `ListStepsResponse`     | List steps for a workflow            |
| **Health**    | `HealthCheck`   | `HealthCheckRequest`   | `HealthCheckResponse`   | Check service health status          |
|               | `GetServerInfo` | `GetServerInfoRequest` | `GetServerInfoResponse` | Get server version and capabilities  |

**Example:**

```rust
use kagzi_proto::kagzi::v1::{admin_service_client::AdminServiceClient, ListWorkersRequest};

let mut client = AdminServiceClient::connect("http://localhost:50051").await?;

let response = client.list_workers(ListWorkersRequest {
    namespace_id: "default".to_string(),
    task_queue: Some("main".to_string()),
    status_filter: Some(WorkerStatus::Online as i32),
    page: Some(PageRequest {
        page_size: 100,
        ..Default::default()
    }),
}).await?;

for worker in response.into_inner().workers {
    println!("Worker {} - {} active tasks", worker.worker_id, worker.active_count);
}
```

### WorkflowScheduleService

Manages scheduled workflow executions using cron expressions.

**Methods:**

| Method                   | Request                         | Response                         | Description                     |
| ------------------------ | ------------------------------- | -------------------------------- | ------------------------------- |
| `CreateWorkflowSchedule` | `CreateWorkflowScheduleRequest` | `CreateWorkflowScheduleResponse` | Create a new scheduled workflow |
| `GetWorkflowSchedule`    | `GetWorkflowScheduleRequest`    | `GetWorkflowScheduleResponse`    | Retrieve schedule details       |
| `ListWorkflowSchedules`  | `ListWorkflowSchedulesRequest`  | `ListWorkflowSchedulesResponse`  | List schedules with pagination  |
| `UpdateWorkflowSchedule` | `UpdateWorkflowScheduleRequest` | `UpdateWorkflowScheduleResponse` | Update schedule configuration   |
| `DeleteWorkflowSchedule` | `DeleteWorkflowScheduleRequest` | `DeleteWorkflowScheduleResponse` | Delete a schedule               |

**Example:**

```rust
use kagzi_proto::kagzi::v1::{
    workflow_schedule_service_client::WorkflowScheduleServiceClient,
    CreateWorkflowScheduleRequest,
};

let mut client = WorkflowScheduleServiceClient::connect("http://localhost:50051").await?;

let response = client.create_workflow_schedule(CreateWorkflowScheduleRequest {
    namespace_id: "default".to_string(),
    task_queue: "main".to_string(),
    workflow_type: "daily-report".to_string(),
    cron_expr: "0 9 * * *".to_string(), // 9 AM daily
    input: Some(Payload {
        data: serde_json::to_vec(&report_config)?,
        metadata: Default::default(),
    }),
    enabled: Some(true),
    max_catchup: Some(5),
    ..Default::default()
}).await?;

let schedule_id = response.into_inner().schedule.schedule_id;
```

## Message Types

### Workflow

Represents a workflow execution instance.

**Fields:**

| Field            | Type             | Description                              |
| ---------------- | ---------------- | ---------------------------------------- |
| `run_id`         | `string`         | Unique identifier for this workflow run  |
| `external_id`    | `string`         | User-provided idempotency key            |
| `namespace_id`   | `string`         | Namespace for isolation                  |
| `task_queue`     | `string`         | Queue for task distribution              |
| `workflow_type`  | `string`         | Type name of the workflow                |
| `status`         | `WorkflowStatus` | Current execution status                 |
| `input`          | `Payload`        | Initial workflow input                   |
| `output`         | `Payload`        | Final workflow output (if completed)     |
| `error`          | `ErrorDetail`    | Error details (if failed)                |
| `attempts`       | `int32`          | Number of retry attempts                 |
| `created_at`     | `Timestamp`      | Creation timestamp                       |
| `started_at`     | `Timestamp`      | Execution start timestamp                |
| `finished_at`    | `Timestamp`      | Completion timestamp                     |
| `wake_up_at`     | `Timestamp`      | Next wake-up time for sleeping workflows |
| `deadline_at`    | `Timestamp`      | Execution deadline                       |
| `worker_id`      | `string`         | ID of worker processing this workflow    |
| `version`        | `string`         | Workflow version identifier              |
| `parent_step_id` | `string`         | Parent step for child workflows          |

### Step

Represents a step within a workflow execution.

**Fields:**

| Field            | Type          | Description                           |
| ---------------- | ------------- | ------------------------------------- |
| `step_id`        | `string`      | Unique step identifier                |
| `run_id`         | `string`      | Parent workflow run ID                |
| `namespace_id`   | `string`      | Namespace for isolation               |
| `name`           | `string`      | Step name                             |
| `kind`           | `StepKind`    | Type of step (function, sleep)        |
| `status`         | `StepStatus`  | Current execution status              |
| `attempt_number` | `int32`       | Current attempt number                |
| `input`          | `Payload`     | Step input data                       |
| `output`         | `Payload`     | Step output data (if completed)       |
| `error`          | `ErrorDetail` | Error details (if failed)             |
| `created_at`     | `Timestamp`   | Step creation timestamp               |
| `started_at`     | `Timestamp`   | Execution start timestamp             |
| `finished_at`    | `Timestamp`   | Completion timestamp                  |
| `child_run_id`   | `string`      | Child workflow run ID (if applicable) |

### Worker

Represents a registered worker instance.

**Fields:**

| Field                       | Type                               | Description                            |
| --------------------------- | ---------------------------------- | -------------------------------------- |
| `worker_id`                 | `string`                           | Unique worker identifier               |
| `namespace_id`              | `string`                           | Namespace for isolation                |
| `task_queue`                | `string`                           | Assigned task queue                    |
| `status`                    | `WorkerStatus`                     | Current worker status                  |
| `hostname`                  | `string`                           | Worker hostname                        |
| `pid`                       | `int32`                            | Process ID                             |
| `version`                   | `string`                           | Worker software version                |
| `workflow_types`            | `repeated string`                  | Capable workflow types                 |
| `max_concurrent`            | `int32`                            | Maximum concurrent tasks               |
| `active_count`              | `int32`                            | Currently executing tasks              |
| `total_completed`           | `int64`                            | Lifetime completed tasks               |
| `total_failed`              | `int64`                            | Lifetime failed tasks                  |
| `registered_at`             | `Timestamp`                        | Registration timestamp                 |
| `last_heartbeat_at`         | `Timestamp`                        | Last heartbeat timestamp               |
| `labels`                    | `map<string, string>`              | Worker metadata labels                 |
| `queue_concurrency_limit`   | `int32`                            | Optional queue-level concurrency limit |
| `workflow_type_concurrency` | `repeated WorkflowTypeConcurrency` | Per-workflow-type limits               |

### WorkflowSchedule

Represents a scheduled workflow execution.

**Fields:**

| Field           | Type        | Description                          |
| --------------- | ----------- | ------------------------------------ |
| `schedule_id`   | `string`    | Unique schedule identifier           |
| `namespace_id`  | `string`    | Namespace for isolation              |
| `task_queue`    | `string`    | Target task queue                    |
| `workflow_type` | `string`    | Type of workflow to execute          |
| `cron_expr`     | `string`    | Cron expression for scheduling       |
| `input`         | `Payload`   | Input data for each execution        |
| `enabled`       | `bool`      | Whether schedule is active           |
| `max_catchup`   | `int32`     | Maximum missed executions to recover |
| `next_fire_at`  | `Timestamp` | Next scheduled execution time        |
| `last_fired_at` | `Timestamp` | Last execution timestamp             |
| `version`       | `string`    | Workflow version to use              |
| `created_at`    | `Timestamp` | Schedule creation timestamp          |
| `updated_at`    | `Timestamp` | Last update timestamp                |

## Common Types

### Payload

Flexible container for serialized data with metadata.

**Fields:**

| Field      | Type                  | Description                                            |
| ---------- | --------------------- | ------------------------------------------------------ |
| `data`     | `bytes`               | Serialized data (JSON, MessagePack, etc.)              |
| `metadata` | `map<string, string>` | Encoding info (e.g., `content-type: application/json`) |

**Example:**

```rust
use kagzi_proto::kagzi::v1::Payload;

let payload = Payload {
    data: serde_json::to_vec(&my_data)?,
    metadata: {
        let mut map = HashMap::new();
        map.insert("content-type".to_string(), "application/json".to_string());
        map.insert("encoding".to_string(), "utf-8".to_string());
        map
    },
};
```

### RetryPolicy

Configuration for retry behavior.

**Fields:**

| Field                  | Type              | Description                            |
| ---------------------- | ----------------- | -------------------------------------- |
| `maximum_attempts`     | `int32`           | Maximum retry attempts                 |
| `initial_interval_ms`  | `int64`           | Initial retry interval in milliseconds |
| `backoff_coefficient`  | `double`          | Exponential backoff multiplier         |
| `maximum_interval_ms`  | `int64`           | Maximum retry interval in milliseconds |
| `non_retryable_errors` | `repeated string` | Error types that should not be retried |

### Pagination

**PageRequest:**

| Field                 | Type     | Description                         |
| --------------------- | -------- | ----------------------------------- |
| `page_size`           | `int32`  | Number of results per page          |
| `page_token`          | `string` | Pagination token from previous page |
| `include_total_count` | `bool`   | Whether to include total count      |

**PageInfo:**

| Field             | Type     | Description                            |
| ----------------- | -------- | -------------------------------------- |
| `next_page_token` | `string` | Token for next page (empty if last)    |
| `has_more`        | `bool`   | Whether more pages exist               |
| `total_count`     | `int64`  | Total number of results (if requested) |

### ErrorDetail

Detailed error information.

**Fields:**

| Field            | Type                  | Description                         |
| ---------------- | --------------------- | ----------------------------------- |
| `code`           | `ErrorCode`           | Error category                      |
| `message`        | `string`              | Human-readable error message        |
| `non_retryable`  | `bool`                | Whether error should not be retried |
| `retry_after_ms` | `int64`               | Suggested delay before retry        |
| `subject`        | `string`              | Entity that caused the error        |
| `subject_id`     | `string`              | ID of the entity                    |
| `metadata`       | `map<string, string>` | Additional error context            |

## Enums

### WorkflowStatus

| Value       | Description                       |
| ----------- | --------------------------------- |
| `PENDING`   | Workflow created, not yet started |
| `RUNNING`   | Workflow actively executing       |
| `SLEEPING`  | Workflow waiting for wake-up      |
| `COMPLETED` | Workflow finished successfully    |
| `FAILED`    | Workflow terminated with error    |
| `CANCELLED` | Workflow was cancelled            |

### WorkerStatus

| Value      | Description                                            |
| ---------- | ------------------------------------------------------ |
| `ONLINE`   | Worker accepting tasks                                 |
| `DRAINING` | Worker completing active tasks, not accepting new ones |
| `OFFLINE`  | Worker unregistered or not responding                  |

### StepKind

| Value      | Description              |
| ---------- | ------------------------ |
| `FUNCTION` | Executable function step |
| `SLEEP`    | Delay/sleep step         |

### StepStatus

| Value       | Description                |
| ----------- | -------------------------- |
| `PENDING`   | Step awaiting execution    |
| `RUNNING`   | Step currently executing   |
| `COMPLETED` | Step finished successfully |
| `FAILED`    | Step terminated with error |

### ErrorCode

| Value                 | Description                           |
| --------------------- | ------------------------------------- |
| `NOT_FOUND`           | Resource does not exist               |
| `INVALID_ARGUMENT`    | Invalid request parameters            |
| `PRECONDITION_FAILED` | Precondition not met                  |
| `CONFLICT`            | Resource conflict (e.g., idempotency) |
| `UNAUTHORIZED`        | Authentication/authorization failed   |
| `UNAVAILABLE`         | Service temporarily unavailable       |
| `INTERNAL`            | Internal server error                 |
| `TIMEOUT`             | Operation timed out                   |

### ServingStatus

| Value         | Description                             |
| ------------- | --------------------------------------- |
| `SERVING`     | Service is healthy and serving requests |
| `NOT_SERVING` | Service is not serving requests         |

## File Descriptor Set

The crate includes a compiled file descriptor set at `src/descriptor.bin`, exposed via:

```rust
use kagzi_proto::FILE_DESCRIPTOR_SET;

// FILE_DESCRIPTOR_SET: &[u8] - Binary file descriptor set
```

This enables:

- **gRPC Reflection:** Server can expose schema information dynamically
- **Dynamic Clients:** Clients can introspect service definitions at runtime
- **Language Interoperability:** Non-Rust clients can regenerate bindings

To enable reflection in your server:

```rust
use kagzi_proto::FILE_DESCRIPTOR_SET;
use tonic_reflection::server::{ServerReflection, ServerReflectionServer};

Server::builder()
    .add_service(WorkflowServiceServer::new(workflow_impl))
    .add_service(ServerReflectionServer::new(
        ServerReflection::new(vec![FILE_DESCRIPTOR_SET.to_vec()])
    ))
    .serve(addr)
    .await?;
```

## Build Process

### Build Configuration

The `build.rs` script uses `tonic_prost_build` to compile Protocol Buffer files:

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .file_descriptor_set_path("src/descriptor.bin")
        .compile_protos(
            &[
                "../../proto/common.proto",
                "../../proto/workflow.proto",
                "../../proto/worker.proto",
                "../../proto/admin.proto",
                "../../proto/workflow_schedule.proto",
            ],
            &["../../proto"],
        )?;
    Ok(())
}
```

### Generated Output

The build process generates:

1. **Message Types:** Rust structs for all Protocol Buffer messages
2. **Service Traits:** gRPC service traits with request/response types
3. **Server Macros:** `#[tonic::async_trait]` traits for server implementation
4. **Client Types:** Client stubs for service invocation
5. **File Descriptor Set:** Binary descriptor set at `src/descriptor.bin`

### Generated Structure

All generated code is exposed under `kagzi::v1`:

```rust
use kagzi_proto::kagzi::v1::{
    // Services
    workflow_service_server::WorkflowServiceServer,
    workflow_service_client::WorkflowServiceClient,

    // Messages
    Workflow, Step, Worker, Payload,

    // Enums
    WorkflowStatus, StepStatus, WorkerStatus,

    // ... etc
};
```

## Regenerating Code

When Protocol Buffer files are modified, regenerate the Rust code:

```bash
# Using just (recommended)
just build-proto

# Or directly
cd crates/kagzi-proto
cargo clean && cargo build
```

**What happens:**

1. `cargo build` triggers `build.rs`
2. `tonic_prost_build` compiles all `.proto` files
3. Generated code is written to `target/debug/build/kagzi-proto-*/out/`
4. File descriptor set is written to `src/descriptor.bin`
5. New code is available for import

**Note:** After regenerating, run:

```bash
cargo fmt --all   # Format generated code
just lint         # Verify no new warnings
```

## Usage Examples

### Server Implementation

```rust
use kagzi_proto::kagzi::v1::{
    workflow_service_server::{WorkflowService, WorkflowServiceServer},
    StartWorkflowRequest, StartWorkflowResponse,
};
use tonic::{Request, Response, Status};

#[derive(Debug, Default)]
pub struct WorkflowServiceImpl {
    // Add state here (database connection, etc.)
}

#[tonic::async_trait]
impl WorkflowService for WorkflowServiceImpl {
    async fn start_workflow(
        &self,
        request: Request<StartWorkflowRequest>,
    ) -> Result<Response<StartWorkflowResponse>, Status> {
        let req = request.into_inner();

        // Validate input
        if req.workflow_type.is_empty() {
            return Err(Status::invalid_argument("workflow_type is required"));
        }

        // Check idempotency
        if let Some(external_id) = &req.external_id {
            if let Some(existing) = self.find_by_external_id(external_id).await? {
                return Ok(Response::new(StartWorkflowResponse {
                    run_id: existing.run_id,
                    already_exists: true,
                }));
            }
        }

        // Create workflow
        let run_id = self.create_workflow(&req).await?;

        Ok(Response::new(StartWorkflowResponse {
            run_id,
            already_exists: false,
        }))
    }

    // Implement other methods...
}

// Start server
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:50051".parse()?;

    let workflow_service = WorkflowServiceImpl::default();

    Server::builder()
        .add_service(WorkflowServiceServer::new(workflow_service))
        .serve(addr)
        .await?;

    Ok(())
}
```

### Client Implementation

```rust
use kagzi_proto::kagzi::v1::{
    workflow_service_client::WorkflowServiceClient,
    StartWorkflowRequest, Payload,
};
use tonic::transport::Channel;

struct KagziClient {
    workflow: WorkflowServiceClient<Channel>,
}

impl KagziClient {
    pub async fn connect(addr: String) -> Result<Self, tonic::transport::Error> {
        let channel = Channel::from_shared(addr)?.connect().await?;
        Ok(Self {
            workflow: WorkflowServiceClient::new(channel),
        })
    }

    pub async fn start_workflow<T: serde::Serialize>(
        &mut self,
        namespace_id: &str,
        task_queue: &str,
        workflow_type: &str,
        input: T,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let request = StartWorkflowRequest {
            external_id: None,
            namespace_id: namespace_id.to_string(),
            task_queue: task_queue.to_string(),
            workflow_type: workflow_type.to_string(),
            input: Some(Payload {
                data: serde_json::to_vec(&input)?,
                metadata: Default::default(),
            }),
            ..Default::default()
        };

        let response = self.workflow.start_workflow(request).await?;
        Ok(response.into_inner().run_id)
    }

    pub async fn get_workflow(
        &mut self,
        run_id: &str,
        namespace_id: &str,
    ) -> Result<Option<Workflow>, Box<dyn std::error::Error>> {
        let request = GetWorkflowRequest {
            run_id: run_id.to_string(),
            namespace_id: namespace_id.to_string(),
        };

        let response = self.workflow.get_workflow(request).await?;
        Ok(Some(response.into_inner().workflow))
    }
}
```

### Worker with Automatic Reconnection

```rust
use kagzi_proto::kagzi::v1::{
    worker_service_client::WorkerServiceClient,
    RegisterRequest, PollTaskRequest, BeginStepRequest,
    CompleteStepRequest, Payload,
};
use tonic::transport::Channel;

async fn run_worker(server_addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = WorkerServiceClient::connect(server_addr).await?;

    // Register worker
    let register_response = client.register(RegisterRequest {
        namespace_id: "default".to_string(),
        task_queue: "main".to_string(),
        workflow_types: vec!["process-order".to_string()],
        hostname: hostname::get()?.to_string_lossy().to_string(),
        pid: std::process::id() as i32,
        version: env!("CARGO_PKG_VERSION").to_string(),
        max_concurrent: 10,
        ..Default::default()
    }).await?;

    let worker_id = register_response.into_inner().worker_id;
    let heartbeat_interval = Duration::from_secs(10);

    // Spawn heartbeat task
    let heartbeat_client = WorkerServiceClient::connect(server_addr).await?;
    let worker_id_clone = worker_id.clone();
    tokio::spawn(async move {
        let mut client = heartbeat_client;
        loop {
            tokio::time::sleep(heartbeat_interval).await;
            let _ = client.heartbeat(HeartbeatRequest {
                worker_id: worker_id_clone.clone(),
                active_count: 0,
                completed_delta: 0,
                failed_delta: 0,
            }).await;
        }
    });

    // Main task loop
    loop {
        match client.poll_task(PollTaskRequest {
            worker_id: worker_id.clone(),
            namespace_id: "default".to_string(),
            task_queue: "main".to_string(),
            workflow_types: vec!["process-order".to_string()],
        }).await {
            Ok(response) => {
                let task = response.into_inner();
                if let Some(input) = task.input {
                    // Begin step
                    let begin_response = client.begin_step(BeginStepRequest {
                        run_id: task.run_id.clone(),
                        step_name: "process".to_string(),
                        kind: Some(step_kind::Kind::Function(Empty {})),
                        input: Some(input.clone()),
                        ..Default::default()
                    }).await?;

                    let step_id = begin_response.into_inner().step_id;

                    // Execute step
                    let result = process_workflow(&task.workflow_type, &input)?;

                    // Complete step
                    client.complete_step(CompleteStepRequest {
                        run_id: task.run_id,
                        step_id,
                        output: Some(Payload {
                            data: serde_json::to_vec(&result)?,
                            metadata: Default::default(),
                        }),
                    }).await?;
                }
            }
            Err(e) => {
                eprintln!("Poll error: {}, reconnecting...", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                client = WorkerServiceClient::connect(server_addr).await?;
            }
        }
    }
}
```

## Development

### Adding New Services

1. Create `.proto` file in `proto/` directory
2. Define service and messages using existing patterns
3. Add proto file to `build.rs` compile list
4. Run `just build-proto` to regenerate code
5. Update `src/lib.rs` if needed (usually automatic)

### Protocol Style Guide

- Use `snake_case` for field names
- Use `PascalCase` for message and enum types
- Include descriptive comments in `.proto` files
- Prefer `optional` fields for truly optional data
- Use `bytes` with metadata maps for flexible payloads
- Always include error detail information

### Versioning

The crate uses `kagzi.v1` package naming to support future versions:

- Current: `kagzi.v1`
- Future: `kagzi.v2` (breaking changes)
- Migration: Provide adapters in client libraries

## Related Crates

- `kagzi`: Main client library
- `kagzi-server`: Server implementation
- `kagzi-macros`: Procedural macros for workflow definition

## License

See project root LICENSE file.
