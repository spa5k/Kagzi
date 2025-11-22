-- Migration 002: Add Structured Error Support
-- Changes error columns from TEXT to JSONB for structured error information
-- Part of V2 Feature 4: Advanced Error Handling

-- Convert workflow_runs.error from TEXT to JSONB
-- The USING clause handles conversion of existing string errors to JSON
ALTER TABLE workflow_runs
ALTER COLUMN error TYPE JSONB
USING CASE
    WHEN error IS NULL THEN NULL
    WHEN error = '' THEN NULL
    ELSE jsonb_build_object('message', error)
END;

-- Convert step_runs.error from TEXT to JSONB
-- The USING clause handles conversion of existing string errors to JSON
ALTER TABLE step_runs
ALTER COLUMN error TYPE JSONB
USING CASE
    WHEN error IS NULL THEN NULL
    WHEN error = '' THEN NULL
    ELSE jsonb_build_object('message', error)
END;

-- Create index for querying errors by kind (for analytics)
CREATE INDEX idx_workflow_runs_error_kind ON workflow_runs((error->>'kind')) WHERE error IS NOT NULL;
CREATE INDEX idx_step_runs_error_kind ON step_runs((error->>'kind')) WHERE error IS NOT NULL;
