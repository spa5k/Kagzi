-- Test wake_sleeping function
-- Create a test sleeping workflow that should wake up
INSERT INTO kagzi.workflow_runs (
    run_id,
    namespace_id,
    task_queue,
    workflow_type,
    status,
    wake_up_at,
    created_at,
    updated_at
) VALUES (
    gen_random_uuid(),
    'default',
    'test',
    'test_workflow',
    'SLEEPING',
    NOW() - INTERVAL '1 second', -- 1 second ago (should wake up)
    NOW(),
    NOW()
);

-- Check if it's there
SELECT run_id, status, wake_up_at, NOW() as current_time
FROM kagzi.workflow_runs
WHERE status = 'SLEEPING' AND wake_up_at <= NOW();

-- Run wake_sleeping logic
UPDATE kagzi.workflow_runs
SET status = 'PENDING',
    wake_up_at = NULL
WHERE run_id IN (
    SELECT run_id
    FROM kagzi.workflow_runs
    WHERE status = 'SLEEPING'
      AND wake_up_at <= NOW()
    FOR UPDATE SKIP LOCKED
    LIMIT 10
);

-- Check result
SELECT status, wake_up_at
FROM kagzi.workflow_runs
WHERE workflow_type = 'test_workflow';

-- Clean up
DELETE FROM kagzi.workflow_runs WHERE workflow_type = 'test_workflow';