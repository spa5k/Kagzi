import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useWorkflows } from "@/hooks/use-dashboard";
import { cn } from "@/lib/utils";
import { WorkflowStatus, WorkflowStatusLabel } from "@/types";
import { Timestamp } from "@bufbuild/protobuf";
import {
  Alert01Icon,
  Clock01Icon,
  FilterHorizontalIcon,
  FlashIcon,
  RefreshIcon,
  Search01Icon,
} from "@hugeicons/core-free-icons";
import type { IconSvgElement } from "@hugeicons/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { createFileRoute, useNavigate, useSearch } from "@tanstack/react-router";

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

export const Route = createFileRoute("/workflows/")({
  validateSearch: (search: Record<string, unknown>) => ({
    status: (search.status as string) || "all",
    timeRange: (search.timeRange as string) || "24h",
  }),
  component: WorkflowsPage,
});

function WorkflowsPage() {
  const { status } = useSearch({ from: "/workflows/" });
  const navigate = useNavigate({ from: "/workflows/" });

  const {
    data: workflows,
    isLoading,
    error,
    refetch,
  } = useWorkflows(status === "all" ? undefined : status);

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
      if (status === "running") return w.status === WorkflowStatus.RUNNING;
      if (status === "failed") return w.status === WorkflowStatus.FAILED;
      if (status === "completed") return w.status === WorkflowStatus.COMPLETED;
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
          <Button
            className="bg-primary text-primary-foreground hover:bg-primary/90 rounded-none h-10 px-6 font-mono text-xs uppercase tracking-wider shadow-[4px_4px_0_0_rgba(0,0,0,1)] dark:shadow-[4px_4px_0_0_rgba(255,255,255,0.2)] active:translate-x-[2px] active:translate-y-[2px] active:shadow-none transition-all"
            disabled={isLoading}
          >
            <Icon icon={FlashIcon} className="size-3 mr-2" />
            Initialize Process
          </Button>
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
                className="pl-10 h-10 bg-background border-border rounded-none focus-visible:ring-1 focus-visible:ring-primary font-mono text-xs"
              />
            </div>
            <div className="flex items-center gap-2 ml-auto">
              <div className="h-8 px-3 flex items-center gap-2 bg-background border border-border text-[10px] font-mono text-muted-foreground uppercase tracking-wider">
                <Icon icon={Clock01Icon} className="size-3" />
                <span>24H Window</span>
              </div>
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
                        navigate({ to: "/workflows/$id", params: { id: workflow.runId } })
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
