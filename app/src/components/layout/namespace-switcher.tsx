"use client";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SidebarMenu, SidebarMenuButton, SidebarMenuItem } from "@/components/ui/sidebar";
import type { ListNamespacesRequest } from "@/gen/namespace_pb";
import type { PageRequest } from "@/gen/common_pb";
import { useListNamespaces } from "@/hooks/use-grpc-services";
import { cn } from "@/lib/utils";
import { Check, Layers, LoaderCircle } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import { useNavigate, useParams } from "@tanstack/react-router";

const NAMESPACE_STORAGE_KEY = "kagzi:namespace";

export function NamespaceSwitcher() {
  const params = useParams({ strict: false });
  const namespace = (params as { namespaceId?: string }).namespaceId || "default";
  const navigate = useNavigate();

  const { data: namespacesData, isLoading: isLoadingNamespaces } = useListNamespaces({
    page: {
      pageSize: 100,
      pageToken: "",
    },
  });

  const namespaces = namespacesData?.namespaces ?? [];
  const currentNamespace = namespaces.find((n) => n.namespace === namespace);
  const displayName = currentNamespace?.displayName || currentNamespace?.namespace || "Namespace";
  const isEmpty = !isLoadingNamespaces && namespaces.length === 0;

  const handleNamespaceChange = async (newNamespaceId: string) => {
    // Save to localStorage for persistence
    localStorage.setItem(NAMESPACE_STORAGE_KEY, newNamespaceId);

    // Get current path and determine the route pattern
    const currentPath = window.location.pathname;
    const pathParts = currentPath.split("/").filter(Boolean);

    // If we're in a namespace-specific route, maintain the route structure
    if (pathParts.length > 1) {
      // Replace the namespace but keep the rest of the path
      // e.g., /default/workflows/abc -> /production/workflows/abc
      const newPath = "/" + [newNamespaceId, ...pathParts.slice(1)].join("/");
      await navigate({ to: newPath as any });
    } else {
      // If we're at root or just namespace level, go to namespace dashboard
      await navigate({
        to: "/$namespaceId",
        params: { namespaceId: newNamespaceId },
      });
    }
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
              namespaces
                .filter((ns) => ns.enabled)
                .map((ns) => (
                  <DropdownMenuItem
                    key={ns.namespace}
                    // onSelect={() => handleNamespaceChange(ns.namespace)}
                    onClick={() => handleNamespaceChange(ns.namespace)}
                    className="hover:border-border/40 hover:bg-transparent focus:bg-transparent border border-transparent transition-all"
                  >
                    <div className="flex size-6 items-center justify-center rounded-sm bg-muted/50">
                      <HugeiconsIcon icon={Layers} className="size-3" />
                    </div>
                    <div className="flex flex-col flex-1 min-w-0 ml-2">
                      <span className="font-medium truncate">{ns.displayName || ns.namespace}</span>
                      {ns.displayName && ns.displayName !== ns.namespace && (
                        <span className="text-xs text-muted-foreground truncate">
                          {ns.namespace}
                        </span>
                      )}
                    </div>
                    {ns.namespace === namespace && (
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
