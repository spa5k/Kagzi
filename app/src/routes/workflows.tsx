import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { mockWorkflows } from "@/lib/mock-data";
import { cn } from "@/lib/utils";
import { WorkflowStatus, WorkflowStatusLabel } from "@/types";
import {
  Alert01Icon,
  ArrowLeft01Icon,
  Cancel01Icon,
  CheckmarkCircle01Icon,
  Clock01Icon,
  Copy01Icon,
  FilterHorizontalIcon,
  FlashIcon,
  Menu01Icon,
  RefreshIcon,
  Search01Icon,
  ViewIcon,
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
      return "text-blue-500 bg-blue-500/10 border-blue-200 dark:border-blue-900";
    case WorkflowStatus.COMPLETED:
      return "text-green-600 bg-green-500/10 border-green-200 dark:border-green-900";
    case WorkflowStatus.FAILED:
      return "text-red-600 bg-red-500/10 border-red-200 dark:border-red-900";
    case WorkflowStatus.CANCELLED:
      return "text-gray-500 bg-gray-500/10 border-gray-200 dark:border-gray-800";
    case WorkflowStatus.SLEEPING:
    case WorkflowStatus.PAUSED:
      return "text-orange-600 bg-orange-500/10 border-orange-200 dark:border-orange-900";
    default:
      return "text-gray-500 bg-gray-100 dark:bg-gray-800 border-gray-200 dark:border-gray-700";
  }
}

function getStatusIcon(status: number) {
  switch (status) {
    case WorkflowStatus.RUNNING:
      return <div className="w-2 h-2 rounded-full bg-blue-500 animate-pulse" />;
    case WorkflowStatus.COMPLETED:
      return <Icon icon={CheckmarkCircle01Icon} className="w-4 h-4" />;
    case WorkflowStatus.FAILED:
      return <Icon icon={Alert01Icon} className="w-4 h-4" />;
    case WorkflowStatus.CANCELLED:
      return <Icon icon={Cancel01Icon} className="w-4 h-4" />;
    default:
      return <Icon icon={Clock01Icon} className="w-4 h-4" />;
  }
}

function formatDateTime(dateStr: string) {
  if (!dateStr) return "-";
  return new Date(dateStr).toLocaleString("en-US", {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "numeric",
    second: "numeric",
    hour12: true,
  });
}

function formatDuration(startStr?: string | null, endStr?: string | null) {
  if (!startStr) return "-";
  const start = new Date(startStr).getTime();
  const end = endStr ? new Date(endStr).getTime() : Date.now();
  const diff = end - start;

  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

export const Route = createFileRoute("/workflows")({
  validateSearch: (search: Record<string, unknown>) => ({
    selected: (search.selected as string) || null,
    status: (search.status as string) || "all",
    timeRange: (search.timeRange as string) || "24h",
  }),
  component: WorkflowsPage,
});

function WorkflowsPage() {
  const { selected, status } = useSearch({ from: "/workflows" });
  const navigate = useNavigate({ from: "/workflows" });
  const workflows = mockWorkflows;

  const filteredWorkflows = workflows.filter((w) => {
    if (status === "running") return w.status === WorkflowStatus.RUNNING;
    if (status === "failed") return w.status === WorkflowStatus.FAILED;
    if (status === "completed") return w.status === WorkflowStatus.COMPLETED;
    return true;
  });

  const selectedWorkflow = selected ? workflows.find((w) => w.runId === selected) : null;

  if (selectedWorkflow) {
    return (
      <div className="h-full flex flex-col bg-background animate-in fade-in slide-in-from-right-4 duration-200">
        <div className="border-b border-border bg-background px-6 py-4 flex items-center gap-4 sticky top-0 z-10">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => navigate({ search: (prev) => ({ ...prev, selected: null }) })}
            className="text-muted-foreground hover:text-foreground -ml-2 gap-1"
          >
            <Icon icon={ArrowLeft01Icon} className="w-4 h-4" />
            Back to Workflows
          </Button>
          <div className="h-4 w-px bg-border mx-2" />
          <div className="flex items-center gap-2">
            <span className="font-semibold text-lg">{selectedWorkflow.workflowType}</span>
            <Badge
              variant="outline"
              className={cn(
                "gap-1.5 pl-1.5 pr-2.5 py-0.5 border bg-transparent font-normal",
                getStatusColor(selectedWorkflow.status),
              )}
            >
              {getStatusIcon(selectedWorkflow.status)}
              {WorkflowStatusLabel[selectedWorkflow.status]}
            </Badge>
          </div>
          <div className="ml-auto flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              className="text-red-600 hover:text-red-700 hover:bg-red-50 dark:hover:bg-red-950/30 border-red-200 dark:border-red-900/50"
            >
              <Icon icon={Cancel01Icon} className="w-4 h-4 mr-2" />
              Request Cancellation
            </Button>
            <Button variant="outline" size="icon" className="w-9 h-9">
              <Icon icon={Menu01Icon} className="w-4 h-4" />
            </Button>
          </div>
        </div>

        <div className="flex-1 overflow-auto p-8 max-w-7xl mx-auto w-full">
          <WorkflowDetailContent workflow={selectedWorkflow} />
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full bg-background">
      <div className="w-64 border-r border-border bg-sidebar hidden md:flex flex-col">
        <div className="p-4 border-b border-border">
          <h2 className="font-semibold text-sm uppercase tracking-wider text-muted-foreground mb-4">
            Views
          </h2>
          <nav className="space-y-1">
            <FilterButton
              label="All Workflows"
              active={status === "all"}
              count={workflows.length}
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "all" }) })}
            />
            <FilterButton
              label="Running"
              active={status === "running"}
              count={workflows.filter((w) => w.status === WorkflowStatus.RUNNING).length}
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "running" }) })}
            />
            <FilterButton
              label="Completed"
              active={status === "completed"}
              count={workflows.filter((w) => w.status === WorkflowStatus.COMPLETED).length}
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "completed" }) })}
            />
            <FilterButton
              label="Failed"
              active={status === "failed"}
              count={workflows.filter((w) => w.status === WorkflowStatus.FAILED).length}
              onClick={() => navigate({ search: (prev) => ({ ...prev, status: "failed" }) })}
            />
          </nav>
        </div>
      </div>

      <div className="flex-1 flex flex-col min-w-0">
        <div className="border-b border-border p-6 flex justify-between items-start bg-background">
          <div>
            <h1 className="text-2xl font-bold tracking-tight">Workflows</h1>
            <div className="flex items-center gap-2 mt-2 text-sm text-muted-foreground">
              <Icon icon={Clock01Icon} className="w-4 h-4" />
              <span>Last 24 hours</span>
              <span className="w-1 h-1 rounded-full bg-muted-foreground" />
              <span>{filteredWorkflows.length} results</span>
            </div>
          </div>
          <Button className="bg-blue-600 hover:bg-blue-700 text-white shadow-sm gap-2">
            <Icon icon={FlashIcon} className="w-4 h-4" />
            Start Workflow
          </Button>
        </div>

        <div className="p-4 flex items-center gap-4 border-b border-border bg-muted/20">
          <div className="relative flex-1 max-w-md">
            <Icon
              icon={Search01Icon}
              className="absolute left-2.5 top-2.5 w-4 h-4 text-muted-foreground"
            />
            <Input
              placeholder="Filter by Workflow ID, Run ID, or Type..."
              className="pl-9 bg-background"
            />
          </div>
          <Button variant="outline" size="sm" className="gap-2 ml-auto">
            <Icon icon={FilterHorizontalIcon} className="w-4 h-4" />
            Filter
          </Button>
          <Button variant="ghost" size="icon" className="w-9 h-9">
            <Icon icon={RefreshIcon} className="w-4 h-4" />
          </Button>
        </div>

        <div className="flex-1 overflow-auto bg-muted/5 p-6">
          <div className="rounded-lg border border-border bg-background shadow-sm overflow-hidden">
            <table className="w-full text-sm text-left">
              <thead className="bg-muted/50 text-muted-foreground font-medium border-b border-border">
                <tr>
                  <th className="px-4 py-3 w-10">
                    <input type="checkbox" className="rounded border-gray-300" />
                  </th>
                  <th className="px-4 py-3 font-medium">Status</th>
                  <th className="px-4 py-3 font-medium">Workflow ID</th>
                  <th className="px-4 py-3 font-medium">Run ID</th>
                  <th className="px-4 py-3 font-medium">Type</th>
                  <th className="px-4 py-3 font-medium">Start Time</th>
                  <th className="px-4 py-3 font-medium text-right">End Time</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {filteredWorkflows.map((workflow) => (
                  <tr
                    key={workflow.runId}
                    className="hover:bg-muted/30 cursor-pointer transition-colors group"
                    onClick={() =>
                      navigate({ search: (prev) => ({ ...prev, selected: workflow.runId }) })
                    }
                  >
                    <td className="px-4 py-3" onClick={(e) => e.stopPropagation()}>
                      <input type="checkbox" className="rounded border-gray-300" />
                    </td>
                    <td className="px-4 py-3">
                      <div
                        className={cn(
                          "inline-flex items-center gap-1.5 px-2 py-0.5 rounded-full text-xs font-medium border",
                          getStatusColor(workflow.status),
                        )}
                      >
                        {getStatusIcon(workflow.status)}
                        {WorkflowStatusLabel[workflow.status]}
                      </div>
                    </td>
                    <td className="px-4 py-3 font-mono text-xs text-foreground font-medium">
                      {workflow.runId.slice(0, 18)}...
                    </td>
                    <td className="px-4 py-3 font-mono text-xs text-muted-foreground">
                      {workflow.runId.slice(0, 8)}
                    </td>
                    <td className="px-4 py-3 font-medium">{workflow.workflowType}</td>
                    <td className="px-4 py-3 text-muted-foreground whitespace-nowrap">
                      {formatDateTime(workflow.createdAt)}
                    </td>
                    <td className="px-4 py-3 text-right text-muted-foreground whitespace-nowrap">
                      {workflow.finishedAt ? formatDateTime(workflow.finishedAt) : "-"}
                    </td>
                  </tr>
                ))}
                {filteredWorkflows.length === 0 && (
                  <tr>
                    <td colSpan={7} className="px-4 py-12 text-center text-muted-foreground">
                      No workflows found matching current filters.
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
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
}: {
  label: string;
  active?: boolean;
  count?: number;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "w-full flex items-center justify-between px-3 py-2 rounded-md text-sm transition-colors",
        active
          ? "bg-blue-50 text-blue-700 dark:bg-blue-900/20 dark:text-blue-400 font-medium"
          : "text-muted-foreground hover:bg-muted/50 hover:text-foreground",
      )}
    >
      <span>{label}</span>
      {count !== undefined && (
        <span
          className={cn(
            "text-xs px-1.5 py-0.5 rounded-full",
            active ? "bg-blue-100 dark:bg-blue-900/40" : "bg-muted",
          )}
        >
          {count}
        </span>
      )}
    </button>
  );
}

function WorkflowDetailContent({ workflow }: { workflow: (typeof mockWorkflows)[0] }) {
  const [activeTab, setActiveTab] = useState("summary");

  return (
    <div className="space-y-8">
      <div className="grid grid-cols-2 md:grid-cols-4 gap-6 p-6 bg-card border border-border rounded-lg shadow-sm">
        <MetadataItem label="Run ID" value={workflow.runId} copyable mono />
        <MetadataItem label="Workflow Type" value={workflow.workflowType} copyable />
        <MetadataItem label="Task Queue" value={workflow.taskQueue} mono />
        <MetadataItem label="Status" value={WorkflowStatusLabel[workflow.status]} />
        <MetadataItem label="Start Time" value={formatDateTime(workflow.createdAt)} />
        <MetadataItem label="Close Time" value={formatDateTime(workflow.finishedAt)} />
        <MetadataItem
          label="Duration"
          value={formatDuration(workflow.createdAt, workflow.finishedAt)}
        />
        <MetadataItem
          label="Parent ID"
          value={workflow.parentStepId || "None"}
          mono={!!workflow.parentStepId}
        />
      </div>

      <div>
        <div className="flex items-center gap-6 border-b border-border mb-6">
          <TabButton
            label="Summary"
            active={activeTab === "summary"}
            onClick={() => setActiveTab("summary")}
          />
          <TabButton
            label="History"
            active={activeTab === "history"}
            onClick={() => setActiveTab("history")}
          />
          <TabButton
            label="Relationships"
            active={activeTab === "relationships"}
            onClick={() => setActiveTab("relationships")}
          />
          <TabButton
            label="Workers"
            active={activeTab === "workers"}
            onClick={() => setActiveTab("workers")}
          />
          <TabButton
            label="Pending Activities"
            active={activeTab === "pending"}
            onClick={() => setActiveTab("pending")}
          />
        </div>

        {activeTab === "summary" && (
          <div className="grid gap-6 animate-in fade-in slide-in-from-bottom-2 duration-300">
            <div className="grid md:grid-cols-2 gap-6">
              <div className="space-y-2">
                <h3 className="font-semibold text-lg">Input</h3>
                <div className="border border-border rounded-md bg-blue-50/50 dark:bg-blue-950/10 p-4 font-mono text-sm overflow-auto max-h-96">
                  <CodeBlock data={workflow.input?.data} />
                </div>
              </div>
              <div className="space-y-2">
                <h3 className="font-semibold text-lg">Result</h3>
                <div className="border border-border rounded-md bg-muted/30 p-4 font-mono text-sm overflow-auto max-h-96">
                  <CodeBlock data={workflow.output?.data} />
                </div>
              </div>
            </div>

            {workflow.error && (
              <div className="border border-red-200 dark:border-red-900 bg-red-50 dark:bg-red-950/10 rounded-md p-4">
                <h3 className="font-semibold text-red-700 dark:text-red-400 mb-2 flex items-center gap-2">
                  <Icon icon={Alert01Icon} className="w-5 h-5" />
                  Workflow Failure
                </h3>
                <div className="font-mono text-sm text-red-600 dark:text-red-300">
                  {workflow.error.message}
                </div>
              </div>
            )}
          </div>
        )}

        {activeTab !== "summary" && (
          <div className="py-12 text-center text-muted-foreground border border-dashed border-border rounded-lg bg-muted/10">
            <Icon icon={ViewIcon} className="w-10 h-10 mx-auto mb-2 opacity-50" />
            <p>This view is not implemented in the prototype.</p>
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
}: {
  label: string;
  value: string;
  copyable?: boolean;
  mono?: boolean;
}) {
  return (
    <div className="group">
      <dt className="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-1">
        {label}
      </dt>
      <dd className="flex items-center gap-2">
        <span
          className={cn(
            "text-sm text-foreground truncate block max-w-[200px]",
            mono && "font-mono",
          )}
        >
          {value || "-"}
        </span>
        {copyable && value && (
          <button className="opacity-0 group-hover:opacity-100 transition-opacity text-muted-foreground hover:text-blue-500">
            <Icon icon={Copy01Icon} className="w-3.5 h-3.5" />
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
        "pb-3 text-sm font-medium border-b-2 transition-colors px-1",
        active
          ? "border-blue-500 text-blue-600 dark:text-blue-400"
          : "border-transparent text-muted-foreground hover:text-foreground hover:border-muted",
      )}
    >
      {label}
    </button>
  );
}

function CodeBlock({ data }: { data?: string }) {
  if (!data) return <span className="text-muted-foreground italic">No data</span>;

  let content = data;
  try {
    const parsed = JSON.parse(atob(data));
    content = JSON.stringify(parsed, null, 2);
  } catch {
    try {
      content = JSON.stringify(JSON.parse(data), null, 2);
    } catch {
      /* empty */
    }
  }

  return <pre>{content}</pre>;
}
