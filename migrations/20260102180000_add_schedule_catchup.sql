-- Add schedule catchup columns to workflow_runs
-- Enables backfill-aware schedule firing

-- last_fired_at: Tracks when the schedule last fired (for catchup calculation)
ALTER TABLE kagzi.workflow_runs ADD COLUMN last_fired_at TIMESTAMPTZ;

-- max_catchup: Per-schedule limit for missed run catchup
-- Default 50: catch up to 50 missed runs
-- Value 0: never backfill, skip to current time
ALTER TABLE kagzi.workflow_runs ADD COLUMN max_catchup INT NOT NULL DEFAULT 50;
