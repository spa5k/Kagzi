import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SidebarMenu, SidebarMenuItem, useSidebar } from "@/components/ui/sidebar";
import { mockNamespaces } from "@/lib/mock-data";
import type { Namespace } from "@/types";
import { ChevronDown, Database } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import * as React from "react";

interface NamespaceSwitcherProps {
  namespaces?: Namespace[];
}

export function NamespaceSwitcher({ namespaces = mockNamespaces }: NamespaceSwitcherProps) {
  const { isMobile } = useSidebar();
  const [activeNamespace, setActiveNamespace] = React.useState(namespaces[0]);

  if (!activeNamespace) {
    return null;
  }

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger className="ring-sidebar-ring hover:bg-sidebar-accent hover:text-sidebar-accent-foreground active:bg-sidebar-accent active:text-sidebar-accent-foreground data-open:bg-sidebar-accent data-open:text-sidebar-accent-foreground gap-2 p-2 text-left text-sm transition-[width,height,padding] focus-visible:ring-2 data-active:font-medium peer/menu-button flex w-full items-center overflow-hidden outline-hidden group/menu-button disabled:pointer-events-none disabled:opacity-50 [&>span:last-child]:truncate [&_svg]:size-4 [&_svg]:shrink-0 data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground h-12 border border-sidebar-border">
            <div className="bg-sidebar-primary text-sidebar-primary-foreground flex aspect-square size-8 items-center justify-center">
              <HugeiconsIcon icon={Database} className="size-4" />
            </div>
            <div className="grid flex-1 text-left text-sm leading-tight">
              <span className="truncate font-medium">{activeNamespace.name}</span>
              <span className="truncate text-xs text-muted-foreground">namespace</span>
            </div>
            <HugeiconsIcon icon={ChevronDown} className="ml-auto" />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-(--radix-dropdown-menu-trigger-width) min-w-56"
            align="start"
            side={isMobile ? "bottom" : "right"}
            sideOffset={4}
          >
            <DropdownMenuLabel className="text-muted-foreground text-xs">
              Namespaces
            </DropdownMenuLabel>
            {namespaces.map((namespace) => (
              <DropdownMenuItem
                key={namespace.id}
                onClick={() => setActiveNamespace(namespace)}
                className="gap-2 p-2"
              >
                <div className="flex size-6 items-center justify-center border bg-background">
                  <HugeiconsIcon icon={Database} className="size-3" />
                </div>
                <span className="font-mono text-sm">{namespace.name}</span>
                {namespace.id === activeNamespace.id && (
                  <span className="ml-auto text-xs text-primary">active</span>
                )}
              </DropdownMenuItem>
            ))}
            <DropdownMenuSeparator />
            <DropdownMenuItem className="gap-2 p-2 text-muted-foreground" disabled>
              <span className="text-xs">Create namespace (not implemented)</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
