# Schedule Test Example

This example demonstrates live schedule testing with Kagzi's cron-based scheduling system.

## What it does

1. **Starts a worker** that can execute the `cleanup_workflow`
2. **Creates a schedule** that fires every 2 minutes using cron expression `0 */2 * * * *`
3. **Runs for 5 minutes** to observe multiple scheduled executions
4. **Cleans up** by deleting the schedule

## Running the Example

Make sure the Kagzi server is running:

```bash
# Start database
just db-up

# Start server
just dev
```

Then run the example:

```bash
cargo run --example 11_schedule_test
```

## Expected Output

You should see:

- Schedule creation confirmation
- Worker started message
- 🔥🔥🔥 messages every 2 minutes when the workflow fires
- Countdown timer showing time remaining
- Schedule deletion confirmation

## Cron Expression Format

Kagzi uses 6-field cron expressions:

```
second minute hour day month weekday
0      */2     *    *   *     *
```

- `0` - At second 0
- `*/2` - Every 2 minutes
- `*` - Every hour
- `*` - Every day
- `*` - Every month
- `*` - Every weekday

Common patterns:

- `0 */5 * * * *` - Every 5 minutes
- `0 0 * * * *` - Every hour (at minute 0)
- `0 0 9 * * *` - Every day at 9:00 AM
- `0 0 9 * * 1-5` - Every weekday at 9:00 AM

## Key Features Demonstrated

- **Schedule Creation**: Using the `.schedule()` API
- **Cron Expressions**: 6-field format with second precision
- **Live Execution**: Observing multiple scheduled firings
- **Worker Integration**: Worker automatically picks up scheduled workflows
- **Cleanup**: Deleting schedules properly
