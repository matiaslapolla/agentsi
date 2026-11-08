"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { BookOpen, Inbox, Workflow } from "lucide-react";
import { HealthIcon } from "@/components/health-icon";
import { RosterMark } from "@/components/roster";
import { ThemeToggle } from "@/components/theme-toggle";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
  SidebarRail,
  useSidebar,
} from "@/components/ui/sidebar";
import { navState, type NavItem } from "@/lib/nav";
import type { Health } from "@/lib/inbox";

export interface SidebarProduct {
  id: string;
  name: string;
  health: Health;
  /** Priorities to mark, decisions to resolve and calls to hear. */
  waiting: number;
  /** Stories mode is on: the group shows a Stories item. */
  stories: boolean;
}

const SUB_ITEMS: { item: NavItem; label: string; href: (id: string) => string; stories?: true }[] = [
  { item: "minutes", label: "Minutes", href: (id) => `/p/${id}#minutes` },
  { item: "stories", label: "Stories", href: (id) => `/p/${id}/stories`, stories: true },
  { item: "roles", label: "Roles", href: (id) => `/p/${id}/roles` },
  { item: "memory", label: "Memory", href: (id) => `/p/${id}/memory` },
];

export function AppSidebar({ products, waitingCount }: { products: SidebarProduct[]; waitingCount: number }) {
  const pathname = usePathname();
  const { setOpenMobile } = useSidebar();
  const closeSheet = () => setOpenMobile(false);
  const nav = navState(pathname, products);
  const page = (on: boolean) => (on ? ("page" as const) : undefined);

  return (
    <Sidebar aria-label="Principal">
      <SidebarHeader>
        <Link href="/" onClick={closeSheet} className="flex items-center gap-3 px-2 py-1.5 font-semibold tracking-tight" aria-label="AgentSI, inicio">
          <RosterMark />
          <span className="text-[16px]">AgentSI</span>
        </Link>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton asChild isActive={nav.item === "agency"}>
                <Link href="/" onClick={closeSheet} aria-current={page(nav.item === "agency")}>
                  <Inbox />
                  <span>Agencia</span>
                </Link>
              </SidebarMenuButton>
              {waitingCount > 0 && <SidebarMenuBadge aria-label={`${waitingCount} te tocan`}>{waitingCount}</SidebarMenuBadge>}
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>
        {products.map((p) => {
          const inGroup = nav.product === p.id;
          return (
            <SidebarGroup key={p.id} className="py-0">
              <SidebarMenu>
                <SidebarMenuItem>
                  <SidebarMenuButton asChild isActive={inGroup && nav.item === "product"}>
                    <Link href={`/p/${p.id}`} onClick={closeSheet} aria-current={page(inGroup && nav.item === "product")}>
                      <HealthIcon health={p.health} />
                      <span>{p.name}</span>
                    </Link>
                  </SidebarMenuButton>
                  {p.waiting > 0 && <SidebarMenuBadge aria-label={`${p.waiting} te esperan`}>{p.waiting}</SidebarMenuBadge>}
                  <SidebarMenuSub>
                    {SUB_ITEMS.filter((s) => !s.stories || p.stories).map((s) => {
                      const on = inGroup && nav.item === s.item;
                      return (
                        <SidebarMenuSubItem key={s.item}>
                          <SidebarMenuSubButton asChild isActive={on}>
                            <Link href={s.href(p.id)} onClick={closeSheet} aria-current={page(on)}>
                              <span>{s.label}</span>
                            </Link>
                          </SidebarMenuSubButton>
                        </SidebarMenuSubItem>
                      );
                    })}
                  </SidebarMenuSub>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroup>
          );
        })}
        <SidebarGroup>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton asChild isActive={nav.item === "guide"}>
                <Link href="/guia" onClick={closeSheet} aria-current={page(nav.item === "guide")}>
                  <BookOpen />
                  <span>Guía</span>
                </Link>
              </SidebarMenuButton>
            </SidebarMenuItem>
            <SidebarMenuItem>
              <SidebarMenuButton asChild isActive={nav.item === "how"}>
                <Link href="/como-funciona" onClick={closeSheet} aria-current={page(nav.item === "how")}>
                  <Workflow />
                  <span>Cómo funciona</span>
                </Link>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <div className="flex items-center justify-between px-2 text-sm text-sidebar-foreground">
          <span>Tema</span>
          <ThemeToggle />
        </div>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}
