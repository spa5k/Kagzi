import {
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";
import type { IconSvgElement } from "@hugeicons/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { Link, useRouterState } from "@tanstack/react-router";

export interface NavItem {
  title: string;
  url: string;
  icon?: IconSvgElement;
}

interface NavMainProps {
  items: NavItem[];
  label?: string;
}

export function NavMain({ items, label = "Monitor" }: NavMainProps) {
  const routerState = useRouterState();
  const currentPath = routerState.location.pathname;

  return (
    <SidebarGroup>
      <SidebarGroupLabel>{label}</SidebarGroupLabel>
      <SidebarMenu>
        {items.map((item) => {
          // Extract the route part after the namespace (if any)
          // For namespace routes like /scheduling/workflows, we want to match /workflows
          // For root routes like /workers, we want exact match
          const pathParts = currentPath.split("/").filter(Boolean);
          const itemParts = item.url.split("/").filter(Boolean);

          let isActive = false;

          if (itemParts.length === 1) {
            // Root level route like /workers - exact match
            isActive = currentPath === item.url;
          } else if (itemParts.length === 2 && pathParts.length >= 2) {
            // Namespace-scoped route like /scheduling/workflows
            // Check if the second part matches (the actual route)
            const currentRoute = pathParts[1];
            const itemRoute = itemParts[1];

            if (itemRoute === undefined) {
              // This is the namespace root (overview)
              isActive =
                pathParts.length === 1 || (pathParts.length === 2 && currentRoute === undefined);
            } else {
              // Match the route part and allow sub-routes
              isActive = currentRoute === itemRoute || currentPath.startsWith(`${item.url}/`);
            }
          }

          return (
            <SidebarMenuItem key={item.title}>
              <Link to={item.url}>
                {" "}
                <SidebarMenuButton
                  tooltip={item.title}
                  isActive={isActive}
                  // render={(props) => <Link to={item.url} {...props} />}
                >
                  {item.icon && <HugeiconsIcon icon={item.icon} />}
                  <span>{item.title}</span>
                </SidebarMenuButton>
              </Link>
            </SidebarMenuItem>
          );
        })}
      </SidebarMenu>
    </SidebarGroup>
  );
}
