import type { Metadata } from "next";
import { cookies } from "next/headers";
import Link from "next/link";
import { AppSidebar } from "@/components/app-sidebar";
import { MigrationPanel } from "@/components/migration-panel";
import { RosterMark } from "@/components/roster";
import { SidebarInset, SidebarProvider, SidebarTrigger } from "@/components/ui/sidebar";
import { toDoQuests, waitingItems } from "@/lib/inbox";
import { loadInbox } from "@/lib/inbox-data";
import { readRegistry } from "@/lib/registry";
import "./globals.css";

export const metadata: Metadata = {
  title: { default: "AgentSI", template: "%s · AgentSI" },
  description: "Agencia de roles para tus productos.",
};

const themeInit = `var t;try{t=localStorage.getItem("agentsi-theme")}catch(e){}try{if(t!=="light"&&t!=="dark")t=matchMedia("(prefers-color-scheme: light)").matches?"light":"dark";document.documentElement.dataset.theme=t}catch(e){}`;

export default async function RootLayout({ children }: { children: React.ReactNode }) {
  const [inbox, jar] = await Promise.all([loadInbox(), cookies()]);
  const withStories = new Set(readRegistry().filter((p) => p.stories).map((p) => p.id));
  const products = inbox.products.map((p) => ({
    id: p.id,
    name: p.name,
    health: p.health,
    waiting: waitingItems(p),
    stories: withStories.has(p.id),
  }));
  const waitingCount = toDoQuests(inbox.products).length;
  return (
    <html lang="es" data-theme="dark" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeInit }} />
      </head>
      <body className="min-h-screen">
        <SidebarProvider defaultOpen={jar.get("sidebar_state")?.value !== "false"}>
          <AppSidebar products={products} waitingCount={waitingCount} />
          <SidebarInset className="min-w-0">
            <header className="sticky top-0 z-20 flex h-14 items-center gap-3 border-b border-border bg-background px-4 md:hidden">
              <SidebarTrigger aria-label="Abrir menú" />
              <Link href="/" className="flex items-center gap-3 font-semibold tracking-tight" aria-label="AgentSI, inicio">
                <RosterMark />
                <span className="text-[16px]">AgentSI</span>
              </Link>
            </header>
            <div className="mx-auto w-full max-w-[1180px] px-4 pb-24 pt-8 sm:px-6">
              <MigrationPanel />
              {children}
            </div>
          </SidebarInset>
        </SidebarProvider>
      </body>
    </html>
  );
}
