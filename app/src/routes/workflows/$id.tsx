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
import { ListStepsRequest } from "@/gen/admin_pb";
import { StepStatus } from "@/gen/worker_pb";
import type { Workflow } from "@/gen/workflow_pb";
import {
  CancelWorkflowRequest,
  RetryWorkflowRequest,
  TerminateWorkflowRequest,
} from "@/gen/workflow_pb";
import { useWorkflows } from "@/hooks/use-dashboard";
import {
  useCancelWorkflow,
  useListSteps,
  useRetryWorkflow,
  useTerminateWorkflow,
} from "@/hooks/use-grpc-services";
import { cn } from "@/lib/utils";
import { WorkflowStatus, WorkflowStatusLabel } from "@/types";
import { Timestamp } from "@bufbuild/protobuf";
import {
  Alert01Icon,
  ArrowLeft01Icon,
  Cancel01Icon,
  CheckmarkCircle01Icon,
  Clock01Icon,
  Copy01Icon,
  Menu01Icon,
  Refresh01Icon,
  StopIcon,
  ViewIcon,
} from "@hugeicons/core-free-icons";
import type { IconSvgElement } from "@hugeicons/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";

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
    case WorkflowStatus.SLEEPING:
    case WorkflowStatus.PAUSED:
      return "text-blue-500 bg-blue-500/10 border-blue-500/20";
    default:
      return "text-muted-foreground bg-muted/50 border-border";
  }
}

function getStatusIcon(status: number) {
  switch (status) {
    case WorkflowStatus.RUNNING:
      return (
        <div className="size-2 rounded-full bg-primary animate-pulse shadow-[0_0_8px] shadow-primary/40" />
      );
    case WorkflowStatus.COMPLETED:
      return <Icon icon={CheckmarkCircle01Icon} className="size-4 text-primary" />;
    case WorkflowStatus.FAILED:
      return <Icon icon={Alert01Icon} className="size-4 text-destructive" />;
    case WorkflowStatus.CANCELLED:
      return <Icon icon={Cancel01Icon} className="size-4 text-muted-foreground" />;
    default:
      return <Icon icon={Clock01Icon} className="size-4 text-muted-foreground" />;
  }
}

function getStepStatusColor(status: number) {
  switch (status) {
    case StepStatus.RUNNING:
      return "text-primary bg-primary/10 border-primary/20";
    case StepStatus.COMPLETED:
      return "text-primary bg-primary/10 border-primary/20";
    case StepStatus.FAILED:
      return "text-destructive bg-destructive/10 border-destructive/20";
    case StepStatus.PENDING:
      return "text-yellow-600 bg-yellow-600/10 border-yellow-600/20";
    default:
      return "text-muted-foreground bg-muted/50 border-border";
  }
}

function formatDateTime(timestamp: Timestamp | undefined) {
  if (!timestamp) return "-";
  return new Date(timestamp.toDate()).toLocaleString("en-US", {
    month: "numeric",
    day: "numeric",
    hour: "numeric",
    minute: "numeric",
    second: "numeric",
    hour12: false,
  });
}

function formatDuration(startTs?: Timestamp, endTs?: Timestamp) {
  if (!startTs) return "-";
  const start = new Date(startTs.toDate()).getTime();
  const end = endTs ? new Date(endTs.toDate()).getTime() : Date.now();
  const diff = end - start;

  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export const Route = createFileRoute("/workflows/$id")({
  component: WorkflowDetailPage,
});

function WorkflowDetailPage() {
  const { id } = Route.useParams();
  const navigate = useNavigate();
  const { data: workflows } = useWorkflows();
  const terminateWorkflow = useTerminateWorkflow();
  const cancelWorkflow = useCancelWorkflow();
  const retryWorkflow = useRetryWorkflow();

  const workflow = workflows?.find((w) => w.runId === id);

  const handleTerminate = async () => {
    try {
      await terminateWorkflow.mutateAsync(
        new TerminateWorkflowRequest({
          runId: id,
          namespaceId: "default",
        }),
      );
    } catch (error) {
      console.error("Failed to terminate workflow:", error);
    }
  };

  const handleCancel = async () => {
    try {
      await cancelWorkflow.mutateAsync(
        new CancelWorkflowRequest({
          runId: id,
          namespaceId: "default",
        }),
      );
    } catch (error) {
      console.error("Failed to cancel workflow:", error);
    }
  };

  const handleRetry = async () => {
    try {
      await retryWorkflow.mutateAsync(
        new RetryWorkflowRequest({
          runId: id,
          namespaceId: "default",
        }),
      );
    } catch (error) {
      console.error("Failed to retry workflow:", error);
    }
  };

  if (!workflow) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background font-mono text-xs text-destructive">
        <div className="border border-destructive/20 bg-destructive/5 p-8 max-w-md w-full">
          <div className="flex items-center gap-2 mb-4 text-destructive">
            <Icon icon={Alert01Icon} className="size-5" />
            <h3 className="font-bold uppercase tracking-widest">Process Not Found</h3>
          </div>
          <p className="mb-6 opacity-80">The requested workflow could not be found.</p>
          <Button
            onClick={() =>
              navigate({ to: "/workflows", search: { status: "all", timeRange: "24h" } })
            }
            variant="outline"
            className="w-full font-mono text-xs uppercase tracking-wider border-destructive text-destructive hover:bg-destructive hover:text-white"
          >
            Return to Index
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background text-foreground animate-in fade-in duration-200">
      <div className="sticky top-0 z-10 border-b border-border bg-background/80 backdrop-blur-md">
        <div className="flex items-center justify-between px-6 py-4 max-w-[1800px] mx-auto w-full">
          <div className="flex items-center gap-6">
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                navigate({ to: "/workflows", search: { status: "all", timeRange: "24h" } })
              }
              className="font-mono text-xs uppercase tracking-wider text-muted-foreground hover:text-primary -ml-2 gap-2"
            >
              <Icon icon={ArrowLeft01Icon} className="size-4" />
              Return to Index
            </Button>
            <div className="h-6 w-px bg-border" />
            <div className="flex items-center gap-3">
              <span className="font-bold text-lg tracking-tight uppercase">
                {workflow.workflowType}
              </span>
              <Badge
                variant="outline"
                className={cn(
                  "gap-1.5 pl-1.5 pr-2.5 py-0.5 rounded-full text-[10px] font-mono uppercase tracking-widest",
                  getStatusColor(workflow.status),
                )}
              >
                {getStatusIcon(workflow.status)}
                {WorkflowStatusLabel[workflow.status]}
              </Badge>
            </div>
          </div>

          <div className="flex items-center gap-3">
            {/* Cancel button - only for running workflows */}
            {workflow.status === WorkflowStatus.RUNNING && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleCancel}
                disabled={cancelWorkflow.isPending || terminateWorkflow.isPending}
                className="font-mono text-xs uppercase tracking-wider"
              >
                <Icon icon={StopIcon} className="size-3 mr-2" />
                {cancelWorkflow.isPending ? "Cancelling..." : "Cancel"}
              </Button>
            )}

            {/* Retry button - only for failed workflows */}
            {workflow.status === WorkflowStatus.FAILED && (
              <Button
                variant="outline"
                size="sm"
                onClick={handleRetry}
                disabled={retryWorkflow.isPending}
                className="font-mono text-xs uppercase tracking-wider"
              >
                <Icon icon={Refresh01Icon} className="size-3 mr-2" />
                {retryWorkflow.isPending ? "Retrying..." : "Retry"}
              </Button>
            )}

            <AlertDialog>
              <AlertDialogTrigger
                disabled={
                  !workflow ||
                  workflow.status === WorkflowStatus.COMPLETED ||
                  workflow.status === WorkflowStatus.FAILED ||
                  workflow.status === WorkflowStatus.CANCELLED ||
                  terminateWorkflow.isPending ||
                  cancelWorkflow.isPending
                }
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
                <HugeiconsIcon icon={Cancel01Icon} className="size-3" />
                Terminate Process
              </AlertDialogTrigger>
              <AlertDialogContent size="default">
                <AlertDialogHeader>
                  <AlertDialogMedia>
                    <Icon icon={Alert01Icon} className="size-6 text-destructive" />
                  </AlertDialogMedia>
                  <AlertDialogTitle className="font-mono text-sm uppercase tracking-wider">
                    Terminate Process?
                  </AlertDialogTitle>
                  <AlertDialogDescription className="font-mono text-xs">
                    This action will forcefully terminate the workflow execution{" "}
                    <span className="font-bold text-foreground">{workflow?.runId.slice(0, 8)}</span>
                    {"."}
                    <br />
                    <br />
                    This cannot be undone. The process will be marked as terminated.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel className="font-mono text-xs uppercase tracking-wider">
                    Cancel
                  </AlertDialogCancel>
                  <AlertDialogAction
                    onClick={handleTerminate}
                    className="bg-destructive text-destructive-foreground hover:bg-destructive/90 font-mono text-xs uppercase tracking-wider"
                  >
                    {terminateWorkflow.isPending ? (
                      <>
                        <Icon icon={Clock01Icon} className="size-3 mr-2 animate-spin" />
                        Terminating...
                      </>
                    ) : (
                      <>
                        <Icon icon={Cancel01Icon} className="size-3 mr-2" />
                        Terminate
                      </>
                    )}
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
            <Button variant="outline" size="icon" className="size-8 rounded-full">
              <Icon icon={Menu01Icon} className="size-4" />
            </Button>
          </div>
        </div>
      </div>

      <div className="p-6 md:p-12 max-w-[1600px] mx-auto w-full">
        <WorkflowDetailContent workflow={workflow} />
      </div>
    </div>
  );
}

function WorkflowDetailContent({ workflow }: { workflow: Workflow }) {
  const [activeTab, setActiveTab] = useState("summary");

  // Fetch workflow steps for the history tab
  const { data: stepsData, isLoading: stepsLoading } = useListSteps(
    new ListStepsRequest({
      runId: workflow.runId,
      namespaceId: "default",
    }),
  );
  const steps = stepsData?.steps ?? [];

  return (
    <div className="space-y-12">
      <div className="grid grid-cols-2 md:grid-cols-4 gap-px bg-border border border-border overflow-hidden rounded-lg">
        <MetadataItem
          label="Run Identifier"
          value={workflow.runId}
          copyable
          mono
          className="bg-background"
        />
        <MetadataItem
          label="Process Type"
          value={workflow.workflowType}
          copyable
          className="bg-background"
        />
        <MetadataItem
          label="Target Queue"
          value={workflow.taskQueue}
          mono
          className="bg-background"
        />
        <MetadataItem
          label="Current Status"
          value={WorkflowStatusLabel[workflow.status]}
          className="bg-background"
        />
        <MetadataItem
          label="Initialized"
          value={formatDateTime(workflow.createdAt)}
          className="bg-background"
        />
        <MetadataItem
          label="Terminated"
          value={formatDateTime(workflow.finishedAt)}
          className="bg-background"
        />
        <MetadataItem
          label="Execution Time"
          value={formatDuration(workflow.createdAt, workflow.finishedAt)}
          className="bg-background"
        />
        <MetadataItem
          label="Parent Process"
          value={workflow.parentStepId || "Root Process"}
          mono={!!workflow.parentStepId}
          className="bg-background"
        />
      </div>

      <div>
        <div className="flex items-center gap-8 border-b border-border mb-8 overflow-x-auto">
          {["Summary", "History", "Relationships", "Workers", "Pending Activities"].map((tab) => {
            const id = tab.toLowerCase().replace(" ", "-");
            return (
              <TabButton
                key={id}
                label={tab}
                active={activeTab === id}
                onClick={() => setActiveTab(id)}
              />
            );
          })}
        </div>

        {activeTab === "summary" && (
          <div className="grid gap-8 animate-in fade-in slide-in-from-bottom-4 duration-300">
            {workflow.error && (
              <div className="border border-destructive/50 bg-destructive/5 rounded-none p-6 relative overflow-hidden">
                <div className="absolute top-0 left-0 w-1 h-full bg-destructive" />
                <h3 className="font-bold text-destructive mb-2 flex items-center gap-2 uppercase tracking-widest text-sm">
                  <Icon icon={Alert01Icon} className="size-4" />
                  Process Failure Detected
                </h3>
                <div className="font-mono text-xs text-destructive/80 leading-relaxed break-all">
                  {workflow.error.message}
                </div>
              </div>
            )}

            <div className="grid md:grid-cols-2 gap-8">
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <h3 className="font-mono text-xs uppercase tracking-widest text-muted-foreground">
                    Input Parameters
                  </h3>
                  <Badge variant="outline" className="font-mono text-[10px]">
                    JSON
                  </Badge>
                </div>
                <div className="border border-border bg-muted/20 p-4 font-mono text-xs overflow-auto max-h-[500px] rounded-sm">
                  <CodeBlock data={workflow.input?.data} />
                </div>
              </div>
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <h3 className="font-mono text-xs uppercase tracking-widest text-muted-foreground">
                    Output Result
                  </h3>
                  <Badge variant="outline" className="font-mono text-[10px]">
                    JSON
                  </Badge>
                </div>
                <div className="border border-border bg-muted/20 p-4 font-mono text-xs overflow-auto max-h-[500px] rounded-sm">
                  <CodeBlock data={workflow.output?.data} />
                </div>
              </div>
            </div>
          </div>
        )}

        {activeTab === "history" && (
          <div className="animate-in fade-in slide-in-from-bottom-4 duration-300">
            {stepsLoading ? (
              <div className="space-y-2">
                {[...Array(5)].map((_, i) => (
                  <div
                    key={i}
                    className="h-24 bg-muted/20 animate-pulse w-full border border-border/50"
                  />
                ))}
              </div>
            ) : steps.length === 0 ? (
              <div className="border-2 border-dashed border-border/50 rounded-lg bg-muted/5 p-12 text-center">
                <Icon
                  icon={Clock01Icon}
                  className="size-12 mx-auto mb-4 text-muted-foreground/30"
                />
                <p className="font-mono text-xs text-muted-foreground uppercase tracking-widest">
                  No execution steps recorded
                </p>
              </div>
            ) : (
              <div className="border border-border bg-background shadow-sm overflow-hidden rounded-lg">
                <table className="w-full text-sm text-left">
                  <thead className="bg-muted/30 text-muted-foreground font-mono text-[10px] uppercase tracking-wider border-b border-border">
                    <tr>
                      <th className="px-6 py-4 w-16 text-center">#</th>
                      <th className="px-6 py-4">Step ID</th>
                      <th className="px-6 py-4">Step Name</th>
                      <th className="px-6 py-4">Status</th>
                      <th className="px-6 py-4">Started</th>
                      <th className="px-6 py-4 text-right">Duration</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border/50">
                    {steps.map((step, idx) => (
                      <tr
                        key={step.stepId}
                        className="group hover:bg-muted/20 transition-all duration-200"
                      >
                        <td className="px-6 py-4 font-mono text-xs text-muted-foreground text-center">
                          {(idx + 1).toString().padStart(2, "0")}
                        </td>
                        <td className="px-6 py-4">
                          <span className="font-mono text-xs text-muted-foreground">
                            {step.stepId.slice(0, 8)}
                            <span className="opacity-30">{step.stepId.slice(8, 16)}</span>
                          </span>
                        </td>
                        <td className="px-6 py-4 font-bold tracking-tight text-sm">
                          {step.name || <span className="text-muted-foreground">Unnamed Step</span>}
                        </td>
                        <td className="px-6 py-4">
                          <Badge
                            variant="outline"
                            className={cn(
                              "rounded-full px-2.5 py-0.5 text-[10px] font-mono uppercase tracking-wider border bg-transparent",
                              getStepStatusColor(step.status),
                            )}
                          >
                            {StepStatus[step.status] || "UNKNOWN"}
                          </Badge>
                        </td>
                        <td className="px-6 py-4 font-mono text-xs text-muted-foreground">
                          {formatDateTime(step.createdAt)}
                        </td>
                        <td className="px-6 py-4 text-right font-mono text-xs font-medium">
                          {formatDuration(step.createdAt, step.finishedAt)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        )}

        {activeTab !== "summary" && activeTab !== "history" && (
          <div className="py-24 text-center border-2 border-dashed border-border/50 rounded-lg bg-muted/5">
            <Icon icon={ViewIcon} className="size-12 mx-auto mb-4 text-muted-foreground/30" />
            <p className="font-mono text-xs text-muted-foreground uppercase tracking-widest">
              View Module Not Implemented
            </p>
          </div>
        )}
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

function TabButton({
  label,
  active,
  onClick,
}: {
  label: string;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "pb-4 text-xs font-bold uppercase tracking-widest border-b-2 transition-all px-2 whitespace-nowrap",
        active
          ? "border-primary text-primary"
          : "border-transparent text-muted-foreground hover:text-foreground hover:border-muted",
      )}
    >
      {label}
    </button>
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
    } catch {
      content = dataStr;
    }
  }

  return <pre className="text-foreground/80">{content}</pre>;
}
