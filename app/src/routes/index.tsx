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
  const totalWorkers = workers?.length ?? 0;

  const isLoading = workflowsLoading || schedulesLoading || workersLoading;
  const error = workflowsError || schedulesError || workersError;

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background text-foreground font-mono">
        <QueryError
          error={error}
          title="SYSTEM_ERROR"
          description="CONNECTION_FAILURE: Unable to synchronize with Kagzi core."
          className="max-w-md border-2 border-destructive bg-destructive/5"
        />
      </div>
    );
  }

  return (
    <div className="min-h-screen p-6 md:p-12 max-w-[1600px] mx-auto font-sans bg-background text-foreground">
      <header className="mb-16 flex flex-col md:flex-row md:items-end justify-between gap-8 border-b border-border pb-8">
        <div className="space-y-2">
          <div className="flex items-center gap-3">
            <div className="size-4 bg-primary animate-pulse" />
            <span className="font-mono text-xs tracking-widest text-muted-foreground uppercase">
              Dashboard // Main
            </span>
          </div>
          <h1 className="text-5xl md:text-7xl font-black tracking-tighter uppercase leading-[0.9]">
            Kagzi<span className="text-primary">.</span>
          </h1>
        </div>

        <div className="flex flex-col items-start md:items-end font-mono text-xs text-muted-foreground space-y-1 tracking-wide uppercase">
          <span>Sys_Time: {new Date().toLocaleTimeString()}</span>
          <span>Env: Production</span>
          <span className="flex items-center gap-2">
            Status: <span className="text-primary">Operational</span>
          </span>
        </div>
      </header>

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 mb-20">
        <MetricCard
          label="Running Processes"
          value={runningWorkflows}
          variant="primary"
          href="/workflows?status=running"
        />
        <MetricCard
          label="Completed Tasks"
          value={completedWorkflows}
          variant="success"
          href="/workflows?status=completed"
        />
        <MetricCard
          label="System Failures"
          value={failedWorkflows}
          variant="destructive"
          href="/workflows?status=failed"
        />
        <MetricCard
          label="Active Workers"
          value={onlineWorkers}
          subValue={`/${totalWorkers}`}
          variant="neutral"
          href="/workers"
        />
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-x-12 gap-y-16">
        <section>
          <SectionHeader title="Recent Executions" link="/workflows" linkText="View All Logs" />
          <div className="space-y-2">
            {isLoading ? (
              <LoadingSkeleton />
            ) : (
              <div className="border-t border-border">
                {workflows?.slice(0, 5).map((workflow) => (
                  <Link
                    key={workflow.runId}
                    to="/workflows"
                    search={{ selected: workflow.runId }}
                    className="group flex items-center justify-between py-4 border-b border-border hover:bg-muted/50 transition-colors px-2"
                  >
                    <div className="flex items-center gap-4">
                      <StatusIndicator status={workflow.status} />
                      <div className="flex flex-col">
                        <span className="font-bold text-sm tracking-tight uppercase group-hover:text-primary transition-colors">
                          {workflow.workflowType}
                        </span>
                        <span className="font-mono text-[10px] text-muted-foreground">
                          ID: {workflow.runId.slice(0, 8)}
                        </span>
                      </div>
                    </div>
                    <div className="font-mono text-xs opacity-50 group-hover:opacity-100 transition-opacity">
                      Running
                    </div>
                  </Link>
                ))}
              </div>
            )}
          </div>
        </section>

        <section>
          <SectionHeader title="Active Schedules" link="/schedules" linkText="Manage Schedules" />
          <div className="space-y-2">
            {isLoading ? (
              <LoadingSkeleton />
            ) : (
              <div className="border-t border-border">
                {schedules
                  ?.filter((s) => s.enabled)
                  .map((schedule) => (
                    <div
                      key={schedule.scheduleId}
                      className="group flex items-center justify-between py-4 border-b border-border px-2"
                    >
                      <div className="flex items-center gap-4">
                        <div className="size-2 bg-foreground" />
                        <div className="flex flex-col">
                          <span className="font-bold text-sm tracking-tight uppercase">
                            {schedule.workflowType}
                          </span>
                          <span className="font-mono text-[10px] text-muted-foreground">
                            ID: {schedule.scheduleId.slice(0, 8)}
                          </span>
                        </div>
                      </div>
                      <div className="px-2 py-1 bg-muted font-mono text-[10px]">
                        {schedule.cronExpr}
                      </div>
                    </div>
                  ))}
                {activeSchedules === 0 && (
                  <div className="py-8 text-center border-b border-border">
                    <p className="font-mono text-xs text-muted-foreground uppercase">
                      No active schedules
                    </p>
                  </div>
                )}
              </div>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}

function MetricCard({
  label,
  value,
  subValue,
  variant = "neutral",
  href,
}: {
  label: string;
  value: number;
  subValue?: string;
  variant?: "primary" | "success" | "destructive" | "neutral";
  href: string;
}) {
  const colors = {
    primary: "group-hover:text-primary group-hover:border-primary",
    success: "group-hover:text-accent group-hover:border-accent",
    destructive: "group-hover:text-destructive group-hover:border-destructive",
    neutral: "group-hover:text-foreground group-hover:border-foreground",
  };

  return (
    <Link
      to={href}
      className={`group relative p-6 border border-border bg-card hover:shadow-lg transition-all duration-300 ${colors[variant]}`}
    >
      <div className="flex flex-col h-full justify-between gap-4">
        <div className="flex justify-between items-start">
          <span className="font-mono text-[10px] uppercase tracking-widest text-muted-foreground group-hover:text-current transition-colors">
            {label}
          </span>
          <div
            className={`size-2 rounded-full ${variant === "neutral" ? "bg-muted-foreground" : `bg-${variant === "primary" ? "primary" : variant === "success" ? "accent" : "destructive"}`}`}
          />
        </div>
        <div className="flex items-baseline gap-2">
          <span className="text-6xl font-black tracking-tighter leading-none">{value}</span>
          {subValue && (
            <span className="font-mono text-xl text-muted-foreground group-hover:text-current opacity-50">
              {subValue}
            </span>
          )}
        </div>
      </div>
      <div className="absolute top-0 right-0 p-1 opacity-0 group-hover:opacity-100 transition-opacity">
        <svg width="10" height="10" viewBox="0 0 10 10" className="fill-current">
          <path d="M0 0H10V10L0 0Z" />
        </svg>
      </div>
    </Link>
  );
}

function SectionHeader({
  title,
  link,
  linkText,
}: {
  title: string;
  link: string;
  linkText: string;
}) {
  return (
    <div className="flex items-end justify-between mb-6">
      <h2 className="text-xl font-bold tracking-tight uppercase flex items-center gap-2">
        <span className="w-1 h-6 bg-primary block" />
        {title}
      </h2>
      <Link
        to={link}
        className="font-mono text-xs uppercase tracking-wider text-muted-foreground hover:text-primary transition-colors flex items-center gap-2 group"
      >
        {linkText}
        <span className="group-hover:translate-x-1 transition-transform">→</span>
      </Link>
    </div>
  );
}

function StatusIndicator({ status }: { status: ProtoWorkflowStatus }) {
  const styles =
    {
      [ProtoWorkflowStatus.RUNNING]: "bg-primary animate-pulse",
      [ProtoWorkflowStatus.COMPLETED]: "bg-accent",
      [ProtoWorkflowStatus.FAILED]: "bg-destructive",
      [ProtoWorkflowStatus.PENDING]: "bg-muted-foreground",
      [ProtoWorkflowStatus.SLEEPING]: "bg-blue-500",
      [ProtoWorkflowStatus.CANCELLED]: "bg-muted",
      [ProtoWorkflowStatus.UNSPECIFIED]: "bg-muted",
      [ProtoWorkflowStatus.SCHEDULED]: "bg-yellow-500",
      [ProtoWorkflowStatus.PAUSED]: "bg-yellow-500",
    }[status] || "bg-muted";

  return <div className={`size-3 border border-background ring-1 ring-border ${styles}`} />;
}

function LoadingSkeleton() {
  return (
    <div className="space-y-4 animate-pulse">
      {[...Array(3)].map((_, i) => (
        <div key={i} className="h-16 bg-muted/50 w-full" />
      ))}
    </div>
  );
}
