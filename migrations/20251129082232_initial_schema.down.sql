-- Revert migration script here
DROP TABLE IF EXISTS step_runs;
DROP INDEX IF EXISTS idx_queue_poll;
DROP TABLE IF EXISTS workflow_runs;

