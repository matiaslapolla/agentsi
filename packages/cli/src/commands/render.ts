import { findStoredMinutes, renderAudio } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";

export const render: Command = {
  name: "render",
  summary: "Render the audio of a meeting",
  usage: "agentsi render <minutes-id>",
  async run(argv) {
    const id = argv[0];
    if (!id) throw new UsageError("Missing <minutes-id>");
    console.log(await renderAudio(await findStoredMinutes(id)));
  },
};
