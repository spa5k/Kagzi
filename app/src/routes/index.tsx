import { mockWorkflows, mockSchedules, mockWorkers } from "@/lib/mock-data";
import { WorkflowStatus, WorkerStatus } from "@/types";
import { createFileRoute, Link } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: Index,
});

function Index() {
  const runningWorkflows = mockWorkflows.filter((w) => w.status === WorkflowStatus.RUNNING).length;
  const failedWorkflows = mockWorkflows.filter((w) => w.status === WorkflowStatus.FAILED).length;
  const completedWorkflows = mockWorkflows.filter(
    (w) => w.status === WorkflowStatus.COMPLETED,
  ).length;
  const activeSchedules = mockSchedules.filter((s) => s.enabled).length;
  const onlineWorkers = mockWorkers.filter((w) => w.status === WorkerStatus.ONLINE).length;

  return (
    <div className="p-6">
      <div className="mb-8">
        <h1 className="text-2xl font-medium mb-2">Kagzi Dashboard</h1>
        <p className="text-muted-foreground">Workflow orchestration overview</p>
      </div>

      <div className="grid grid-cols-4 gap-4 mb-8">
        <StatCard title="Running" value={runningWorkflows} href="/workflows" variant="running" />
        <StatCard
          title="Completed"
          value={completedWorkflows}
          href="/workflows"
          variant="completed"
        />
        <StatCard title="Failed" value={failedWorkflows} href="/workflows" variant="failed" />
        <StatCard
          title="Workers Online"
          value={onlineWorkers}
          total={mockWorkers.length}
          href="/workers"
        />
      </div>

      <div className="grid grid-cols-2 gap-6">
        <div className="border border-border">
          <div className="p-4 border-b border-border flex items-center justify-between">
            <h2 className="font-medium">Recent Workflows</h2>
            <Link
              to="/workflows"
              search={{ selected: null }}
              className="text-sm text-primary hover:underline"
            >
              View all
            </Link>
          </div>
          <div>
            {mockWorkflows.slice(0, 5).map((workflow) => (
              <Link
                key={workflow.runId}
                to="/workflows"
                search={{ selected: workflow.runId }}
                className="flex items-center gap-3 p-3 border-b border-border last:border-b-0 hover:bg-muted/50"
              >
                <StatusDot status={workflow.status} />
                <span className="font-medium text-sm">{workflow.workflowType}</span>
                <span className="font-mono text-xs text-muted-foreground ml-auto">
                  {workflow.runId.slice(0, 8)}
                </span>
              </Link>
            ))}
          </div>
        </div>

        <div className="border border-border">
          <div className="p-4 border-b border-border flex items-center justify-between">
            <h2 className="font-medium">Active Schedules</h2>
            <Link to="/schedules" className="text-sm text-primary hover:underline">
              View all
            </Link>
          </div>
          <div>
            {mockSchedules
              .filter((s) => s.enabled)
              .map((schedule) => (
                <div
                  key={schedule.scheduleId}
                  className="flex items-center gap-3 p-3 border-b border-border last:border-b-0"
                >
                  <span className="font-medium text-sm">{schedule.workflowType}</span>
                  <span className="font-mono text-xs text-muted-foreground ml-auto">
                    {schedule.cronExpr}
                  </span>
                </div>
              ))}
            {activeSchedules === 0 && (
              <div className="p-3 text-sm text-muted-foreground">No active schedules</div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function StatCard({
  title,
  value,
  total,
  href,
  variant,
}: {
  title: string;
  value: number;
  total?: number;
  href: string;
  variant?: "running" | "completed" | "failed";
}) {
  const variantClasses = {
    running: "border-l-blue-500",
    completed: "border-l-green-500",
    failed: "border-l-red-500",
  };

  return (
    <a
      href={href}
      className={`border border-border p-4 hover:bg-muted/50 transition-colors border-l-4 ${
        variant ? variantClasses[variant] : "border-l-border"
      }`}
    >
      <p className="text-sm text-muted-foreground mb-1">{title}</p>
      <p className="text-3xl font-medium font-mono">
        {value}
        {total !== undefined && <span className="text-lg text-muted-foreground">/{total}</span>}
      </p>
    </a>
  );
}

function StatusDot({ status }: { status: number }) {
  const colorClass =
    {
      [WorkflowStatus.PENDING]: "bg-muted-foreground",
      [WorkflowStatus.RUNNING]: "bg-blue-500 animate-pulse",
      [WorkflowStatus.SLEEPING]: "bg-purple-500",
      [WorkflowStatus.COMPLETED]: "bg-green-500",
      [WorkflowStatus.FAILED]: "bg-red-500",
      [WorkflowStatus.CANCELLED]: "bg-gray-500",
      [WorkflowStatus.SCHEDULED]: "bg-yellow-500",
      [WorkflowStatus.PAUSED]: "bg-yellow-500",
    }[status] || "bg-muted-foreground";

  return <div className={`size-2 ${colorClass}`} />;
}
