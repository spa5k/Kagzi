# Separate Files Example

This example demonstrates how to separate workflow logic from your application code, similar to how you would structure it in a real application.

## Project Structure

```
separate_files/
├── src/
│   ├── main.rs       # Application code that triggers workflows
│   ├── worker.rs     # Worker process that executes workflows
│   ├── trigger.rs    # Simple script to trigger a workflow
│   ├── types.rs      # Shared types
│   ├── workflows.rs  # Workflow definitions
│   └── lib.rs        # Library exports
└── Cargo.toml
```

## How to Run

1. **Start the Kagzi server:**
   ```bash
   # Terminal 1
   cd /path/to/kagzi
   docker-compose up -d
   cargo run -p kagzi-server
   ```

2. **Start the worker:**
   ```bash
   # Terminal 2
   cd /path/to/kagzi/examples/separate_files
   cargo run --bin worker
   ```

3. **Trigger workflows:**

   Option A - Run the main app:
   ```bash
   # Terminal 3
   cd /path/to/kagzi/examples/separate_files
   cargo run --bin server
   ```

   Option B - Or trigger manually:
   ```bash
   # Terminal 3
   cd /path/to/kagzi/examples/separate_files
   cargo run --bin trigger
   ```

## What Happens

1. The **worker** process connects to Kagzi and registers the `send-welcome-email` workflow
2. When you **trigger** the workflow, it gets queued in the database
3. The **worker** picks up the workflow and executes it step by step
4. Each step is **memoized** - if the worker crashes, it will resume from the last completed step

## Key Concepts Demonstrated

- **Separation of concerns**: Workflow logic is separate from application code
- **Durable execution**: Workflows survive crashes and restarts
- **Step memoization**: Each step runs exactly once
- **Background processing**: Workers run independently of your app
