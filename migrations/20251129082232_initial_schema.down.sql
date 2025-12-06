-- Revert migration script here
DROP TABLE IF EXISTS kagzi.step_runs;
DROP INDEX IF EXISTS kagzi.idx_queue_poll;
DROP TABLE IF EXISTS kagzi.workflow_runs;

