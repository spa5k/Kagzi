#!/bin/bash

echo "=== Recent test workflows ==="
psql postgres://postgres:postgres@localhost:54122/postgres -c "
SELECT w.run_id, w.status, w.wake_up_at, w.created_at, w.task_queue,
       s.step_id, s.status as step_status, s.created_at as step_created
FROM kagzi.workflow_runs w
LEFT JOIN kagzi.step_runs s ON w.run_id = s.run_id
WHERE w.task_queue = 'e2e-concurrency-transition'
   AND w.created_at > NOW() - INTERVAL '1 minute'
ORDER BY w.created_at DESC
LIMIT 10;"

echo -e "\n=== Scheduler tick logs ==="
# Can't easily check logs here, but we could add logging to see if scheduler is running