-- Refactor namespace table to use UUID primary key and rename namespace_id to namespace
-- Add enabled field for enable/disable functionality (deletion is not allowed)

-- Step 1: Add id UUID column
ALTER TABLE kagzi.namespaces
ADD COLUMN id UUID DEFAULT gen_random_uuid();

-- Step 2: Populate id for existing rows
UPDATE kagzi.namespaces
SET id = gen_random_uuid()
WHERE id IS NULL;

-- Step 3: Make id NOT NULL
ALTER TABLE kagzi.namespaces
ALTER COLUMN id SET NOT NULL;

-- Step 4: Add enabled boolean column
ALTER TABLE kagzi.namespaces
ADD COLUMN IF NOT EXISTS enabled BOOLEAN NOT NULL DEFAULT TRUE;

-- Step 5: Drop existing primary key
ALTER TABLE kagzi.namespaces
DROP CONSTRAINT namespaces_pkey;

-- Step 6: Rename namespace_id to namespace
ALTER TABLE kagzi.namespaces
RENAME COLUMN namespace_id TO namespace;

-- Step 7: Add unique constraint on namespace
ALTER TABLE kagzi.namespaces
ADD CONSTRAINT namespaces_namespace_key UNIQUE (namespace);

-- Step 8: Add primary key on id
ALTER TABLE kagzi.namespaces
ADD PRIMARY KEY (id);

-- Step 9: Update index for active namespaces to include enabled flag
DROP INDEX IF EXISTS kagzi.idx_namespaces_active;
CREATE INDEX idx_namespaces_active ON kagzi.namespaces (namespace) WHERE deleted_at IS NULL AND enabled = TRUE;

-- Step 10: Rename namespace_id column in workflow_runs table
ALTER TABLE kagzi.workflow_runs RENAME COLUMN namespace_id TO namespace;

-- Step 11: Rename namespace_id column in workers table
ALTER TABLE kagzi.workers RENAME COLUMN namespace_id TO namespace;

-- Step 12: Rename namespace_id column in step_runs table
ALTER TABLE kagzi.step_runs RENAME COLUMN namespace_id TO namespace;

-- Step 13: Update indexes that reference namespace (they will auto-update, but let's document)
-- These indexes are automatically updated by PostgreSQL:
-- - idx_workflow_available (now uses namespace instead of namespace_id)
-- - idx_workflow_runs_running (now uses namespace instead of namespace_id)
-- - idx_workflow_schedules_due (now uses namespace instead of namespace_id)
-- - idx_workflow_status_lookup (now uses namespace instead of namespace_id)
-- - idx_workers_queue (now uses namespace instead of namespace_id)
-- - idx_workers_active_unique (now uses namespace instead of namespace_id)
-- - uq_active_workflow (now uses namespace instead of namespace_id)

-- Step 14: Add foreign key constraints with RESTRICT to prevent namespace deletion with resources
-- Note: NOT VALID skips checking existing data for performance. Validate later if needed.
-- Run: ALTER TABLE kagzi.workflow_runs VALIDATE CONSTRAINT fk_workflow_namespace;

ALTER TABLE kagzi.workflow_runs
ADD CONSTRAINT fk_workflow_namespace
FOREIGN KEY (namespace) REFERENCES kagzi.namespaces(namespace)
ON DELETE RESTRICT
ON UPDATE CASCADE
NOT VALID;

ALTER TABLE kagzi.workers
ADD CONSTRAINT fk_worker_namespace
FOREIGN KEY (namespace) REFERENCES kagzi.namespaces(namespace)
ON DELETE RESTRICT
ON UPDATE CASCADE
NOT VALID;

ALTER TABLE kagzi.step_runs
ADD CONSTRAINT fk_step_namespace
FOREIGN KEY (namespace) REFERENCES kagzi.namespaces(namespace)
ON DELETE RESTRICT
ON UPDATE CASCADE
NOT VALID;

-- Step 15: Add comment to document that namespaces are enabled/disabled, not deleted
COMMENT ON TABLE kagzi.namespaces IS 'Namespaces provide multi-tenancy isolation. Namespaces are enabled/disabled, not deleted.';
COMMENT ON COLUMN kagzi.namespaces.enabled IS 'When FALSE, namespace is disabled and no workflows can run in it';
COMMENT ON COLUMN kagzi.namespaces.namespace IS 'Unique identifier for the namespace (user-facing, like "production" or "staging")';
COMMENT ON COLUMN kagzi.namespaces.id IS 'Internal UUID primary key for database operations';
