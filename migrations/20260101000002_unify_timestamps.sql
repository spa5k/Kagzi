-- Phase 2: Unify timestamps into single `available_at` column
-- Replaces: wake_up_at, locked_until
-- Controls when a workflow becomes claimable by workers

-- Add new column
ALTER TABLE kagzi.workflow_runs ADD COLUMN available_at TIMESTAMPTZ;

-- Populate from existing data
UPDATE kagzi.workflow_runs SET available_at = 
    CASE 
        WHEN status = 'PENDING' THEN COALESCE(wake_up_at, created_at)
        WHEN status = 'SLEEPING' THEN wake_up_at
        WHEN status = 'RUNNING' THEN locked_until
        ELSE NULL  -- terminal states don't need available_at
    END;

-- Make non-null for active workflows
ALTER TABLE kagzi.workflow_runs 
    ADD CONSTRAINT chk_available_at 
    CHECK (status IN ('COMPLETED', 'FAILED', 'CANCELLED') OR available_at IS NOT NULL);

-- Drop old columns
ALTER TABLE kagzi.workflow_runs DROP COLUMN IF EXISTS wake_up_at;
ALTER TABLE kagzi.workflow_runs DROP COLUMN IF EXISTS locked_until;

-- Drop old indexes that reference removed columns
DROP INDEX IF EXISTS kagzi.idx_queue_poll;
DROP INDEX IF EXISTS kagzi.idx_queue_poll_hot;
DROP INDEX IF EXISTS kagzi.idx_workflow_poll;
DROP INDEX IF EXISTS kagzi.idx_workflow_runs_claimable;

-- Create new unified index for polling
CREATE INDEX idx_workflow_available ON kagzi.workflow_runs 
    USING btree (namespace_id, task_queue, available_at)
    WHERE status IN ('PENDING', 'SLEEPING', 'RUNNING');
