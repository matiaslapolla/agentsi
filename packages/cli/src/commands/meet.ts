import { parseArgs } from "node:util";
import { renderAudio, runMeeting, type MeetingEvent } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { assertNotArchived, migrationPending, resolveProduct } from "../context.ts";

const WEB = "http://127.0.0.1:3361";

function progress(e: MeetingEvent): void {
  const who = e.role ?? e.stage;
  if (e.phase === "start") {
    if (e.stage !== "check") console.error(`[${e.stage}] ${who} started${e.note ? ` (${e.note})` : ""}`);
    return;
  }
  if (e.stage === "index" || e.stage === "check") {
    console.error(`[${e.stage}] ${e.note ?? "done"}`);
    return;
  }
  const secs = `${(e.seconds ?? 0).toFixed(1)}s`;
  if (e.error) console.error(`[${e.stage}] ${who} FAILED after ${secs}: ${e.error}`);
  else console.error(`[${e.stage}] ${who} finished in ${secs}, $${(e.cost_usd ?? 0).toFixed(4)}, input ${e.input_tokens} tokens`);
}

export const meet: Command = {
  name: "meet",
  summary: "Run a meeting",
  usage: 'agentsi meet [--product <id>] --question "<q>" [--roles pm,dev] [--audio]',
  async run(argv) {
    const { values } = parseArgs({
      args: argv,
      options: {
        product: { type: "string" },
        question: { type: "string" },
        roles: { type: "string" },
        audio: { type: "boolean" },
      },
    });
    if (!values.question) throw new UsageError("Missing --question");
    const { product, scope } = resolveProduct(values.product);
    await assertNotArchived(product);
    if (await migrationPending()) return 1;
    const started = Date.now();
    const res = await runMeeting({
      product: product.id,
      scope,
      question: values.question,
      roles: values.roles?.split(",").map((r) => r.trim()).filter(Boolean),
      onEvent: progress,
    });
    const m = res.minutes;

    if (values.audio) {
      console.error("[audio] rendering");
      try {
        const dir = await renderAudio(m);
        console.error(`[audio] done: ${dir}`);
      } catch (err) {
        console.error(`WARNING: audio failed, minutes were kept: ${(err as Error).message}`);
      }
    }

    console.log(`Minutes: ${m.id}`);
    console.log(`Status: ${m.status}${m.check.repaired ? " (repaired)" : ""}`);
    if (!m.check.passed) for (const f of m.check.failures) console.log(`  check failure: ${f}`);
    console.log("Top priorities:");
    for (const [i, p] of m.priorities.slice(0, 3).entries()) console.log(`  ${i + 1}. ${p.title} (${p.owner_role})`);
    console.log(`Cost: $${res.totalCostUsd.toFixed(4)} in ${((Date.now() - started) / 1000).toFixed(0)}s`);
    console.log(`Web: ${WEB}/p/${m.product}/m/${m.id}`);
    console.log(`Call: ${WEB}/p/${m.product}/m/${m.id}/call`);
    return m.status === "valid" ? 0 : 1;
  },
};
