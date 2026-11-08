"use client";

import { useEffect, useRef, useState } from "react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { tabForHash, type MinutesTab } from "@/lib/minutes-tabs";

export interface MinutesTabItem {
  value: MinutesTab;
  label: string;
  count: number;
  content: React.ReactNode;
}

export function MinutesTabs({ tabs }: { tabs: MinutesTabItem[] }) {
  const available = tabs.map((t) => t.value);
  const key = available.join(",");
  const [tab, setTab] = useState<MinutesTab>(available[0]);
  const [target, setTarget] = useState<{ id: string } | null>(null);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const shown = key.split(",") as MinutesTab[];
    const select = (hash: string) => {
      const t = tabForHash(hash, shown);
      if (!t) return;
      setTab(t);
      setTarget({ id: hash.slice(1) });
    };
    const sync = () => select(window.location.hash);
    // Clicking the segment of the current hash fires no `hashchange`, so the tab would stay on another panel.
    const onClick = (e: MouseEvent) => {
      const a = (e.target as Element).closest<HTMLAnchorElement>('a[href^="#p-"], a[href^="#d-"]');
      if (a) select(a.getAttribute("href")!);
    };
    sync();
    window.addEventListener("hashchange", sync);
    document.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("hashchange", sync);
      document.removeEventListener("click", onClick);
    };
  }, [key]);

  // Inactive tab panels are not in the DOM, so the browser cannot scroll to an item anchor by itself.
  useEffect(() => {
    if (!target) return;
    (document.getElementById(target.id) ?? root.current)?.scrollIntoView({ block: "start" });
  }, [target]);

  return (
    <Tabs
      ref={root}
      value={tab}
      onValueChange={(v) => {
        setTab(v as MinutesTab);
        window.history.replaceState(null, "", `#${v}`);
      }}
      className="scroll-mt-32 gap-6 md:scroll-mt-24"
    >
      <div className="-mx-4 overflow-x-auto px-4 sm:mx-0 sm:px-0">
        <TabsList className="h-10">
          {tabs.map((t) => (
            <TabsTrigger key={t.value} value={t.value} className="px-3">
              {t.label}
              <span className="mono text-xs tabular-nums text-muted-foreground">{t.count}</span>
            </TabsTrigger>
          ))}
        </TabsList>
      </div>
      {tabs.map((t) => (
        <TabsContent key={t.value} value={t.value}>
          {t.content}
        </TabsContent>
      ))}
    </Tabs>
  );
}
