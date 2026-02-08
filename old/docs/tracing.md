# Distributed Tracing in Kagzi

Kagzi implements W3C Trace Context for distributed tracing across the server and SDK.

## How It Works

### Trace ID Propagation Flow

```
SDK Worker (Rust)                    Kagzi Server
     |                                     |
     | 1. Create workflow_execution span  |
     |    with trace_id & span_id          |
     |                                     |
     | 2. Inject W3C headers              |
     |    (traceparent, tracestate)       |
     |    into gRPC metadata              |
     |                                     |
     +------ complete_workflow RPC ------>|
                                           |
                                3. Extract W3C headers
                                   from gRPC metadata
                                           |
                                4. Set as parent context
                                   for server span
                                           |
                                5. All server operations
                                   inherit same trace_id
```

### Key Components

#### SDK Side (`kagzi-rs`)

- **`propagation::inject_context()`**: Injects current trace context into gRPC metadata
- **`workflow_execution` span**: Created for each workflow execution
- **`step_execution` span**: Created for each step execution
- Context is automatically injected on:
  - `complete_workflow()`
  - `fail_workflow()`
  - `begin_step()`
  - `complete_step()`
  - `fail_step()`
  - `sleep()`

#### Server Side (`kagzi-server`)

- **`propagation::extract_context()`**: Extracts W3C context from gRPC metadata
- **`OpenTelemetrySpanExt::set_parent()`**: Sets extracted context as parent
- Context is automatically extracted on all gRPC service methods
- All spans created during request processing share the same `trace_id`

## Viewing Traces

### Development (Clean Logs)

By default, OTEL exporters are **disabled** for cleaner console output:

```bash
just dev
```

You'll see clean tracing logs but no verbose OTEL span exports.

### Enable OTEL Exporters

To see full span data with trace IDs:

```bash
KAGZI_TELEMETRY_ENABLED=true just dev
```

Output will include verbose span information:

```
Span #0
    Instrumentation Scope
        Name: "kagzi-server"
    
    Name: complete_workflow
    TraceId: 6dd30c72d6bbc4ff7673f945b54d4091  ← Same across worker & server!
    SpanId: 5432f751d0a6eb2f
    ParentSpanId: a1b2c3d4e5f6a7b8  ← Links to SDK span
    ...
```

### Production: Export to OTEL Collector

For production, replace stdout exporters with OTLP exporters:

```toml
# In Cargo.toml, replace:
opentelemetry-stdout = { version = "0.28", features = ["trace", "metrics"] }

# With:
opentelemetry-otlp = { version = "0.28", features = ["trace", "metrics"] }
```

Then update `crates/kagzi-server/src/telemetry/init.rs` to use OTLP exporters
pointing to your collector (Jaeger, Tempo, etc.).

## Configuration

### Environment Variables

- `KAGZI_TELEMETRY_ENABLED` - Enable/disable OTEL exporters (default: `false`)
- `KAGZI_TELEMETRY_SERVICE_NAME` - Service name (default: `"kagzi-server"`)
- `KAGZI_TELEMETRY_LOG_LEVEL` - Log level filter (default: `"info"`)
- `KAGZI_TELEMETRY_LOG_FORMAT` - Log format: `"pretty"` or `"json"` (default: `"pretty"`)
- `RUST_LOG` - Standard tracing env filter (overrides `LOG_LEVEL`)

### Example

```bash
# Clean development logs
just dev

# Debug with full traces
RUST_LOG=debug KAGZI_TELEMETRY_ENABLED=true just dev

# JSON logs for production
KAGZI_TELEMETRY_LOG_FORMAT=json just dev
```

## What Gets Traced

### SDK Spans

- `workflow_execution` - Entire workflow execution
- `step_execution` - Each step execution

### Server Spans

- All gRPC service methods (`start_workflow`, `poll_task`, etc.)
- All repository operations (`create`, `find_by_id`, etc.)
- Queue operations (`notify`)

### Span Attributes

Spans automatically include:

- `run_id` - Workflow run ID
- `workflow_type` - Type of workflow
- `step_name` - Step name (for step operations)
- `namespace_id` - Namespace
- `task_queue` - Task queue
- `code.filepath`, `code.namespace`, `code.lineno` - Source location
- `thread.id`, `thread.name` - Execution thread
- `busy_ns`, `idle_ns` - Performance metrics

## Benefits

1. **End-to-end visibility**: See the complete request flow from SDK → Server → Database
2. **Performance analysis**: Identify bottlenecks with span timing
3. **Error correlation**: Link errors across service boundaries
4. **Debugging**: Follow a specific workflow execution through the entire system

## Language Agnostic

W3C Trace Context is a standard, so you can implement Kagzi workers in any language
(Python, Go, Java, etc.) and traces will propagate correctly as long as the SDK:

1. Injects `traceparent` header into gRPC metadata
2. Creates spans with the OpenTelemetry API

The server will automatically extract and propagate the context.
