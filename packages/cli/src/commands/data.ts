import { resolve } from "node:path";
import { importData } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";

export const data: Command = {
  name: "data",
  summary: "Move data of an old data root into the database",
  usage: "agentsi data import <dir>",
  async run(argv) {
    const [sub, dir, ...extra] = argv;
    if (sub !== "import" || !dir || extra.length) throw new UsageError("Use: agentsi data import <dir>");
    const result = await importData(resolve(dir));
    console.log(
      result.changes
        ? `Imported ${result.stories} stories, ${result.logs} story logs, ${result.memory} memory items, ${result.overlays} overlays, ${result.briefs} briefs, ${result.minutes} minutes, ${result.log} meeting log lines, ${result.snapshots} snapshots, ${result.listens} listens, ${result.reflections} reflections, ${result.archive} archive records and ${result.files} files (registry, audio) from ${resolve(dir)}.`
        : `Nothing to import from ${resolve(dir)}: 0 changes.`,
    );
  },
};
