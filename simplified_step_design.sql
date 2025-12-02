-- Simplified step_runs design that stores all attempts
-- No need for separate step_attempts table

ALTER TABLE kagzi.step_runs 
DROP CONSTRAINT step_runs_pkey,  -- Remove old primary key

-- Add attempt tracking
ADD COLUMN attempt_number INTEGER NOT NULL DEFAULT 1,
ADD COLUMN attempt_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
ADD COLUMN input JSONB,
ADD COLUMN error JSONB,
ADD COLUMN is_latest BOOLEAN DEFAULT true;

-- New composite key for latest attempt lookup
CREATE UNIQUE INDEX idx_step_runs_latest 
ON kagzi.step_runs (run_id, step_id) 
WHERE is_latest = true;

-- Index for attempt history
CREATE INDEX idx_step_runs_history 
ON kagzi.step_runs (run_id, step_id, attempt_number);

-- Query examples:
-- Get latest step result (for BeginStep):
-- SELECT * FROM step_runs WHERE run_id = $1 AND step_id = $2 AND is_latest = true

-- Get all attempts (for ListStepAttempts):
-- SELECT * FROM step_runs WHERE run_id = $1 AND step_id = $2 ORDER BY attempt_number

-- Add new attempt:
-- UPDATE step_runs SET is_latest = false WHERE run_id = $1 AND step_id = $2
-- INSERT INTO step_runs (run_id, step_id, attempt_number, is_latest, ...) VALUES (..., 2, true, ...)