import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { QueryError } from "@/components/ui/query-error";
import { Textarea } from "@/components/ui/textarea";
import {
  useCreateNamespace,
  useDisableNamespace,
  useEnableNamespace,
  useListNamespaces,
  useUpdateNamespace,
} from "@/hooks/use-grpc-services";
import { AlertTriangle, CheckCircle, Layers, Plus, XCircle } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";

export const Route = createFileRoute("/namespaces")({
  component: NamespacesPage,
});

function NamespacesPage() {
  const { data, isLoading, error } = useListNamespaces();
  const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false);
  const [isEditDialogOpen, setIsEditDialogOpen] = useState(false);
  const [editingNamespace, setEditingNamespace] = useState<{
    id: string;
    namespace: string;
    displayName: string;
    description: string;
  } | null>(null);

  const createMutation = useCreateNamespace();
  const updateMutation = useUpdateNamespace();
  const enableMutation = useEnableNamespace();
  const disableMutation = useDisableNamespace();

  const [newNamespace, setNewNamespace] = useState({
    namespace: "",
    displayName: "",
    description: "",
  });

  const handleCreate = async () => {
    try {
      await createMutation.mutateAsync({
        namespace: newNamespace.namespace,
        displayName: newNamespace.displayName,
        description: newNamespace.description || undefined,
      });
      setIsCreateDialogOpen(false);
      setNewNamespace({ namespace: "", displayName: "", description: "" });
    } catch (err) {
      console.error("Failed to create namespace:", err);
    }
  };

  const handleUpdate = async () => {
    if (!editingNamespace) return;
    try {
      await updateMutation.mutateAsync({
        namespace: editingNamespace.namespace,
        displayName: editingNamespace.displayName || undefined,
        description: editingNamespace.description || undefined,
      });
      setIsEditDialogOpen(false);
      setEditingNamespace(null);
    } catch (err) {
      console.error("Failed to update namespace:", err);
    }
  };

  const handleToggleEnabled = async (namespace: string, currentlyEnabled: boolean) => {
    try {
      if (currentlyEnabled) {
        await disableMutation.mutateAsync({ namespace });
      } else {
        await enableMutation.mutateAsync({ namespace });
      }
    } catch (err) {
      console.error("Failed to toggle namespace status:", err);
    }
  };

  const openEditDialog = (ns: any) => {
    setEditingNamespace({
      id: ns.id,
      namespace: ns.namespace,
      displayName: ns.displayName,
      description: ns.description,
    });
    setIsEditDialogOpen(true);
  };

  if (error) {
    return (
      <div className="h-full flex items-center justify-center p-6 bg-background text-foreground font-mono">
        <QueryError
          error={error}
          title="NAMESPACE_ERROR"
          description="Failed to load namespaces from server."
          className="max-w-md border-2 border-destructive bg-destructive/5"
        />
      </div>
    );
  }

  const namespaces = data?.namespaces || [];
  const enabledCount = namespaces.filter((ns) => ns.enabled).length;
  const disabledCount = namespaces.length - enabledCount;

  return (
    <div className="min-h-screen p-6 md:p-12 max-w-[1600px] mx-auto font-sans bg-background text-foreground">
      <header className="mb-16 flex flex-col md:flex-row md:items-end justify-between gap-8 border-b border-border pb-8">
        <div className="space-y-2">
          <div className="flex items-center gap-3">
            <div className="size-4 bg-primary" />
            <span className="font-mono text-xs tracking-widest text-muted-foreground uppercase">
              System Management
            </span>
          </div>
          <h1 className="text-5xl md:text-7xl font-black tracking-tighter uppercase leading-[0.9]">
            Namespaces<span className="text-primary">.</span>
          </h1>
        </div>

        <div className="flex items-center gap-4">
          <div className="flex flex-col items-end font-mono text-xs text-muted-foreground space-y-1 tracking-wide uppercase">
            <span className="flex items-center gap-2">
              <HugeiconsIcon icon={CheckCircle} className="size-3 text-accent" />
              Enabled: {enabledCount}
            </span>
            <span className="flex items-center gap-2">
              <HugeiconsIcon icon={XCircle} className="size-3 text-muted-foreground" />
              Disabled: {disabledCount}
            </span>
          </div>

          <Dialog open={isCreateDialogOpen} onOpenChange={setIsCreateDialogOpen}>
            <DialogTrigger asChild>
              <Button className="gap-2">
                <HugeiconsIcon icon={Plus} className="size-4" />
                Create Namespace
              </Button>
            </DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Create New Namespace</DialogTitle>
                <DialogDescription>
                  Create a new namespace for multi-tenancy isolation. Namespaces are enabled by
                  default.
                </DialogDescription>
              </DialogHeader>
              <div className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="namespace">Namespace Identifier *</Label>
                  <Input
                    id="namespace"
                    placeholder="e.g., production, staging"
                    value={newNamespace.namespace}
                    onChange={(e) =>
                      setNewNamespace({ ...newNamespace, namespace: e.target.value })
                    }
                  />
                  <p className="text-xs text-muted-foreground">
                    Unique identifier (lowercase, no spaces)
                  </p>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="displayName">Display Name *</Label>
                  <Input
                    id="displayName"
                    placeholder="e.g., Production Environment"
                    value={newNamespace.displayName}
                    onChange={(e) =>
                      setNewNamespace({ ...newNamespace, displayName: e.target.value })
                    }
                  />
                </div>
                <div className="space-y-2">
                  <Label htmlFor="description">Description</Label>
                  <Textarea
                    id="description"
                    placeholder="Optional description..."
                    value={newNamespace.description}
                    onChange={(e) =>
                      setNewNamespace({ ...newNamespace, description: e.target.value })
                    }
                  />
                </div>
              </div>
              <DialogFooter>
                <Button
                  variant="outline"
                  onClick={() => setIsCreateDialogOpen(false)}
                  disabled={createMutation.isPending}
                >
                  Cancel
                </Button>
                <Button
                  onClick={handleCreate}
                  disabled={
                    !newNamespace.namespace || !newNamespace.displayName || createMutation.isPending
                  }
                >
                  {createMutation.isPending ? "Creating..." : "Create"}
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      </header>

      {isLoading ? (
        <div className="space-y-4 animate-pulse">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="h-32 bg-muted/50 w-full" />
          ))}
        </div>
      ) : (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {namespaces.map((ns) => (
            <Card
              key={ns.id}
              className={`relative border-2 transition-all ${
                ns.enabled
                  ? "border-border hover:border-primary"
                  : "border-muted bg-muted/20 opacity-75"
              }`}
            >
              <CardHeader>
                <div className="flex items-start justify-between">
                  <div className="flex items-center gap-3">
                    <div
                      className={`flex size-10 items-center justify-center rounded-lg ${
                        ns.enabled ? "bg-primary text-primary-foreground" : "bg-muted"
                      }`}
                    >
                      <HugeiconsIcon icon={Layers} className="size-5" />
                    </div>
                    <div>
                      <CardTitle className="text-xl">{ns.displayName}</CardTitle>
                      <p className="text-xs text-muted-foreground font-mono">{ns.namespace}</p>
                    </div>
                  </div>
                  <Badge variant={ns.enabled ? "default" : "secondary"} className="font-mono">
                    {ns.enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </div>
                {ns.description && (
                  <CardDescription className="mt-2">{ns.description}</CardDescription>
                )}
              </CardHeader>
              <CardContent>
                <div className="flex gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    className="flex-1"
                    onClick={() => openEditDialog(ns)}
                    disabled={updateMutation.isPending}
                  >
                    Edit
                  </Button>
                  <Button
                    size="sm"
                    variant={ns.enabled ? "destructive" : "default"}
                    className="flex-1 gap-2"
                    onClick={() => handleToggleEnabled(ns.namespace, ns.enabled)}
                    disabled={enableMutation.isPending || disableMutation.isPending}
                  >
                    {ns.enabled ? (
                      <>
                        <HugeiconsIcon icon={XCircle} className="size-4" />
                        Disable
                      </>
                    ) : (
                      <>
                        <HugeiconsIcon icon={CheckCircle} className="size-4" />
                        Enable
                      </>
                    )}
                  </Button>
                </div>
                {!ns.enabled && (
                  <div className="mt-3 flex items-center gap-2 text-xs text-muted-foreground">
                    <HugeiconsIcon icon={AlertTriangle} className="size-3" />
                    <span>No new workflows can start in this namespace</span>
                  </div>
                )}
              </CardContent>
            </Card>
          ))}
        </div>
      )}

      {/* Edit Dialog */}
      <Dialog open={isEditDialogOpen} onOpenChange={setIsEditDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit Namespace</DialogTitle>
            <DialogDescription>
              Update the display name and description. The namespace identifier cannot be changed.
            </DialogDescription>
          </DialogHeader>
          {editingNamespace && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="edit-namespace">Namespace Identifier</Label>
                <Input
                  id="edit-namespace"
                  value={editingNamespace.namespace}
                  disabled
                  className="font-mono text-muted-foreground"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="edit-displayName">Display Name *</Label>
                <Input
                  id="edit-displayName"
                  placeholder="e.g., Production Environment"
                  value={editingNamespace.displayName}
                  onChange={(e) =>
                    setEditingNamespace({ ...editingNamespace, displayName: e.target.value })
                  }
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="edit-description">Description</Label>
                <Textarea
                  id="edit-description"
                  placeholder="Optional description..."
                  value={editingNamespace.description}
                  onChange={(e) =>
                    setEditingNamespace({ ...editingNamespace, description: e.target.value })
                  }
                />
              </div>
            </div>
          )}
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => {
                setIsEditDialogOpen(false);
                setEditingNamespace(null);
              }}
              disabled={updateMutation.isPending}
            >
              Cancel
            </Button>
            <Button
              onClick={handleUpdate}
              disabled={!editingNamespace?.displayName || updateMutation.isPending}
            >
              {updateMutation.isPending ? "Saving..." : "Save Changes"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
