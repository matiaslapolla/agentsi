import { runStdio } from "../../../mcp/src/index.ts";
import type { Command } from "../command.ts";

export const mcp: Command = {
  name: "mcp",
  summary: "Run the MCP server on stdio",
  usage: "agentsi mcp",
  async run() {
    await runStdio();
  },
};
