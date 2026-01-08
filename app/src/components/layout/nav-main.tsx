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
          const isActive =
            currentPath === item.url ||
            (item.url !== "/" && currentPath.startsWith(`${item.url}/`));

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
