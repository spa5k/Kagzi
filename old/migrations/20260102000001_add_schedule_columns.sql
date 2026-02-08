-- Add schedule columns to workflow_runs
-- Part of schedule consolidation: 6 tables -> 4 tables

-- Add cron_expr and schedule_id columns to workflow_runs
ALTER TABLE kagzi.workflow_runs ADD COLUMN cron_expr TEXT;
ALTER TABLE kagzi.workflow_runs ADD COLUMN schedule_id UUID;

-- Index: find due schedules
CREATE INDEX idx_workflow_schedules_due
ON kagzi.workflow_runs (namespace_id, available_at)
WHERE status = 'SCHEDULED';

-- Index: schedule history
CREATE INDEX idx_workflow_schedule_history
ON kagzi.workflow_runs (schedule_id, created_at DESC)
WHERE schedule_id IS NOT NULL;

-- Index: deduplication (prevent double-firing)
CREATE UNIQUE INDEX idx_schedule_firing_unique
ON kagzi.workflow_runs (schedule_id, available_at)
WHERE schedule_id IS NOT NULL;

-- Foreign key (self-referential)
ALTER TABLE kagzi.workflow_runs
ADD CONSTRAINT fk_schedule_id
FOREIGN KEY (schedule_id) REFERENCES kagzi.workflow_runs(run_id) ON DELETE SET NULL;
