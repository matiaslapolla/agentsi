import { actionLine, archivedDecisionProduct, decide as applyDecision } from "@agentsi/core";
import { migrationPending } from "../context.ts";
import { UsageError, type Command } from "../command.ts";

export const decide: Command = {
  name: "decide",
  summary: "Approve or reject a pending decision",
  usage: "agentsi decide <decision-id> approve|reject",
  async run(argv) {
    const [id, verdict] = argv;
    if (!id || !/^\d+$/.test(id) || (verdict !== "approve" && verdict !== "reject")) {
      throw new UsageError(this.usage);
    }
    if (await archivedDecisionProduct(Number(id))) {
      console.error(`Decision ${id} is archived. Run agentsi restore ${id} first, then decide.`);
      return 1;
    }
    if (await migrationPending()) return 1;
    const res = await applyDecision(Number(id), verdict === "approve" ? "approved" : "rejected");
    if (res.commit.error) console.error(`Commit skipped: ${res.commit.error}`);
    if (res.status === "stale") {
      console.error(`Decision ${id} not applied: the target changed since the proposal, so it is now stale.`);
      return 1;
    }
    console.log(`Decision ${id} ${res.status}${res.minutesId ? ` (${res.minutesId})` : ""}.`);
    const action = actionLine(res);
    if (action) console.log(action);
  },
};
