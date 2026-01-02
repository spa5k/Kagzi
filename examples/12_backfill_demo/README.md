# Schedule Backfill Demo

Demonstrates Kagzi's schedule backfill behavior when a worker is unavailable.

## What It Does

1. Creates a schedule that fires every **10 seconds**
2. Runs worker normally for 30 seconds (expect ~3 fires)
3. **Stops the worker** for 25 seconds (missing 2-3 scheduled runs)
4. Restarts worker and **watches backfill** catch up missed runs

## Run

```bash
# Make sure server is running
cargo run --example 12_backfill_demo
```

## Expected Output

```
🔥 [12:00:00] FIRE #1 - Workflow executed
🔥 [12:00:10] FIRE #2 - Workflow executed
🔥 [12:00:20] FIRE #3 - Workflow executed

🔴 PHASE 2: Worker OFFLINE (25 seconds)
  💤 Downtime: 5 seconds... (missed fires accumulating)
  ...

🟢 PHASE 3: Worker BACK ONLINE - Watching backfill!
🔥 [12:00:50] FIRE #4 - Workflow executed (backfill: 12:00:30)
🔥 [12:00:51] FIRE #5 - Workflow executed (backfill: 12:00:40)
🔥 [12:00:52] FIRE #6 - Workflow executed (current)

✅ Backfill detected! Missed runs were caught up.
```

## Key Config

- `max_catchup: 50` (default) - catches up to 50 missed runs
- `max_catchup: 0` - skip all missed, jump to current time
