CREATE OR REPLACE FUNCTION kagzi.notify_new_work() RETURNS trigger AS $$
BEGIN
  PERFORM pg_notify('kagzi_work_' || md5(NEW.namespace_id || '_' || NEW.task_queue), NEW.run_id::text);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trigger_workflow_new_work
AFTER INSERT OR UPDATE ON kagzi.workflow_runs
FOR EACH ROW
WHEN (NEW.status = 'PENDING' AND (NEW.wake_up_at IS NULL OR NEW.wake_up_at <= NOW()))
EXECUTE FUNCTION kagzi.notify_new_work();
