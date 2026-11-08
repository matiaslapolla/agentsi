import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

const web = path.resolve(import.meta.dirname, "..");

test("the web app has no write component and no write action except finishCall, removeListen, the story actions and the archive and restore actions", () => {
  for (const f of ["decision-buttons.tsx", "mark-controls.tsx", "revert-button.tsx"]) {
    assert.equal(fs.existsSync(path.join(web, "components", f)), false, f);
  }
  const actions = fs.readFileSync(path.join(web, "app", "actions.ts"), "utf8");
  // S012 added archive and restore of minutes, S015 restore of a whole product. Any other new write fails this test.
  assert.deepEqual(
    [...actions.matchAll(/export async function (\w+)/g)].map((m) => m[1]).sort(),
    ["approveStoryAction", "archiveMinutesAction", "discardStoryAction", "finishCall", "removeListen", "requestStoryChangesAction", "restoreMinutesAction", "restoreProductAction"],
  );
  for (const name of ["DecisionButtons", "MarkControls", "RevertButton", "markPriority", "revertReflection"]) {
    assert.equal(new RegExp(`\\b${name}\\b`).test(actions), false, name);
  }
});

test("the front desk skill names agentsi archive and agentsi restore and tells the front desk to run them", () => {
  const skill = fs.readFileSync(path.resolve(import.meta.dirname, "..", "..", "..", "plugin", "skills", "agentsi", "SKILL.md"), "utf8");
  assert.match(skill, /run `agentsi archive <minutes-id>/);
  assert.match(skill, /run `agentsi restore <minutes-id>/);
});

test("every page that lost a button names the command to run", () => {
  const read = (p: string) => fs.readFileSync(path.join(web, p), "utf8");
  assert.match(read("app/p/[product]/page.tsx"), /DecideHint/);
  assert.match(read("app/p/[product]/m/[id]/page.tsx"), /DecideHint/);
  assert.match(read("app/p/[product]/m/[id]/page.tsx"), /MarkState/);
  assert.match(read("app/p/[product]/m/[id]/page.tsx"), /<CliHint>agentsi mark<\/CliHint>/);
  assert.match(read("components/roles-sections.tsx"), /agentsi reflect revert/);
});
