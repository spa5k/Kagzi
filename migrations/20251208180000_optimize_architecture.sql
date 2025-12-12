-- Kagzi Architecture Simplification Migration
-- This migration removes server-side concurrency counters and optimizes for SKIP LOCKED polling

-- 1. Remove fragile Server-Side Concurrency Limits
DROP TABLE IF EXISTS kagzi.queue_counters;
DROP TABLE IF EXISTS kagzi.queue_configs;
DROP TABLE IF EXISTS kagzi.workflow_type_configs;

-- 2. Index for Robust Polling 
-- Supports: PENDING (New), SLEEPING (Wakeups), RUNNING (Lock Stealing)
DROP INDEX IF EXISTS kagzi.idx_workflow_poll;
CREATE INDEX idx_workflow_poll 
ON kagzi.workflow_runs (namespace_id, task_queue, wake_up_at ASC NULLS FIRST, created_at ASC)
WHERE status IN ('PENDING', 'SLEEPING', 'RUNNING');

-- 3. Index for Admin UI (List by status efficiently)
CREATE INDEX IF NOT EXISTS idx_workflow_status_lookup 
ON kagzi.workflow_runs (namespace_id, status, created_at DESC);
