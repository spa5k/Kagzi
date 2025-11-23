-- Migration 005: Add Workflow Versioning Support
-- Enables safe deployment of workflow changes with version management
-- Part of V2 Feature 3: Workflow Versioning

-- Drop the old TEXT workflow_version column and add INTEGER version
-- This is safe since versioning feature was not yet implemented
ALTER TABLE workflow_runs
DROP COLUMN workflow_version,
ADD COLUMN workflow_version INTEGER NOT NULL DEFAULT 1;

-- Create index for efficient version queries
CREATE INDEX idx_workflow_runs_name_version
ON workflow_runs(workflow_name, workflow_version);

-- Create workflow_versions table to track available versions
CREATE TABLE workflow_versions (
    workflow_name TEXT NOT NULL,
    version INTEGER NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deprecated_at TIMESTAMPTZ,
    description TEXT,
    PRIMARY KEY (workflow_name, version)
);

-- Ensure only one default version per workflow
CREATE UNIQUE INDEX idx_workflow_versions_default
ON workflow_versions(workflow_name, is_default)
WHERE is_default = TRUE;

-- Create index for active (non-deprecated) versions
CREATE INDEX idx_workflow_versions_active
ON workflow_versions(workflow_name, created_at)
WHERE deprecated_at IS NULL;

-- Add comments for documentation
COMMENT ON COLUMN workflow_runs.workflow_version IS 'Version of the workflow definition used for this run';
COMMENT ON TABLE workflow_versions IS 'Registry of available workflow versions with default tracking';
COMMENT ON COLUMN workflow_versions.is_default IS 'Whether this version should be used for new workflow runs';
COMMENT ON COLUMN workflow_versions.deprecated_at IS 'When this version was marked as deprecated (NULL if active)';
