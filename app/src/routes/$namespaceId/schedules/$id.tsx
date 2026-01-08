import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  GetWorkflowScheduleRequest,
  ListScheduleRunsRequest,
  PauseWorkflowScheduleRequest,
  ResumeWorkflowScheduleRequest,
  TriggerWorkflowScheduleRequest,
} from "@/gen/workflow_schedule_pb";
import {
  useDeleteSchedule,
  useGetSchedule,
  useListScheduleRuns,
  usePauseSchedule,
  useResumeSchedule,
  useTriggerSchedule,
} from "@/hooks/use-grpc-services";
import { useNamespace } from "@/hooks/use-namespace";
import { cn } from "@/lib/utils";
import { WorkflowStatus, WorkflowStatusLabel } from "@/types";
import {
  Alert01Icon,
  ArrowLeft01Icon,
  Clock01Icon,
  Copy01Icon,
  FlashIcon,
  PauseIcon,
  PlayIcon,
  Trash,
} from "@hugeicons/core-free-icons";
import type { IconSvgElement } from "@hugeicons/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { createFileRoute, useNavigate } from "@tanstack/react-router";

function Icon({ icon, className }: { icon: IconSvgElement; className?: string }) {
  return <HugeiconsIcon icon={icon} className={className} />;
}

function getStatusColor(status: number) {
  switch (status) {
    case WorkflowStatus.RUNNING:
      return "text-primary bg-primary/10 border-primary/20";
    case WorkflowStatus.COMPLETED:
      return "text-primary bg-primary/10 border-primary/20";
    case WorkflowStatus.FAILED:
      return "text-destructive bg-destructive/10 border-destructive/20";
    case WorkflowStatus.CANCELLED:
      return "text-muted-foreground bg-muted/50 border-border";
    default:
      return "text-muted-foreground bg-muted/50 border-border";
  }
}

function formatDateTime(timestamp?: Date) {
  if (!timestamp) return "-";
  return new Date(timestamp).toLocaleString("en-US", {
    month: "numeric",
    day: "numeric",
    hour: "numeric",
    minute: "numeric",
    second: "numeric",
    hour12: false,
  });
}

function formatDuration(startTime?: Date, endTime?: Date) {
  if (!startTime) return "-";
  const start = new Date(startTime).getTime();
  const end = endTime ? new Date(endTime).getTime() : Date.now();
  const diff = end - start;

  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export const Route = createFileRoute("/$namespaceId/schedules/$id")({
  component: ScheduleDetailPage,
});

function ScheduleDetailPage() {
  const { id, namespaceId } = Route.useParams();
  const navigate = useNavigate();
  const { namespace } = useNamespace();

  const {
    data: schedule,
    isLoading: scheduleLoading,
    error: scheduleError,
    refetch: refetchSchedule,
  } = useGetSchedule(
    new GetWorkflowScheduleRequest({
      scheduleId: id,
      namespaceId: namespace,
    }),
  );

  const {
    data: runsData,
    isLoading: runsLoading,
    refetch: refetchRuns,
  } = useListScheduleRuns(
    new ListScheduleRunsRequest({
      scheduleId: id,
      namespaceId: namespace,
    }),
  );

  const triggerSchedule = useTriggerSchedule();
  const pauseSchedule = usePauseSchedule();
  const resumeSchedule = useResumeSchedule();
  const deleteSchedule = useDeleteSchedule();

  const handleTrigger = async () => {
    try {
      await triggerSchedule.mutateAsync(
        new TriggerWorkflowScheduleRequest({
          scheduleId: id,
          namespaceId: namespace,
        }),
      );
      refetchSchedule();
      refetchRuns();
    } catch (error) {
      console.error("Failed to trigger schedule:", error);
    }
  };

  const handlePause = async () => {
    try {
      await pauseSchedule.mutateAsync(
        new PauseWorkflowScheduleRequest({
          scheduleId: id,
          namespaceId: namespace,
        }),
      );
      refetchSchedule();
    } catch (error) {
      console.error("Failed to pause schedule:", error);
    }
  };

  const handleResume = async () => {
    try {
      await resumeSchedule.mutateAsync(
        new ResumeWorkflowScheduleRequest({
          scheduleId: id,
          namespaceId: namespace,
        }),
      );
      refetchSchedule();
    } catch (error) {
      console.error("Failed to resume schedule:", error);
    }
  };

  const handleDelete = async () => {
    try {
      await deleteSchedule.mutateAsync({
        scheduleId: id,
        namespaceId: namespace,
      });
      navigate({ to: "/$namespaceId/schedules", params: { namespaceId } });
    } catch (error) {
      console.error("Failed to delete schedule:", error);
    }
  };

  if (scheduleLoading) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background">
        <div className="text-muted-foreground font-mono text-xs animate-pulse">
          Loading schedule...
        </div>
      </div>
    );
  }

  if (scheduleError || !schedule) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background font-mono text-xs text-destructive">
        <div className="border border-destructive/20 bg-destructive/5 p-8 max-w-md w-full">
          <div className="flex items-center gap-2 mb-4 text-destructive">
            <Icon icon={Alert01Icon} className="size-5" />
            <h3 className="font-bold uppercase tracking-widest">Schedule Not Found</h3>
          </div>
          <p className="mb-6 opacity-80">
            {scheduleError?.message || "The requested schedule could not be found."}
          </p>
          <Button
            onClick={() => navigate({ to: "/$namespaceId/schedules", params: { namespaceId } })}
            variant="outline"
            className="w-full font-mono text-xs uppercase tracking-wider border-destructive text-destructive hover:bg-destructive hover:text-white"
          >
            Return to Schedules
          </Button>
        </div>
      </div>
    );
  }

  const runs = runsData?.runs ?? [];
  const isPending =
    triggerSchedule.isPending ||
    pauseSchedule.isPending ||
    resumeSchedule.isPending ||
    deleteSchedule.isPending;

  return (
    <div className="min-h-screen bg-background text-foreground animate-in fade-in duration-200">
      <div className="sticky top-0 z-10 border-b border-border bg-background/80 backdrop-blur-md">
        <div className="flex items-center justify-between px-6 py-4 max-w-[1800px] mx-auto w-full">
          <div className="flex items-center gap-6">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => navigate({ to: "/$namespaceId/schedules", params: { namespaceId } })}
              className="font-mono text-xs uppercase tracking-wider text-muted-foreground hover:text-primary -ml-2 gap-2"
            >
              <Icon icon={ArrowLeft01Icon} className="size-4" />
              Return to Schedules
            </Button>
            <div className="h-6 w-px bg-border" />
            <div className="flex items-center gap-3">
              <span className="font-bold text-lg tracking-tight uppercase">
                {schedule.schedule?.workflowType}
              </span>
              <Badge
                variant="outline"
                className={cn(
                  "gap-1.5 pl-1.5 pr-2.5 py-0.5 rounded-full text-[10px] font-mono uppercase tracking-widest",
                  schedule.schedule?.enabled
                    ? "text-primary bg-primary/10 border-primary/20"
                    : "text-muted-foreground bg-muted/50 border-border",
                )}
              >
                {schedule.schedule?.enabled ? (
                  <>
                    <div className="size-1.5 rounded-full bg-primary animate-pulse" />
                    Active
                  </>
                ) : (
                  <>
                    <div className="size-1.5 rounded-full bg-muted-foreground" />
                    Paused
                  </>
                )}
              </Badge>
            </div>
          </div>

          <div className="flex items-center gap-3">
            <Button
              variant="outline"
              size="sm"
              onClick={handleTrigger}
              disabled={isPending || !schedule.schedule?.enabled}
              className="font-mono text-xs uppercase tracking-wider"
            >
              <Icon icon={FlashIcon} className="size-3 mr-2" />
              Trigger Now
            </Button>

            {schedule.schedule?.enabled ? (
              <Button
                variant="outline"
                size="sm"
                onClick={handlePause}
                disabled={isPending}
                className="font-mono text-xs uppercase tracking-wider"
              >
                <Icon icon={PauseIcon} className="size-3 mr-2" />
                Pause
              </Button>
            ) : (
              <Button
                variant="outline"
                size="sm"
                onClick={handleResume}
                disabled={isPending}
                className="font-mono text-xs uppercase tracking-wider"
              >
                <Icon icon={PlayIcon} className="size-3 mr-2" />
                Resume
              </Button>
            )}

            <AlertDialog>
              <AlertDialogTrigger
                disabled={isPending}
                className={cn(
                  "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors",
                  "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                  "disabled:pointer-events-none disabled:opacity-50",
                  "border border-input bg-background shadow-sm hover:bg-accent hover:text-accent-foreground",
                  "h-9 px-4 py-2",
                  "font-mono text-xs uppercase tracking-wider",
                  "text-destructive border-destructive/30 hover:bg-destructive hover:text-white hover:border-destructive transition-colors",
                )}
              >
                <HugeiconsIcon icon={Trash} className="size-3" />
                Delete
              </AlertDialogTrigger>
              <AlertDialogContent size="default">
                <AlertDialogHeader>
                  <AlertDialogMedia>
                    <Icon icon={Alert01Icon} className="size-6 text-destructive" />
                  </AlertDialogMedia>
                  <AlertDialogTitle className="font-mono text-sm uppercase tracking-wider">
                    Delete Schedule?
                  </AlertDialogTitle>
                  <AlertDialogDescription className="font-mono text-xs">
                    This action will permanently delete the schedule{" "}
                    <span className="font-bold text-foreground">
                      {schedule.schedule?.scheduleId.slice(0, 8)}
                    </span>
                    .
                    <br />
                    <br />
                    This cannot be undone. All schedule configuration and history will be removed.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel className="font-mono text-xs uppercase tracking-wider">
                    Cancel
                  </AlertDialogCancel>
                  <AlertDialogAction
                    onClick={handleDelete}
                    className="bg-destructive text-destructive-foreground hover:bg-destructive/90 font-mono text-xs uppercase tracking-wider"
                  >
                    {deleteSchedule.isPending ? (
                      <>
                        <Icon icon={Clock01Icon} className="size-3 mr-2 animate-spin" />
                        Deleting...
                      </>
                    ) : (
                      <>
                        <Icon icon={Trash} className="size-3 mr-2" />
                        Delete
                      </>
                    )}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        </div>
      </div>

      <div className="p-6 md:p-12 max-w-[1600px] mx-auto w-full space-y-12">
        {/* Schedule Details */}
        <div className="grid grid-cols-2 md:grid-cols-4 gap-px bg-border border border-border overflow-hidden rounded-lg">
          <MetadataItem
            label="Schedule ID"
            value={schedule.schedule?.scheduleId ?? ""}
            copyable
            mono
            className="bg-background"
          />
          <MetadataItem
            label="Workflow Type"
            value={schedule.schedule?.workflowType ?? ""}
            className="bg-background"
          />
          <MetadataItem
            label="Task Queue"
            value={schedule.schedule?.taskQueue ?? ""}
            mono
            className="bg-background"
          />
          <MetadataItem
            label="Version"
            value={schedule.schedule?.version ?? ""}
            mono
            className="bg-background"
          />
          <MetadataItem
            label="Cron Expression"
            value={schedule.schedule?.cronExpr ?? ""}
            mono
            className="bg-background"
          />
          <MetadataItem
            label="Status"
            value={schedule.schedule?.enabled ? "Active" : "Paused"}
            className="bg-background"
          />
          <MetadataItem
            label="Max Catchup"
            value={schedule.schedule?.maxCatchup.toString() ?? ""}
            className="bg-background"
          />
          <MetadataItem
            label="Total Runs"
            value={schedule.schedule?.totalRuns.toString() ?? ""}
            className="bg-background"
          />
          <MetadataItem
            label="Successful Runs"
            value={schedule.schedule?.successfulRuns.toString() ?? ""}
            className="bg-background"
          />
          <MetadataItem
            label="Failed Runs"
            value={schedule.schedule?.failedRuns.toString() ?? ""}
            className="bg-background"
          />
          <MetadataItem
            label="Created At"
            value={formatDateTime(schedule.schedule?.createdAt?.toDate())}
            className="bg-background"
          />
          <MetadataItem
            label="Updated At"
            value={formatDateTime(schedule.schedule?.updatedAt?.toDate())}
            className="bg-background"
          />
          <MetadataItem
            label="Next Run"
            value={formatDateTime(schedule.schedule?.nextFireAt?.toDate())}
            className="bg-background"
          />
          <MetadataItem
            label="Last Run"
            value={formatDateTime(schedule.schedule?.lastFiredAt?.toDate())}
            className="bg-background"
          />
        </div>

        {/* Input Parameters */}
        {schedule.schedule?.input && (
          <div className="space-y-3">
            <div className="flex items-center justify-between">
              <h3 className="font-mono text-xs uppercase tracking-widest text-muted-foreground">
                Input Parameters
              </h3>
              <Badge variant="outline" className="font-mono text-[10px]">
                JSON
              </Badge>
            </div>
            <div className="border border-border bg-muted/20 p-4 font-mono text-xs overflow-auto max-h-[300px] rounded-sm">
              <CodeBlock data={schedule.schedule?.input?.data} />
            </div>
          </div>
        )}

        {/* Execution History */}
        <div className="space-y-4">
          <div className="flex items-center justify-between">
            <h3 className="font-mono text-xs uppercase tracking-widest text-muted-foreground">
              Execution History
            </h3>
            <Badge variant="outline" className="font-mono text-[10px]">
              {runs.length} Runs
            </Badge>
          </div>

          {runsLoading ? (
            <div className="space-y-2">
              {[...Array(5)].map((_, i) => (
                <div
                  key={i}
                  className="h-16 bg-muted/20 animate-pulse w-full border border-border/50"
                />
              ))}
            </div>
          ) : runs.length === 0 ? (
            <div className="border-2 border-dashed border-border/50 rounded-lg bg-muted/5 p-12 text-center">
              <Icon icon={Clock01Icon} className="size-12 mx-auto mb-4 text-muted-foreground/30" />
              <p className="font-mono text-xs text-muted-foreground uppercase tracking-widest">
                No execution history
              </p>
            </div>
          ) : (
            <div className="border border-border bg-background shadow-sm overflow-hidden rounded-lg">
              <table className="w-full text-sm text-left">
                <thead className="bg-muted/30 text-muted-foreground font-mono text-[10px] uppercase tracking-wider border-b border-border">
                  <tr>
                    <th className="px-6 py-4 w-16 text-center">#</th>
                    <th className="px-6 py-4">Status</th>
                    <th className="px-6 py-4">Run ID</th>
                    <th className="px-6 py-4">Started</th>
                    <th className="px-6 py-4 text-right">Duration</th>
                    <th className="px-6 py-4 text-center">Actions</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border/50">
                  {runs.map((run, idx) => (
                    <tr
                      key={run.runId}
                      className="group hover:bg-muted/20 transition-all duration-200"
                    >
                      <td className="px-6 py-4 font-mono text-xs text-muted-foreground text-center">
                        {(idx + 1).toString().padStart(2, "0")}
                      </td>
                      <td className="px-6 py-4">
                        <Badge
                          variant="outline"
                          className={cn(
                            "rounded-full px-2.5 py-0.5 text-[10px] font-mono uppercase tracking-wider border bg-transparent",
                            getStatusColor(run.status),
                          )}
                        >
                          {WorkflowStatusLabel[run.status]}
                        </Badge>
                      </td>
                      <td className="px-6 py-4">
                        <button
                          onClick={() =>
                            navigate({
                              to: "/$namespaceId/workflows/$id",
                              params: { namespaceId, id: run.runId },
                            })
                          }
                          className="font-mono text-xs text-muted-foreground group-hover:text-primary transition-colors hover:underline"
                        >
                          {run.runId.slice(0, 8)}
                          <span className="opacity-30">{run.runId.slice(8, 18)}</span>
                        </button>
                      </td>
                      <td className="px-6 py-4 font-mono text-xs text-muted-foreground">
                        {formatDateTime(run.createdAt?.toDate())}
                      </td>
                      <td className="px-6 py-4 text-right font-mono text-xs font-medium">
                        {formatDuration(run.createdAt?.toDate(), run.finishedAt?.toDate())}
                      </td>
                      <td className="px-6 py-4 text-center">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() =>
                            navigate({
                              to: "/$namespaceId/workflows/$id",
                              params: { namespaceId, id: run.runId },
                            })
                          }
                          className="font-mono text-xs uppercase tracking-wider"
                        >
                          View
                        </Button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function MetadataItem({
  label,
  value,
  copyable,
  mono,
  className,
}: {
  label: string;
  value: string;
  copyable?: boolean;
  mono?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("p-6 hover:bg-muted/20 transition-colors group", className)}>
      <dt className="text-[10px] font-bold text-muted-foreground uppercase tracking-widest mb-2 flex items-center gap-2">
        {label}
      </dt>
      <dd className="flex items-center gap-3">
        <span
          className={cn(
            "text-sm text-foreground truncate block max-w-[200px]",
            mono ? "font-mono text-xs" : "font-medium",
          )}
        >
          {value || "-"}
        </span>
        {copyable && value && (
          <button className="opacity-0 group-hover:opacity-100 transition-opacity text-muted-foreground hover:text-primary">
            <Icon icon={Copy01Icon} className="size-3" />
          </button>
        )}
      </dd>
    </div>
  );
}

function CodeBlock({ data }: { data?: string | Uint8Array }) {
  if (!data)
    return <span className="text-muted-foreground/50 italic font-mono text-[10px]">NULL_DATA</span>;

  let dataStr: string;
  if (data instanceof Uint8Array) {
    dataStr = new TextDecoder().decode(data);
  } else {
    dataStr = data;
  }

  let content = dataStr;
  try {
    const parsed = JSON.parse(atob(dataStr));
    content = JSON.stringify(parsed, null, 2);
  } catch {
    try {
      content = JSON.stringify(JSON.parse(dataStr), null, 2);
    } catch {}
  }

  return <pre className="text-foreground/80">{content}</pre>;
}
