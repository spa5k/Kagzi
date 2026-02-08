# Kagzi Examples TUI

A terminal user interface for running and exploring Kagzi workflow examples with real-time log viewing.

## Running the TUI

```bash
# Using just
just tui

# Or using cargo directly
cargo run --example tui
```

## Features

- **Browse Examples**: Navigate through all 10 example categories
- **Select Variants**: Each example has multiple variants demonstrating different features
- **Real-time Logs**: Watch example output as it runs (streamed live from the process)
- **Scrollable Output**: Navigate through long output with scroll support
- **Keyboard Navigation**: Use arrow keys or vim-style navigation (j/k)
- **Non-blocking**: The TUI remains responsive while examples run in the background

## Keyboard Shortcuts

### Example/Variant List

- `↑` / `k` - Move up
- `↓` / `j` - Move down
- `Enter` - Select example/variant
- `q` - Quit

### While Running/Viewing Output

- `↑` / `k` - Scroll up (after completion)
- `↓` / `j` - Scroll down (after completion)
- `Enter` - Return to example list (after completion)
- `q` - Quit
- `Any other key` - Stop the running process

## Examples

The TUI covers all examples:

1. **01_basics** - Basic workflow execution (hello, chain, context, sleep)
2. **02_error_handling** - Error handling strategies (flaky, fatal, override)
3. **03_scheduling** - Time-based scheduling (cron, sleep, catchup)
4. **04_concurrency** - Concurrency control (local, multi-worker)
5. **05_fan_out_in** - Parallel execution patterns (static fan-out, map-reduce)
6. **06_long_running** - Long-running workflows (polling, timeout)
7. **07_idempotency** - Idempotency guarantees (external ID, memoization)
8. **08_saga_pattern** - Compensating transactions (saga, partial)
9. **09_data_pipeline** - Data processing (transform, large payloads)
10. **10_multi_queue** - Multi-queue patterns (priority, namespace)

## Technical Details

The TUI runs each example in a separate thread and captures stdout line-by-line, displaying it in real-time. The output is stored in an `Arc<Mutex<String>>` for thread-safe access, and the UI auto-scrolls to show the latest output while the example is running.
