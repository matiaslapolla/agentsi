import { parseArgs } from "node:util";
import { applyMark, applyMarks, decide, pendingItems, type Mark, type MarkEntry } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { migrationPending, resolveProduct } from "../context.ts";
import { describe, runWizard } from "./mark-wizard.ts";

const MARKS: Mark[] = ["accepted", "changed", "rejected"];

/** `<minutes-id>:<idx>:<mark>[:new]`. A minutes id holds no colon. */
export function parseSet(spec: string): MarkEntry {
  const [minutesId, idx, verdict, flag, ...extra] = spec.split(":");
  if (!minutesId || !idx || !/^\d+$/.test(idx) || !MARKS.includes(verdict as Mark) || (flag !== undefined && flag !== "new") || extra.length) {
    throw new UsageError(`Bad --set "${spec}". Use <minutes-id>:<idx>:accepted|changed|rejected[:new]`);
  }
  return { minutesId, idx: Number(idx), mark: verdict as Mark, isNew: flag === "new" };
}

export const mark: Command = {
  name: "mark",
  summary: "Mark priorities of a minutes file, or walk through everything pending",
  usage: [
    "agentsi mark <minutes-id> <idx> accepted|changed|rejected [--new]",
    "agentsi mark --set <minutes-id>:<idx>:<mark>[:new] [--set ...]",
    "agentsi mark --pending [--json] [--product <id>]",
    "agentsi mark [--product <id>]   (wizard, needs a terminal)",
  ].join("\n       "),
  async run(argv) {
    const { values, positionals } = parseArgs({
      args: argv,
      allowPositionals: true,
      options: { new: { type: "boolean" }, set: { type: "string", multiple: true }, pending: { type: "boolean" }, json: { type: "boolean" }, product: { type: "string" } },
    });

    if (values.set?.length) {
      if (positionals.length || values.pending || values.new) throw new UsageError(this.usage);
      const commit = await applyMarks(values.set.map(parseSet), { unmarkedOnly: true });
      console.log(`Marked ${values.set.length} ${values.set.length === 1 ? "priority" : "priorities"}.`);
      if (commit.error) console.error(`Commit skipped: ${commit.error}`);
      return;
    }

    if (positionals.length) {
      const [id, idx, verdict] = positionals;
      if (positionals.length !== 3 || !idx || !/^\d+$/.test(idx) || !MARKS.includes(verdict as Mark) || values.pending) throw new UsageError(this.usage);
      const commit = await applyMark(id!, Number(idx), verdict as Mark, values.new ?? false);
      console.log(`Marked ${id} #${idx} ${verdict}${values.new ? " (new)" : ""}.`);
      if (commit.error) console.error(`Commit skipped: ${commit.error}`);
      return;
    }

    if (values.new || (values.json && !values.pending)) throw new UsageError(this.usage);
    if (!values.pending && !process.stdin.isTTY) throw new UsageError(this.usage);
    const { product } = resolveProduct(values.product);
    const { items, decisions_error } = await pendingItems(product.id);
    if (decisions_error) console.error(`decisions_error: ${decisions_error.split("\n")[0]}`);
    if (values.pending) {
      if (values.json) console.log(JSON.stringify(items, null, 2));
      else if (!items.length) console.log("Nothing pending.");
      else items.forEach((item, n) => console.log(`${n + 1}. ${describe(item).join("\n")}`));
      return;
    }
    if (!decisions_error && (await migrationPending())) return 1;
    return runWizard(items, { input: process.stdin, output: process.stdout, error: process.stderr }, {
      applyMarks: (entries) => applyMarks(entries, { unmarkedOnly: true }),
      decide: (id, status) => decide(id, status),
    });
  },
};
