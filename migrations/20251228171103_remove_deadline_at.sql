-- Remove deadline_at column from workflow_runs table
ALTER TABLE kagzi.workflow_runs DROP COLUMN IF EXISTS deadline_at;

