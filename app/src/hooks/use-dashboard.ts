import { useParams } from "@tanstack/react-router";
import { useListSchedules, useListWorkers, useListWorkflows } from "@/lib/api-queries";

export function useWorkflows(statusFilter?: string) {
  const params = useParams({ strict: false });
  const namespace = (params as { namespaceId?: string }).namespaceId || "default";
  return useListWorkflows(namespace, statusFilter);
}

export function useSchedules() {
  const params = useParams({ strict: false });
  const namespace = (params as { namespaceId?: string }).namespaceId || "default";
  const schedules = useListSchedules(namespace);
  return {
    ...schedules,
    data: schedules.data?.schedulesList ?? [],
    isLoading: schedules.isLoading,
    error: schedules.error,
  };
}

export function useWorkers() {
  const params = useParams({ strict: false });
  const namespace = (params as { namespaceId?: string }).namespaceId || "default";
  const workers = useListWorkers(namespace);
  return {
    ...workers,
    data: workers.data?.workersList ?? [],
    isLoading: workers.isLoading,
    error: workers.error,
  };
}
