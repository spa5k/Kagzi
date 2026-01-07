import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SidebarMenu, SidebarMenuItem, useSidebar } from "@/components/ui/sidebar";
import type { Namespace } from "@/types";
import { ArrowDown01, Check, ChevronDown, Database, Layers } from "@hugeicons/core-free-icons";
import { HugeiconsIcon } from "@hugeicons/react";
import * as React from "react";

const HARDCODED_NAMESPACES: Namespace[] = [
  {
    id: "default",
    name: "default",
    createdAt: new Date(Date.now() - 720 * 60 * 60 * 1000).toISOString(),
  },
  {
    id: "production",
    name: "production",
    createdAt: new Date(Date.now() - 2160 * 60 * 60 * 1000).toISOString(),
  },
  {
    id: "staging",
    name: "staging",
    createdAt: new Date(Date.now() - 1440 * 60 * 60 * 1000).toISOString(),
  },
];

interface NamespaceSwitcherProps {
  namespaces?: Namespace[];
}

export function NamespaceSwitcher({ namespaces = HARDCODED_NAMESPACES }: NamespaceSwitcherProps) {
  const { isMobile } = useSidebar();
  const [activeNamespace, setActiveNamespace] = React.useState(namespaces[0]);

  if (!activeNamespace) {
    return null;
  }

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger className="ring-sidebar-ring hover:bg-sidebar-accent hover:text-sidebar-accent-foreground active:bg-sidebar-accent active:text-sidebar-accent-foreground data-open:bg-sidebar-accent data-open:text-sidebar-accent-foreground gap-2 rounded-lg p-2 text-left text-sm transition-[width,height,padding] focus-visible:ring-2 data-active:font-medium peer/menu-button flex w-full items-center overflow-hidden outline-hidden group/trigger disabled:pointer-events-none disabled:opacity-50 [&>span:last-child]:truncate [&_svg]:size-4 [&_svg]:shrink-0 h-12 data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground">
            <div className="flex aspect-square size-8 items-center justify-center rounded-md bg-sidebar-accent text-sidebar-accent-foreground transition-colors group-hover/trigger:bg-sidebar-accent/80">
              <HugeiconsIcon icon={Layers} className="size-4" />
            </div>
            <div className="grid flex-1 text-left text-sm leading-tight">
              <span className="truncate font-semibold">{activeNamespace.name}</span>
              <span className="truncate text-xs">Active Namespace</span>
            </div>
            <HugeiconsIcon icon={ChevronDown} className="ml-auto size-4 transition-colors" />
          </DropdownMenuTrigger>
          <DropdownMenuContent
            className="w-(--radix-dropdown-menu-trigger-width) min-w-56 rounded-lg"
            align="start"
            side={isMobile ? "bottom" : "right"}
            sideOffset={4}
          >
            <DropdownMenuGroup>
              <DropdownMenuLabel className="px-2 py-1.5 text-xs font-medium text-muted-foreground uppercase tracking-wider">
                Switch Namespace
              </DropdownMenuLabel>
              {namespaces.map((namespace) => (
                <DropdownMenuItem
                  key={namespace.id}
                  onClick={() => setActiveNamespace(namespace)}
                  className="gap-2 group/item"
                >
                  <div className="flex size-6 items-center justify-center rounded-sm border border-border bg-background transition-colors group-hover/item:border-primary/50 group-hover/item:bg-primary/5">
                    <HugeiconsIcon
                      icon={Database}
                      className="size-3 text-muted-foreground transition-colors group-hover/item:text-primary"
                    />
                  </div>
                  <span className="flex-1 truncate font-medium">{namespace.name}</span>
                  {namespace.id === activeNamespace.id && (
                    <HugeiconsIcon
                      icon={Check}
                      className="size-4 text-primary transition-colors group-hover/item:text-primary/90"
                    />
                  )}
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              disabled
              className="gap-2 text-muted-foreground cursor-not-allowed opacity-60"
            >
              <div className="flex size-6 items-center justify-center rounded-sm border border-border bg-background">
                <HugeiconsIcon icon={ArrowDown01} className="size-3 text-muted-foreground" />
              </div>
              <span className="text-xs font-medium">Create New (Coming Soon)</span>
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
