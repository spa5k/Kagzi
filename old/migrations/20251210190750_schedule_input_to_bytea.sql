-- Drop the old JSON default first to avoid implicit cast issues, then convert.
ALTER TABLE kagzi.schedules
    ALTER COLUMN input DROP DEFAULT,
    ALTER COLUMN input TYPE BYTEA USING convert_to(input::text, 'UTF8'),
    ALTER COLUMN input SET DEFAULT ''::bytea;
