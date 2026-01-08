import { useNamespace } from "@/hooks/use-namespace";
import { useListSchedules, useListWorkers, useListWorkflows } from "@/lib/api-queries";

export function useWorkflows(statusFilter?: string) {
  const { namespace } = useNamespace();
  return useListWorkflows(namespace, statusFilter);
}

export function useSchedules() {
  const { namespace } = useNamespace();
  const schedules = useListSchedules(namespace);
  return {
    ...schedules,
    data: schedules.data?.schedulesList ?? [],
    isLoading: schedules.isLoading,
    error: schedules.error,
  };
}

export function useWorkers() {
  const { namespace } = useNamespace();
  const workers = useListWorkers(namespace);
  return {
    ...workers,
    data: workers.data?.workersList ?? [],
    isLoading: workers.isLoading,
    error: workers.error,
  };
}
