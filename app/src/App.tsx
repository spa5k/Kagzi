import { AppShell } from "@/components/kagzi/app-shell";

function App() {
  return (
    <AppShell>
      <div className="max-w-6xl mx-auto space-y-8">
        <div>
          <h1 className="text-3xl font-bold tracking-tight mb-2">Dashboard Overview</h1>
          <p className="text-muted-foreground">
            Welcome to the Kagzi workflow engine dashboard. Monitor your workers, schedules, and executions.
          </p>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          <div className="p-6 border bg-card text-card-foreground">
            <h3 className="font-semibold mb-2 text-sm text-muted-foreground uppercase tracking-widest">
              Active Workers
            </h3>
            <div className="text-4xl font-mono font-bold text-primary">0</div>
          </div>
          <div className="p-6 border bg-card text-card-foreground">
            <h3 className="font-semibold mb-2 text-sm text-muted-foreground uppercase tracking-widest">
              Running Workflows
            </h3>
            <div className="text-4xl font-mono font-bold text-accent">0</div>
          </div>
          <div className="p-6 border bg-card text-card-foreground">
            <h3 className="font-semibold mb-2 text-sm text-muted-foreground uppercase tracking-widest">
              Pending Tasks
            </h3>
            <div className="text-4xl font-mono font-bold">0</div>
          </div>
        </div>
      </div>
    </AppShell>
  );
}

export default App;
