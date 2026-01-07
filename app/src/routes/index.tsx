import { QueryError } from "@/components/ui/query-error";
import { WorkflowStatus as ProtoWorkflowStatus } from "@/gen/workflow_pb";
import { useSchedules, useWorkers, useWorkflows } from "@/hooks/use-dashboard";
import { WorkerStatus } from "@/types";
import { createFileRoute, Link } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: Index,
});

function Index() {
  const { data: workflows, isLoading: workflowsLoading, error: workflowsError } = useWorkflows();
  const { data: schedules, isLoading: schedulesLoading, error: schedulesError } = useSchedules();
  const { data: workers, isLoading: workersLoading, error: workersError } = useWorkers();

  const runningWorkflows =
    workflows?.filter((w) => w.status === ProtoWorkflowStatus.RUNNING).length ?? 0;
  const completedWorkflows =
    workflows?.filter((w) => w.status === ProtoWorkflowStatus.COMPLETED).length ?? 0;
  const failedWorkflows =
    workflows?.filter((w) => w.status === ProtoWorkflowStatus.FAILED).length ?? 0;
  const activeSchedules = schedules?.filter((s) => s.enabled).length ?? 0;
  const onlineWorkers = workers?.filter((w) => w.status === WorkerStatus.ONLINE).length ?? 0;

  const isLoading = workflowsLoading || schedulesLoading || workersLoading;
  const error = workflowsError || schedulesError || workersError;

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background">
        <QueryError
          error={error}
          title="Dashboard Error"
          description="Unable to load dashboard data. Please check your connection to Kagzi server."
          className="max-w-md"
        />
      </div>
    );
  }

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
          total={workers?.length ?? 0}
          href="/workers"
        />
      </div>

      <div className="grid grid-cols-2 gap-6">
        <DashboardSection
          title="Recent Workflows"
          link={{ to: "/workflows", search: { selected: null } }}
          isLoading={workflowsLoading}
        >
          {workflows?.slice(0, 5).map((workflow) => (
            <Link
              key={workflow.runId}
              to="/workflows"
              search={{ selected: workflow.runId, status: "all", timeRange: "24h" }}
              className="flex items-center gap-3 p-3 border-b border-border last:border-b-0 hover:bg-muted/50"
            >
              <StatusDot status={workflow.status} />
              <span className="font-medium text-sm">{workflow.workflowType}</span>
              <span className="font-mono text-xs text-muted-foreground ml-auto">
                {workflow.runId.slice(0, 8)}
              </span>
            </Link>
          ))}
        </DashboardSection>

        <DashboardSection
          title="Active Schedules"
          link={{ to: "/schedules" }}
          isLoading={schedulesLoading}
        >
          {schedules
            ?.filter((s) => s.enabled)
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
          {activeSchedules === 0 && !isLoading && (
            <div className="p-3 text-sm text-muted-foreground">No active schedules</div>
          )}
        </DashboardSection>
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

function DashboardSection({
  title,
  link,
  isLoading,
  children,
}: {
  title: string;
  link?: { to: string; search?: Record<string, unknown> };
  isLoading: boolean;
  children: React.ReactNode;
}) {
  if (isLoading) {
    return (
      <div className="border border-border">
        <div className="p-4 border-b border-border flex items-center justify-between">
          <h2 className="font-medium">{title}</h2>
        </div>
        <div className="p-4 space-y-3">
          {[...Array(5)].map((_, i) => (
            <div
              key={i}
              className="animate-pulse flex items-center gap-3 p-3 border-b border-border last:border-b-0"
            >
              <div className="size-2 bg-muted rounded-full" />
              <div className="flex-1 h-4 bg-muted rounded" />
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="border border-border">
      <div className="p-4 border-b border-border flex items-center justify-between">
        <h2 className="font-medium">{title}</h2>
        {link && (
          <Link to={link.to} search={link.search} className="text-sm text-primary hover:underline">
            View all
          </Link>
        )}
      </div>
      <div>{children}</div>
    </div>
  );
}

function StatusDot({ status }: { status: ProtoWorkflowStatus }) {
  const colorClass =
    {
      [ProtoWorkflowStatus.UNSPECIFIED]: "bg-muted-foreground",
      [ProtoWorkflowStatus.PENDING]: "bg-muted-foreground",
      [ProtoWorkflowStatus.RUNNING]: "bg-blue-500 animate-pulse",
      [ProtoWorkflowStatus.SLEEPING]: "bg-purple-500",
      [ProtoWorkflowStatus.COMPLETED]: "bg-green-500",
      [ProtoWorkflowStatus.FAILED]: "bg-red-500",
      [ProtoWorkflowStatus.CANCELLED]: "bg-gray-500",
      [ProtoWorkflowStatus.SCHEDULED]: "bg-yellow-500",
      [ProtoWorkflowStatus.PAUSED]: "bg-yellow-500",
    }[status] || "bg-muted-foreground";

  return <div className={`size-2 ${colorClass}`} />;
}
