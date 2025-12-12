-- Remove unused context column from workflow_payloads and schedules tables
-- Context was never properly implemented and is always NULL

ALTER TABLE kagzi.workflow_payloads DROP COLUMN IF EXISTS context;

ALTER TABLE kagzi.schedules DROP COLUMN IF EXISTS context;