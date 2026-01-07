import { useListWorkflows, useListSchedules, useListWorkers } from "@/lib/api-queries";

export function useWorkflows(statusFilter?: string) {
  return useListWorkflows("default", statusFilter);
}

export function useSchedules() {
  const schedules = useListSchedules("default");
  return {
    ...schedules,
    data: schedules.data?.schedulesList ?? [],
    isLoading: schedules.isLoading,
    error: schedules.error,
  };
}

export function useWorkers() {
  const workers = useListWorkers("default");
  return {
    ...workers,
    data: workers.data?.workersList ?? [],
    isLoading: workers.isLoading,
    error: workers.error,
  };
}
