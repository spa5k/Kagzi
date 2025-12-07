-- Add explicit step_kind to step_runs to stop inferring from step_id
ALTER TABLE kagzi.step_runs
    ADD COLUMN step_kind TEXT NOT NULL DEFAULT 'FUNCTION';

-- Best-effort backfill for existing sleep/wait markers
UPDATE kagzi.step_runs
SET step_kind = 'SLEEP'
WHERE step_kind = 'FUNCTION'
  AND (
    LOWER(step_id) LIKE '%sleep%'
    OR LOWER(step_id) LIKE '%wait%'
  );

