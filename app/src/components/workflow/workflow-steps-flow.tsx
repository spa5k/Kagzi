import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { StepKind, StepStatus } from "@/types";
import { timestampDate } from "@bufbuild/protobuf/wkt";
import type { Node, NodeProps } from "reactflow";
import ReactFlow, { Background, Controls, Handle, MiniMap, Position } from "reactflow";
import type { Step } from "@/gen/worker_pb";
import { useMemo } from "react";

type StepNodeData = {
  name: string;
  stepId: string;
  status: number;
  kind: number;
  attemptNumber: number;
  createdAtLabel: string;
  childRunId?: string;
};

function statusClass(status: number) {
  switch (status) {
    case StepStatus.RUNNING:
      return "border-primary/30 bg-primary/5";
    case StepStatus.COMPLETED:
      return "border-primary/30 bg-primary/5";
    case StepStatus.FAILED:
      return "border-destructive/30 bg-destructive/5";
    case StepStatus.PENDING:
      return "border-yellow-600/30 bg-yellow-600/5";
    default:
      return "border-border bg-background";
  }
}

function statusBadgeVariant(status: number): "default" | "secondary" | "destructive" | "outline" {
  switch (status) {
    case StepStatus.FAILED:
      return "destructive";
    case StepStatus.RUNNING:
      return "default";
    case StepStatus.COMPLETED:
      return "default";
    case StepStatus.PENDING:
      return "secondary";
    default:
      return "outline";
  }
}

function kindLabel(kind: number) {
  switch (kind) {
    case StepKind.FUNCTION:
      return "function";
    case StepKind.SLEEP:
      return "sleep";
    case StepKind.CHILD_WORKFLOW:
      return "child_workflow";
    case StepKind.LIFECYCLE:
      return "lifecycle";
    default:
      return "unspecified";
  }
}

function StepNode({ data }: NodeProps<StepNodeData>) {
  return (
    <div
      className={cn(
        "rounded-md border shadow-sm bg-background min-w-[240px] max-w-[320px]",
        statusClass(data.status),
      )}
    >
      <Handle type="target" position={Position.Top} className="opacity-0" />
      <div className="p-3">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="font-mono text-[10px] text-muted-foreground truncate">
              {data.stepId}
            </div>
            <div className="mt-1 font-medium text-sm truncate">
              {data.name || <span className="text-muted-foreground">Unnamed step</span>}
            </div>
          </div>
          <Badge
            variant={statusBadgeVariant(data.status)}
            className="font-mono text-[10px] uppercase tracking-wider shrink-0"
          >
            {StepStatus[data.status] || "UNKNOWN"}
          </Badge>
        </div>
        <div className="mt-2 flex items-center justify-between gap-2 text-[10px] font-mono text-muted-foreground uppercase tracking-wider">
          <span>{kindLabel(data.kind)}</span>
          <span>attempt {data.attemptNumber}</span>
        </div>
        <div className="mt-2 text-[10px] font-mono text-muted-foreground">
          {data.createdAtLabel}
        </div>
        {data.childRunId && (
          <div className="mt-2 text-[10px] font-mono text-muted-foreground truncate">
            child: {data.childRunId}
          </div>
        )}
      </div>
      <Handle type="source" position={Position.Bottom} className="opacity-0" />
    </div>
  );
}

const nodeTypes = {
  step: StepNode,
} as const;

function formatShortDate(step: Step) {
  if (!step.createdAt) return "—";
  return timestampDate(step.createdAt).toLocaleString("en-US", {
    month: "numeric",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

function sortSteps(steps: Step[]) {
  return [...steps].sort((a, b) => {
    const aT = a.createdAt ? timestampDate(a.createdAt).getTime() : 0;
    const bT = b.createdAt ? timestampDate(b.createdAt).getTime() : 0;
    if (aT !== bT) return aT - bT;
    return a.stepId.localeCompare(b.stepId);
  });
}

export function WorkflowStepsFlow({ steps }: { steps: Step[] }) {
  const { nodes, edges } = useMemo(() => {
    const ordered = sortSteps(steps);

    const nodes: Node<StepNodeData>[] = ordered.map((step, idx) => ({
      id: step.stepId,
      type: "step",
      position: { x: 0, y: idx * 140 },
      data: {
        name: step.name,
        stepId: step.stepId,
        status: step.status,
        kind: step.kind,
        attemptNumber: step.attemptNumber,
        createdAtLabel: formatShortDate(step),
        childRunId: step.childRunId || undefined,
      },
    }));

    const edges = ordered.slice(0, Math.max(0, ordered.length - 1)).map((step, idx) => ({
      id: `e:${step.stepId}:${ordered[idx + 1]!.stepId}`,
      source: step.stepId,
      target: ordered[idx + 1]!.stepId,
      animated: ordered[idx + 1]!.status === StepStatus.RUNNING,
      style: { strokeWidth: 1.5 },
    }));

    // Optional child-workflow nodes (shown to the right of their parent step).
    const childNodesByRunId = new Map<string, Node>();
    const childEdges = ordered.flatMap((step, idx) => {
      if (!step.childRunId) return [];
      const id = `child:${step.childRunId}`;
      if (!childNodesByRunId.has(id)) {
        childNodesByRunId.set(id, {
          id,
          type: "default",
          position: { x: 420, y: idx * 140 },
          data: { label: `child workflow\n${step.childRunId.slice(0, 12)}…` },
          style: {
            background: "var(--card)",
            color: "var(--card-foreground)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            padding: 10,
            fontFamily: "var(--font-mono, ui-monospace, SFMono-Regular, Menlo, monospace)",
            fontSize: 11,
            whiteSpace: "pre-line",
          },
        });
      }
      return [
        {
          id: `e:${step.stepId}:${id}`,
          source: step.stepId,
          target: id,
          animated: false,
          style: { strokeWidth: 1.25, strokeDasharray: "4 4" },
        },
      ];
    });

    return {
      nodes: [...nodes, ...Array.from(childNodesByRunId.values())],
      edges: [...edges, ...childEdges],
    };
  }, [steps]);

  if (steps.length === 0) {
    return (
      <div className="border-2 border-dashed border-border/50 rounded-lg bg-muted/5 p-12 text-center">
        <p className="font-mono text-xs text-muted-foreground uppercase tracking-widest">
          No execution steps recorded
        </p>
      </div>
    );
  }

  return (
    <div className="h-[700px] border border-border rounded-lg overflow-hidden bg-background">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable
        proOptions={{ hideAttribution: true }}
      >
        <MiniMap
          pannable
          zoomable
          nodeStrokeWidth={2}
          nodeColor={(n) => {
            const data = n.data as StepNodeData | undefined;
            const status = data?.status ?? StepStatus.UNSPECIFIED;
            switch (status) {
              case StepStatus.FAILED:
                return "rgb(239 68 68)";
              case StepStatus.RUNNING:
                return "rgb(34 197 94)";
              case StepStatus.COMPLETED:
                return "rgb(34 197 94)";
              case StepStatus.PENDING:
                return "rgb(234 179 8)";
              default:
                return "rgb(148 163 184)";
            }
          }}
        />
        <Controls />
        <Background gap={16} size={1} />
      </ReactFlow>
    </div>
  );
}
