-- Phase 1: Remove the LISTEN/NOTIFY trigger
-- The trigger sends notifications to the wrong channel (md5-hashed).
-- We rely on explicit notify() calls in application code instead.

DROP TRIGGER IF EXISTS trigger_workflow_new_work ON kagzi.workflow_runs;
DROP FUNCTION IF EXISTS kagzi.notify_new_work();
