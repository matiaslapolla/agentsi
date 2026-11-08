"use client";

import { useEffect, useState } from "react";
import { Accordion } from "@/components/ui/accordion";

/** An accordion whose items open when the URL hash names them, for item values like `p-2`. */
export function FoldList({ prefix, className, children }: { prefix?: string; className?: string; children: React.ReactNode }) {
  const [open, setOpen] = useState<string[]>([]);

  useEffect(() => {
    if (!prefix) return;
    const open1 = (id: string) => setOpen((o) => (o.includes(id) ? o : [...o, id]));
    const sync = () => {
      const id = window.location.hash.slice(1);
      if (id.startsWith(`${prefix}-`)) open1(id);
    };
    // A second click on the same segment leaves the hash unchanged, so `hashchange` does not fire.
    const onClick = (e: MouseEvent) => {
      const a = (e.target as Element).closest<HTMLAnchorElement>(`a[href^="#${prefix}-"]`);
      if (a) open1(a.getAttribute("href")!.slice(1));
    };
    sync();
    window.addEventListener("hashchange", sync);
    document.addEventListener("click", onClick);
    return () => {
      window.removeEventListener("hashchange", sync);
      document.removeEventListener("click", onClick);
    };
  }, [prefix]);

  return (
    <Accordion type="multiple" value={open} onValueChange={setOpen} className={className}>
      {children}
    </Accordion>
  );
}
