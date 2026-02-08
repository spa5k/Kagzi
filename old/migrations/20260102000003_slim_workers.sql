-- Remove unused columns from workers table
-- Part of schedule consolidation

ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS max_concurrent;
ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS active_count;
ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS total_completed;
ALTER TABLE kagzi.workers DROP COLUMN IF EXISTS total_failed;
