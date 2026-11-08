import { readFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { check as runCheck, type Minutes } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";

export const check: Command = {
  name: "check",
  summary: "Run the pass/fail check on a minutes file",
  usage: "agentsi check <minutes.json> [--roles pm,dev]",
  async run(argv) {
    const { values, positionals } = parseArgs({
      args: argv,
      allowPositionals: true,
      options: { roles: { type: "string" } },
    });
    const file = positionals[0];
    if (!file) throw new UsageError("Missing <minutes.json>");
    const minutes = JSON.parse(readFileSync(file, "utf8")) as Minutes;
    const result = await runCheck(minutes, { roles: values.roles?.split(",").filter(Boolean) });
    console.log(JSON.stringify(result, null, 2));
    return result.passed ? 0 : 1;
  },
};
