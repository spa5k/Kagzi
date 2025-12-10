-- Convert workflow and step payload columns from JSONB to BYTEA for opaque binary payloads.
-- Existing JSON values are converted using their text representation.

ALTER TABLE kagzi.workflow_payloads
    ALTER COLUMN input TYPE BYTEA USING convert_to(input::text, 'UTF8'),
    ALTER COLUMN output TYPE BYTEA USING convert_to(output::text, 'UTF8');

ALTER TABLE kagzi.step_runs
    ALTER COLUMN input TYPE BYTEA USING convert_to(input::text, 'UTF8'),
    ALTER COLUMN output TYPE BYTEA USING convert_to(output::text, 'UTF8');
