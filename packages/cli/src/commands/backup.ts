import { parseArgs } from "node:util";
import { backup as runBackup, KEEP_BACKUPS } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";

export const backup: Command = {
  name: "backup",
  summary: "Dump the database and copy the registry into the backups folder of the data root",
  usage: `agentsi backup [--keep <n>]   (default ${KEEP_BACKUPS})`,
  async run(argv) {
    const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { keep: { type: "string" } } });
    if (positionals.length) throw new UsageError(backup.usage);
    const keep = values.keep === undefined ? KEEP_BACKUPS : Number(values.keep);
    const r = await runBackup({ keep });
    console.log(`Dump: ${r.dump}`);
    if (r.registry) console.log(`Registry: ${r.registry}`);
    if (r.removed.length) console.log(`Removed: ${r.removed.join(", ")}`);
  },
};
