-- Migration 004: Add Parallel Step Execution Support
-- Enables concurrent execution of multiple steps within a workflow
-- Part of V2 Feature 2: Parallel Step Execution

-- Add parallel execution tracking columns to step_runs
ALTER TABLE step_runs
ADD COLUMN parent_step_id TEXT,
ADD COLUMN parallel_group_id UUID;

-- Add indexes for efficient parallel queries

-- Index for finding all steps in a parallel group
CREATE INDEX idx_step_runs_parallel_group_id
ON step_runs(parallel_group_id)
WHERE parallel_group_id IS NOT NULL;

-- Index for finding child steps of a parent
CREATE INDEX idx_step_runs_parent_step_id
ON step_runs(workflow_run_id, parent_step_id)
WHERE parent_step_id IS NOT NULL;

-- Composite index for parallel group queries with status
CREATE INDEX idx_step_runs_parallel_group_status
ON step_runs(parallel_group_id, status)
WHERE parallel_group_id IS NOT NULL;

-- Index for checking cache with parallel context
-- Useful for checking if a step with same input is already cached in this parallel group
CREATE INDEX idx_step_runs_cache_lookup
ON step_runs(workflow_run_id, step_id, input_hash);

-- Add comment documentation
COMMENT ON COLUMN step_runs.parent_step_id IS 'Reference to parent step for nested parallel execution (NULL for top-level steps)';
COMMENT ON COLUMN step_runs.parallel_group_id IS 'UUID grouping parallel steps executed together (NULL for sequential steps)';
