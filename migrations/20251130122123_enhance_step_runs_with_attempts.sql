-- Enhance step_runs table to support attempt tracking without separate step_attempts table
-- This replaces step_attempts table approach with a simpler single-table design

-- Add version and parent/child tracking to workflow_runs
ALTER TABLE kagzi.workflow_runs 
ADD COLUMN version TEXT,
ADD COLUMN parent_step_attempt_id TEXT;

-- Enhance step_runs table to support multiple attempts
-- First, drop the existing primary key constraint
ALTER TABLE kagzi.step_runs 
DROP CONSTRAINT step_runs_pkey;

-- Add attempt tracking columns
ALTER TABLE kagzi.step_runs
ADD COLUMN attempt_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
ADD COLUMN attempt_number INTEGER NOT NULL DEFAULT 1,
ADD COLUMN is_latest BOOLEAN DEFAULT true,
ADD COLUMN input JSONB,
ADD COLUMN child_workflow_run_id UUID;

-- Note: error column already exists from initial schema, keeping as TEXT for compatibility

-- Create unique index for latest attempt lookup (critical for BeginStep performance)
CREATE UNIQUE INDEX idx_step_runs_latest 
ON kagzi.step_runs (run_id, step_id) 
WHERE is_latest = true;

-- Create index for attempt history queries (for ListStepAttempts)
CREATE INDEX idx_step_runs_history 
ON kagzi.step_runs (run_id, step_id, attempt_number);

-- Add comment to document the new approach
COMMENT ON TABLE kagzi.step_runs IS 'Enhanced step_runs table that stores all attempts. Use is_latest=true for current state, attempt_number for history.';

-- Function to handle creating new attempts (for future use in CompleteStep/FailStep)
CREATE OR REPLACE FUNCTION kagzi.create_step_attempt(
    p_run_id UUID,
    p_step_id TEXT,
    p_attempt_number INTEGER DEFAULT 1,
    p_input JSONB DEFAULT NULL
) RETURNS UUID AS $$
DECLARE
    v_attempt_id UUID;
BEGIN
    -- Mark previous attempts as not latest
    UPDATE kagzi.step_runs 
    SET is_latest = false 
    WHERE run_id = p_run_id AND step_id = p_step_id;
    
    -- Create new attempt
    INSERT INTO kagzi.step_runs (
        run_id, step_id, attempt_number, is_latest, input, created_at
    ) VALUES (
        p_run_id, p_step_id, p_attempt_number, true, p_input, NOW()
    ) RETURNING attempt_id INTO v_attempt_id;
    
    RETURN v_attempt_id;
END;
$$ LANGUAGE plpgsql;