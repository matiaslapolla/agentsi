import fs from "node:fs";
import { Readable } from "node:stream";
import { audioFile } from "@/lib/workspace";

export const dynamic = "force-dynamic";

export async function GET(req: Request, ctx: { params: Promise<{ product: string; id: string }> }) {
  const { product, id } = await ctx.params;
  const file = audioFile(product, id);
  if (!file) return new Response("not found", { status: 404 });

  const size = fs.statSync(file).size;
  const base = {
    "Content-Type": "audio/webm",
    "Accept-Ranges": "bytes",
    "Cache-Control": "no-cache",
  };

  const range = req.headers.get("range");
  const m = range ? /^bytes=(\d*)-(\d*)$/.exec(range.trim()) : null;
  if (!m || (m[1] === "" && m[2] === "")) {
    return new Response(Readable.toWeb(fs.createReadStream(file)) as ReadableStream, {
      headers: { ...base, "Content-Length": String(size) },
    });
  }

  let start: number;
  let end: number;
  if (m[1] === "") {
    const suffix = Number(m[2]);
    start = Math.max(0, size - suffix);
    end = size - 1;
  } else {
    start = Number(m[1]);
    end = m[2] === "" ? size - 1 : Math.min(Number(m[2]), size - 1);
  }
  if (start >= size || start > end) {
    return new Response(null, { status: 416, headers: { ...base, "Content-Range": `bytes */${size}` } });
  }

  return new Response(Readable.toWeb(fs.createReadStream(file, { start, end })) as ReadableStream, {
    status: 206,
    headers: {
      ...base,
      "Content-Range": `bytes ${start}-${end}/${size}`,
      "Content-Length": String(end - start + 1),
    },
  });
}
