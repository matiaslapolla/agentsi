import fs from "node:fs/promises";
import path from "node:path";
import { CODE_ROOT } from "@/lib/env";

const TYPES = { svg: "image/svg+xml", png: "image/png" } as const;

export async function GET(_: Request, { params }: { params: Promise<{ file: string }> }) {
  const { file } = await params;
  const ext = file.match(/^[\w-]+\.(svg|png)$/)?.[1] as keyof typeof TYPES | undefined;
  if (!ext) return new Response(null, { status: 404 });
  const body = await fs.readFile(path.join(CODE_ROOT, "docs/guide/img", file)).catch(() => null);
  if (!body) return new Response(null, { status: 404 });
  return new Response(body, { headers: { "content-type": TYPES[ext] } });
}
