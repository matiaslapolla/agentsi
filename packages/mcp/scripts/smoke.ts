import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { fileURLToPath } from "node:url";

const entry = fileURLToPath(new URL("../../cli/bin/agentsi.mjs", import.meta.url));
const transport = new StdioClientTransport({
  command: process.execPath,
  args: [entry, "mcp"],
  cwd: process.cwd(),
  stderr: "inherit",
});
const client = new Client({ name: "smoke", version: "0.0.0" });
await client.connect(transport);

const show = (title: string, res: unknown) => {
  const r = res as { isError?: boolean; content: { text: string }[] };
  const body = r.content.map((c) => c.text).join("\n");
  console.log(`\n=== ${title}${r.isError ? " (isError)" : ""} ===\n${body.length > 900 ? `${body.slice(0, 900)}\n...[${body.length} chars]` : body}`);
  return body;
};

const { tools } = await client.listTools();
console.log("tools:", tools.map((t) => t.name).join(", "));

const hits = show("search roles permisos", await client.callTool({ name: "search", arguments: { query: "roles permisos" } }));
const first = /^1\. \[(\w+)\] ref=(.+)$/m.exec(hits);
if (first) show(`get_doc ${first[1]} ${first[2]}`, await client.callTool({ name: "get_doc", arguments: { kind: first[1], ref: first[2] } }));
show("list_minutes", await client.callTool({ name: "list_minutes", arguments: {} }));
show("pending_decisions", await client.callTool({ name: "pending_decisions", arguments: {} }));
show("role_memory pm", await client.callTool({ name: "role_memory", arguments: { role: "pm" } }));
show("bad product", await client.callTool({ name: "list_minutes", arguments: { product: "nope" } }));

await client.close();
