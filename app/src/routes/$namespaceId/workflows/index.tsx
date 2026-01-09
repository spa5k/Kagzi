import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet";
import type { ListWorkflowTypesRequest } from "@/gen/admin_pb";
import type { PageRequest } from "@/gen/common_pb";
import { useWorkflows } from "@/hooks/use-dashboard";
import { useListWorkflowTypes, useStartWorkflow } from "@/hooks/use-grpc-services";
import { cn } from "@/lib/utils";
import { WorkflowStatus, WorkflowStatusLabel } from "@/types";
import { type Timestamp, timestampDate } from "@bufbuild/protobuf/wkt";
import {
  Alert01Icon,
  Check,
  FilterHorizontalIcon,
  FlashIcon,
  RefreshIcon,
  Search01Icon,
} from "@hugeicons/core-free-icons";
import type { IconSvgElement } from "@hugeicons/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";
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

function formatDateTime(timestamp: Timestamp | undefined) {
  if (!timestamp) return "-";
  return timestampDate(timestamp).toLocaleString("en-US", {
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
  const start = timestampDate(startTs).getTime();
  const end = endTs ? timestampDate(endTs).getTime() : Date.now();
  const diff = end - start;

  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

function getTimeRangeCutoff(timeRange: string): Date | undefined {
  const now = Date.now();
  switch (timeRange) {
    case "1h":
      return new Date(now - 60 * 60 * 1000);
    case "24h":
      return new Date(now - 24 * 60 * 60 * 1000);
    case "7d":
      return new Date(now - 7 * 24 * 60 * 60 * 1000);
    case "30d":
      return new Date(now - 30 * 24 * 60 * 60 * 1000);
    default:
      return undefined;
  }
}

export const Route = createFileRoute("/$namespaceId/workflows/")({
  validateSearch: (search: Record<string, unknown>) => {
    const timeRange = search.timeRange as string;
    const validTimeRanges = ["1h", "24h", "7d", "30d"];
    return {
      status: (search.status as string) || "all",
      timeRange: validTimeRanges.includes(timeRange) ? timeRange : "24h",
    };
  },
  component: WorkflowsPage,
});

function WorkflowsPage() {
  const { namespaceId } = Route.useParams();
  const { status, timeRange } = useSearch({ from: "/$namespaceId/workflows/" });
  const navigate = useNavigate({ from: "/$namespaceId/workflows/" });
  const namespace = namespaceId;

  const {
    data: workflows,
    isLoading,
    error,
    refetch,
  } = useWorkflows(status === "all" ? undefined : status);

  // Fetch available workflow types
  const { data: workflowTypesData } = useListWorkflowTypes({
    namespace: namespace,
    page: {
      pageSize: 100,
      pageToken: "",
    },
  });
  const workflowTypes = workflowTypesData?.workflowTypes ?? [];

  // State for the new workflow form
  const [isSheetOpen, setIsSheetOpen] = useState(false);
  const [workflowType, setWorkflowType] = useState("");
  const [taskQueue, setTaskQueue] = useState("");
  const [externalId, setExternalId] = useState("");
  const [inputJson, setInputJson] = useState("{}");
  const [searchQuery, setSearchQuery] = useState("");

  // Mutation for starting a workflow
  const startWorkflow = useStartWorkflow();

  const handleStartWorkflow = async () => {
    try {
      const result = await startWorkflow.mutateAsync({
        namespace: namespace,
        workflowType,
        taskQueue,
        externalId: externalId || undefined,
        input: { data: new TextEncoder().encode(JSON.stringify(inputJson)) },
      });

      // Close the sheet and navigate to the new workflow
      setIsSheetOpen(false);
      navigate({ to: "/$namespaceId/workflows/$id", params: { namespaceId, id: result.runId } });

      // Reset form
      setWorkflowType("");
      setTaskQueue("");
      setExternalId("");
      setInputJson("{}");
    } catch (error) {
      console.error("Failed to start workflow:", error);
    }
  };

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background font-mono text-xs text-destructive">
        <div className="border border-destructive/20 bg-destructive/5 p-8 max-w-md w-full">
          <div className="flex items-center gap-2 mb-4 text-destructive">
            <Icon icon={Alert01Icon} className="size-5" />
            <h3 className="font-bold uppercase tracking-widest">System Failure</h3>
          </div>
          <p className="mb-6 opacity-80">
            {error.message || "Failed to retrieve workflow processes."}
          </p>
          <Button
            onClick={() => refetch()}
            variant="outline"
            className="w-full font-mono text-xs uppercase tracking-wider border-destructive text-destructive hover:bg-destructive hover:text-white"
          >
            Retry Connection
          </Button>
        </div>
      </div>
    );
  }

  const filteredWorkflows =
    workflows?.filter((w) => {
      // Status filter
      if (status === "running" && w.status !== WorkflowStatus.RUNNING) return false;
      if (status === "failed" && w.status !== WorkflowStatus.FAILED) return false;
      if (status === "completed" && w.status !== WorkflowStatus.COMPLETED) return false;

      // Search filter
      if (searchQuery) {
        const query = searchQuery.toLowerCase();
        const matchesRunId = w.runId.toLowerCase().includes(query);
        const matchesType = w.workflowType.toLowerCase().includes(query);
        const matchesTaskQueue = w.taskQueue.toLowerCase().includes(query);
        if (!matchesRunId && !matchesType && !matchesTaskQueue) return false;
      }

      // Time range filter
      const cutoff = getTimeRangeCutoff(timeRange);
      if (cutoff && w.createdAt) {
        const createdAt = timestampDate(w.createdAt).getTime();
        if (createdAt < cutoff.getTime()) return false;
      }

      return true;
    }) ?? [];

  return (
    <div className="flex min-h-screen bg-background text-foreground">
      <div className="w-64 border-r border-border bg-background/50 hidden md:flex flex-col sticky top-0 h-screen">
        <div className="p-6 border-b border-border">
          <h2 className="font-mono text-xs font-bold uppercase tracking-widest text-muted-foreground mb-6 flex items-center gap-2">
            <Icon icon={FilterHorizontalIcon} className="size-3" />
            Process Filters
          </h2>
          <nav className="space-y-1">
            <FilterButton
              label="All Processes"
              active={status === "all"}
              count={workflows?.length ?? 0}
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "all" }) })}
            />
            <FilterButton
              label="Running"
              active={status === "running"}
              count={workflows?.filter((w) => w.status === WorkflowStatus.RUNNING).length ?? 0}
              variant="primary"
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "running" }) })}
            />
            <FilterButton
              label="Completed"
              active={status === "completed"}
              count={workflows?.filter((w) => w.status === WorkflowStatus.COMPLETED).length ?? 0}
              variant="primary"
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "completed" }) })}
            />
            <FilterButton
              label="Failed"
              active={status === "failed"}
              count={workflows?.filter((w) => w.status === WorkflowStatus.FAILED).length ?? 0}
              variant="destructive"
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "failed" }) })}
            />
          </nav>
        </div>

        <div className="p-6 mt-auto border-t border-border">
          <div className="text-[10px] font-mono text-muted-foreground uppercase tracking-wider space-y-2">
            <div className="flex justify-between">
              <span>System Status</span>
              <span className="text-primary">Operational</span>
            </div>
            <div className="flex justify-between">
              <span>Last Sync</span>
              <span>{new Date().toLocaleTimeString()}</span>
            </div>
          </div>
        </div>
      </div>

      <div className="flex-1 flex flex-col min-w-0">
        <div className="border-b border-border p-6 md:p-8 flex justify-between items-end bg-background sticky top-0 z-10">
          <div>
            <div className="flex items-center gap-3 mb-2">
              <div className="size-2 bg-primary animate-pulse" />
              <span className="font-mono text-xs tracking-widest text-muted-foreground uppercase">
                Process Monitor
              </span>
            </div>
            <h1 className="text-4xl font-black tracking-tighter uppercase leading-none">
              Active Workflows<span className="text-primary">.</span>
            </h1>
          </div>
          <Sheet open={isSheetOpen} onOpenChange={setIsSheetOpen}>
            <SheetTrigger
              disabled={isLoading}
              className={cn(
                "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors",
                "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                "disabled:pointer-events-none disabled:opacity-50",
                "bg-primary text-primary-foreground hover:bg-primary/90",
                "h-10 px-6 py-2",
                "font-mono text-xs uppercase tracking-wider",
                "shadow-[4px_4px_0_0_rgba(0,0,0,1)] dark:shadow-[4px_4px_0_0_rgba(255,255,255,0.2)] active:translate-x-[2px] active:translate-y-[2px] active:shadow-none transition-all",
              )}
            >
              <HugeiconsIcon icon={FlashIcon} className="size-3" />
              Initialize Process
            </SheetTrigger>
            <SheetContent className="w-full sm:max-w-md" side="right">
              <SheetHeader>
                <SheetTitle className="font-mono text-sm uppercase tracking-wider">
                  Initialize New Process
                </SheetTitle>
                <SheetDescription className="font-mono text-xs">
                  Start a new workflow execution by specifying the workflow type and input
                  parameters.
                </SheetDescription>
              </SheetHeader>

              <div className="flex-1 py-6 space-y-4 overflow-y-auto">
                <div className="space-y-2">
                  <Label
                    htmlFor="workflow-type"
                    className="font-mono text-xs uppercase tracking-wider"
                  >
                    Workflow Type <span className="text-destructive">*</span>
                  </Label>
                  <Input
                    id="workflow-type"
                    placeholder="e.g., example-workflow"
                    value={workflowType}
                    onChange={(e) => setWorkflowType(e.target.value)}
                    className="font-mono text-sm"
                    list="workflow-types"
                  />
                  <datalist id="workflow-types">
                    {workflowTypes.map((type) => (
                      <option key={type.workflowType} value={type.workflowType}>
                        {type.workflowType}
                      </option>
                    ))}
                  </datalist>
                </div>

                <div className="space-y-2">
                  <Label
                    htmlFor="task-queue"
                    className="font-mono text-xs uppercase tracking-wider"
                  >
                    Task Queue <span className="text-destructive">*</span>
                  </Label>
                  <Input
                    id="task-queue"
                    placeholder="e.g., main-queue"
                    value={taskQueue}
                    onChange={(e) => setTaskQueue(e.target.value)}
                    className="font-mono text-sm"
                  />
                </div>

                <div className="space-y-2">
                  <Label
                    htmlFor="external-id"
                    className="font-mono text-xs uppercase tracking-wider"
                  >
                    External ID (Optional)
                  </Label>
                  <Input
                    id="external-id"
                    placeholder="e.g., unique-request-id-123"
                    value={externalId}
                    onChange={(e) => setExternalId(e.target.value)}
                    className="font-mono text-sm"
                  />
                  <p className="text-[10px] text-muted-foreground font-mono">
                    Unique identifier for idempotency. If omitted, a random ID will be generated.
                  </p>
                </div>

                <div className="space-y-2">
                  <Label
                    htmlFor="input-json"
                    className="font-mono text-xs uppercase tracking-wider"
                  >
                    Input Parameters (JSON)
                  </Label>
                  <textarea
                    id="input-json"
                    value={inputJson}
                    onChange={(e) => setInputJson(e.target.value)}
                    className="flex min-h-[120px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 font-mono"
                    placeholder='{"key": "value"}'
                    spellCheck={false}
                  />
                  <p className="text-[10px] text-muted-foreground font-mono">
                    Valid JSON object to pass as input to the workflow.
                  </p>
                </div>
              </div>

              <SheetFooter>
                <SheetClose
                  className={cn(
                    "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium transition-colors",
                    "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                    "disabled:pointer-events-none disabled:opacity-50",
                    "border border-input bg-background shadow-sm hover:bg-accent hover:text-accent-foreground",
                    "h-9 px-4 py-2",
                    "font-mono text-xs uppercase tracking-wider",
                  )}
                >
                  Cancel
                </SheetClose>
                <Button
                  onClick={handleStartWorkflow}
                  disabled={startWorkflow.isPending || !workflowType || !taskQueue}
                  className="bg-primary text-primary-foreground hover:bg-primary/90 font-mono text-xs uppercase tracking-wider"
                >
                  {startWorkflow.isPending ? (
                    <>
                      <Icon icon={RefreshIcon} className="size-3 mr-2 animate-spin" />
                      Starting...
                    </>
                  ) : (
                    <>
                      <Icon icon={Check} className="size-3 mr-2" />
                      Start Process
                    </>
                  )}
                </Button>
              </SheetFooter>
            </SheetContent>
          </Sheet>
        </div>

        <div className="p-6 md:p-8 bg-muted/5 min-h-full">
          <div className="flex items-center gap-4 mb-6">
            <div className="relative flex-1 max-w-md group">
              <Icon
                icon={Search01Icon}
                className="absolute left-3 top-1/2 -translate-y-1/2 size-4 text-muted-foreground group-hover:text-primary transition-colors"
              />
              <Input
                placeholder="Search Process ID / Type..."
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="pl-10 h-10 bg-background border-border rounded-none focus-visible:ring-1 focus-visible:ring-primary font-mono text-xs"
              />
            </div>
            <div className="flex items-center gap-2 ml-auto">
              <Select
                value={timeRange}
                onValueChange={(value) =>
                  navigate({ search: (prev) => ({ ...prev, timeRange: value as string }) })
                }
              >
                <SelectTrigger className="h-10 w-[140px] font-mono text-[10px] uppercase tracking-wider border-border">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="1h">Last 1 Hour</SelectItem>
                  <SelectItem value="24h">Last 24 Hours</SelectItem>
                  <SelectItem value="7d">Last 7 Days</SelectItem>
                  <SelectItem value="30d">Last 30 Days</SelectItem>
                </SelectContent>
              </Select>
              <Button
                variant="outline"
                size="icon"
                className="size-10 rounded-none border-border hover:border-primary hover:text-primary"
                onClick={() => refetch()}
                disabled={isLoading}
              >
                <Icon icon={RefreshIcon} className="size-4" />
              </Button>
            </div>
          </div>

          {isLoading ? (
            <div className="space-y-2">
              {[...Array(8)].map((_, i) => (
                <div
                  key={i}
                  className="h-16 bg-muted/20 animate-pulse w-full border border-border/50"
                />
              ))}
            </div>
          ) : (
            <div className="border border-border bg-background shadow-sm">
              <table className="w-full text-sm text-left">
                <thead className="bg-muted/30 text-muted-foreground font-mono text-[10px] uppercase tracking-wider border-b border-border">
                  <tr>
                    <th className="px-6 py-4 w-16 text-center">#</th>
                    <th className="px-6 py-4">Process Status</th>
                    <th className="px-6 py-4">Workflow Type</th>
                    <th className="px-6 py-4">Run Identifier</th>
                    <th className="px-6 py-4">Started</th>
                    <th className="px-6 py-4 text-right">Duration</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-border/50">
                  {filteredWorkflows.map((workflow, idx) => (
                    <tr
                      key={workflow.runId}
                      className="group hover:bg-muted/20 cursor-pointer transition-all duration-200"
                      onClick={() =>
                        navigate({
                          to: "/$namespaceId/workflows/$id",
                          params: { namespaceId, id: workflow.runId },
                        })
                      }
                    >
                      <td className="px-6 py-4 font-mono text-xs text-muted-foreground text-center">
                        {(idx + 1).toString().padStart(2, "0")}
                      </td>
                      <td className="px-6 py-4">
                        <Badge
                          variant="outline"
                          className={cn(
                            "rounded-full px-2.5 py-0.5 text-[10px] font-mono uppercase tracking-wider border bg-transparent",
                            getStatusColor(workflow.status),
                          )}
                        >
                          {WorkflowStatusLabel[workflow.status]}
                        </Badge>
                      </td>
                      <td className="px-6 py-4 font-bold tracking-tight text-sm group-hover:text-primary transition-colors uppercase">
                        {workflow.workflowType}
                      </td>
                      <td className="px-6 py-4 font-mono text-xs text-muted-foreground group-hover:text-foreground transition-colors">
                        {workflow.runId.slice(0, 8)}
                        <span className="opacity-30">{workflow.runId.slice(8, 18)}</span>
                      </td>
                      <td className="px-6 py-4 font-mono text-xs text-muted-foreground">
                        {formatDateTime(workflow.createdAt)}
                      </td>
                      <td className="px-6 py-4 text-right font-mono text-xs font-medium">
                        {formatDuration(workflow.createdAt, workflow.finishedAt)}
                      </td>
                    </tr>
                  ))}
                  {filteredWorkflows.length === 0 && (
                    <tr>
                      <td colSpan={6} className="px-6 py-24 text-center">
                        <div className="flex flex-col items-center gap-3 text-muted-foreground">
                          <Icon icon={FilterHorizontalIcon} className="size-8 opacity-20" />
                          <p className="font-mono text-xs uppercase tracking-widest">
                            No matching processes found
                          </p>
                        </div>
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function FilterButton({
  label,
  active,
  count,
  onClick,
  variant = "default",
}: {
  label: string;
  active?: boolean;
  count?: number;
  onClick: () => void;
  variant?: "default" | "primary" | "destructive";
}) {
  const activeStyles = {
    default: "bg-muted text-foreground border-l-2 border-foreground",
    primary: "bg-primary/5 text-primary border-l-2 border-primary",
    destructive: "bg-destructive/5 text-destructive border-l-2 border-destructive",
  };

  return (
    <button
      onClick={onClick}
      className={cn(
        "w-full flex items-center justify-between px-4 py-3 text-xs uppercase tracking-wider transition-all border-l-2 border-transparent",
        active
          ? activeStyles[variant]
          : "text-muted-foreground hover:bg-muted/30 hover:text-foreground",
      )}
    >
      <span className="font-medium">{label}</span>
      {count !== undefined && (
        <span className="font-mono text-[10px] opacity-70">
          {count.toString().padStart(2, "0")}
        </span>
      )}
    </button>
  );
}
