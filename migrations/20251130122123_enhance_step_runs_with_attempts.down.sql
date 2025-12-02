-- Rollback the enhanced step_runs changes

-- Drop the helper function
DROP FUNCTION IF EXISTS kagzi.create_step_attempt(UUID, TEXT, INTEGER, JSONB);

-- Drop the new indexes
DROP INDEX IF EXISTS kagzi.idx_step_runs_history;
DROP INDEX IF EXISTS kagzi.idx_step_runs_latest;

-- Remove the new columns from step_runs
ALTER TABLE kagzi.step_runs
DROP COLUMN IF EXISTS child_workflow_run_id,
DROP COLUMN IF EXISTS error,
DROP COLUMN IF EXISTS input,
DROP COLUMN IF EXISTS is_latest,
DROP COLUMN IF EXISTS attempt_number,
DROP COLUMN IF EXISTS attempt_id;

-- Restore the original primary key constraint
ALTER TABLE kagzi.step_runs
ADD CONSTRAINT step_runs_pkey PRIMARY KEY (run_id, step_id);

-- Remove columns from workflow_runs
ALTER TABLE kagzi.workflow_runs
DROP COLUMN IF EXISTS parent_step_attempt_id,
DROP COLUMN IF EXISTS version;