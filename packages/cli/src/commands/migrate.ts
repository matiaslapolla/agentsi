import { migrate as runMigrate } from "@agentsi/core";
import type { Command } from "../command.ts";

export const migrate: Command = {
  name: "migrate",
  summary: "Apply pending SQL migrations",
  usage: "agentsi migrate",
  async run() {
    const applied = await runMigrate();
    console.log(applied.length ? `Applied: ${applied.join(", ")}` : "Database up to date.");
  },
};
