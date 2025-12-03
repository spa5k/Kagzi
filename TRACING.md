# Tracing and Observability

Kagzi now includes comprehensive structured logging and distributed tracing capabilities for production monitoring and debugging.

## Features

### ✅ **Structured Logging**
- JSON-formatted logs for easy parsing by log aggregation systems
- Correlation IDs for request tracking across services
- Trace IDs for distributed tracing
- Thread-aware logging with thread names and IDs
- Configurable log levels via environment variables

### ✅ **Correlation ID Propagation**
- Automatic correlation ID generation for new requests
- Propagation through gRPC metadata headers
- End-to-end request tracking from client to server
- Context preservation across async boundaries

### ✅ **Health Check Endpoint**
- Database connectivity verification
- Service status reporting
- Timestamped health responses
- gRPC-based health checks for load balancers

### ✅ **Distributed Tracing Ready**
- Trace ID propagation across service boundaries
- Structured span creation with context
- OpenTelemetry-compatible (foundation for future integration)
- Jaeger exporter support (configurable)

## Usage

### Server Configuration

```bash
# Set log level (default: info)
export RUST_LOG=info,kagzi_server=debug

# Enable JSON logging
export RUST_LOG_FORMAT=json

# Optional: Enable Jaeger tracing
export JAEGER_ENDPOINT=http://localhost:14268/api/traces
```

### Client Usage

```rust
use kagzi::{Worker, WorkflowContext};
use kagzi::tracing_utils::init_tracing;

// Initialize tracing for the SDK
init_tracing()?;

// Worker automatically propagates correlation IDs
let mut worker = Worker::new("http://localhost:50051", "my-queue").await?;

// All workflow steps will have correlation context
worker.register("MyWorkflow", my_handler);
worker.run().await;
```

### Log Output Example

```json
{
  "timestamp": "2025-12-03T10:30:45.123456Z",
  "level": "INFO",
  "target": "kagzi_server::service",
  "span": {
    "grpc_request": {
      "method": "StartWorkflow",
      "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
      "trace_id": "550e8400-e29b-41d4-a716-446655440001"
    }
  },
  "message": "gRPC request received",
  "fields": {
    "method": "StartWorkflow",
    "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
    "trace_id": "550e8400-e29b-41d4-a716-446655440001"
  }
}
```

## Health Check

### gRPC Request
```protobuf
message HealthCheckRequest {
    string service = 1; // Optional service name to check
}
```

### gRPC Response
```protobuf
message HealthCheckResponse {
    enum ServingStatus {
        SERVING = 1;
        NOT_SERVING = 2;
    }
    ServingStatus status = 1;
    string message = 2;
    google.protobuf.Timestamp timestamp = 3;
}
```

### Client Example
```rust
use kagzi_proto::kagzi::{workflow_service_client::WorkflowServiceClient, HealthCheckRequest};

let mut client = WorkflowServiceClient::connect("http://localhost:50051").await?;

let request = tonic::Request::new(HealthCheckRequest {
    service: "kagzi-server".to_string(),
});

let response = client.health_check(request).await?;
println!("Service status: {:?}", response.into_inner().status);
```

## Environment Variables

| Variable | Default | Description |
|----------|----------|-------------|
| `RUST_LOG` | `info` | Log level filter (e.g., `debug`, `warn`, `error`) |
| `JAEGER_ENDPOINT` | `http://localhost:14268/api/traces` | Jaeger collector endpoint |
| `OTEL_SERVICE_NAME` | `kagzi-server` | Service name for tracing |

## Integration with Monitoring Systems

### **ELK Stack**
The JSON log format integrates seamlessly with Elasticsearch, Logstash, and Kibana.

### **Prometheus + Grafana**
Correlation IDs can be used as labels for metrics aggregation.

### **Jaeger**
When `JAEGER_ENDPOINT` is configured, traces are automatically exported to Jaeger.

### **Datadog**
The structured JSON format is compatible with Datadog log ingestion.

## Performance Considerations

- **Low Overhead**: Tracing uses efficient string interning and lazy evaluation
- **Async-Friendly**: All tracing operations are non-blocking
- **Configurable**: Can be disabled in production via environment variables
- **Metadata Propagation**: Uses efficient gRPC metadata headers

## Security Notes

- **No Sensitive Data**: Correlation and trace IDs are random UUIDs
- **Configurable**: Can disable tracing in high-security environments  
- **Header Sanitization**: All metadata is validated before processing

## Future Enhancements

- [ ] OpenTelemetry metrics integration
- [ ] Automatic sampling configuration
- [ ] Custom span enrichment
- [ ] Log aggregation with buffering
- [ ] Real-time monitoring dashboard