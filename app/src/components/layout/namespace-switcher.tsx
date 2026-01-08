"use client";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem } from "@/components/ui/sidebar";
import { useNamespace } from "@/hooks/use-namespace";
import { cn } from "@/lib/utils";
import { Check, Layers, LoaderCircle } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useNavigate } from "@tanstack/react-router";

export function NamespaceSwitcher() {
  const { namespace, setNamespace, namespaces, isLoadingNamespaces } = useNamespace();
  const navigate = useNavigate();

  const currentNamespace = namespaces.find((n) => n.namespaceId === namespace);
  const displayName = currentNamespace?.displayName || currentNamespace?.namespaceId || "Namespace";
  const isEmpty = !isLoadingNamespaces && namespaces.length === 0;

  const handleNamespaceChange = async (newNamespaceId: string) => {
    setNamespace(newNamespaceId);
    // Navigate to the new namespace dashboard
    await navigate({
      to: "/$namespaceId",
      params: { namespaceId: newNamespaceId },
    });
  };

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={
              <SidebarMenuButton
                variant="outline"
                size="lg"
                className="hover:bg-transparent hover:text-sidebar-foreground hover:border-primary data-[state=open]:border-primary/60 border border-transparent transition-all group"
              />
            }
          >
            <div
              className={cn(
                "flex aspect-square size-8 items-center justify-center rounded-lg transition-all group-hover:bg-primary group-hover:text-primary-foreground",
                isLoadingNamespaces && "bg-muted text-muted-foreground animate-pulse",
                isEmpty && "bg-muted/50 text-muted-foreground",
                !isLoadingNamespaces &&
                  !isEmpty &&
                  "bg-sidebar-primary text-sidebar-primary-foreground",
              )}
            >
              {isLoadingNamespaces ? (
                <HugeiconsIcon icon={LoaderCircle} className="size-4 animate-spin" />
              ) : (
                <HugeiconsIcon icon={Layers} className="size-4" />
              )}
            </div>
            <div className="flex flex-col gap-0.5 leading-none">
              <span className={cn("font-medium", isLoadingNamespaces && "text-muted-foreground")}>
                {displayName}
              </span>
              <span
                className={cn("text-xs", isEmpty ? "text-destructive" : "text-muted-foreground")}
              >
                {isLoadingNamespaces ? "Loading..." : namespace || "Select namespace"}
              </span>
            </div>
            <HugeiconsIcon icon={Check} className="ml-auto size-4 opacity-50" />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-[--radix-dropdown-menu-trigger-width] min-w-56"
            align="start"
          >
            {isLoadingNamespaces ? (
              <div className="flex items-center justify-center py-4 text-sm text-muted-foreground">
                <HugeiconsIcon icon={LoaderCircle} className="mr-2 size-4 animate-spin" />
                Loading namespaces...
              </div>
            ) : isEmpty ? (
              <div className="flex items-center justify-center py-4 text-sm text-muted-foreground">
                No namespaces found
              </div>
            ) : (
              namespaces.map((ns) => (
                <DropdownMenuItem
                  key={ns.namespaceId}
                  // onSelect={() => handleNamespaceChange(ns.namespaceId)}
                  onClick={() => handleNamespaceChange(ns.namespaceId)}
                  className="hover:border-border/40 hover:bg-transparent focus:bg-transparent border border-transparent transition-all"
                >
                  <div className="flex size-6 items-center justify-center rounded-sm bg-muted/50">
                    <HugeiconsIcon icon={Layers} className="size-3" />
                  </div>
                  <div className="flex flex-col flex-1 min-w-0 ml-2">
                    <span className="font-medium truncate">{ns.displayName || ns.namespaceId}</span>
                    {ns.displayName && ns.displayName !== ns.namespaceId && (
                      <span className="text-xs text-muted-foreground truncate">
                        {ns.namespaceId}
                      </span>
                    )}
                  </div>
                  {ns.namespaceId === namespace && (
                    <HugeiconsIcon icon={Check} className="ml-auto size-4 text-primary" />
                  )}
                </DropdownMenuItem>
              ))
            )}
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
