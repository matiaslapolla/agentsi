import fs from "node:fs/promises";
import path from "node:path";
import { CODE_ROOT } from "@/lib/env";

export async function GET() {
  const text = await fs.readFile(path.join(CODE_ROOT, "docs/guide/README.md"), "utf8").catch(() => null);
  if (text === null) return new Response("Guía no encontrada: docs/guide/README.md.", { status: 404 });
  return new Response(text, {
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}
