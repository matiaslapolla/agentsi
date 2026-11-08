import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { codeRoot } from "@agentsi/core";
import { testHome } from "./test-home.ts";

const bin = fileURLToPath(new URL("../bin/agentsi.mjs", import.meta.url));
const git = (cwd: string, ...args: string[]) => execFileSync("git", args, { cwd, encoding: "utf8" }).trim();

test("with AGENTSI_HOME, the CLI reads the registry there, and the code checkout stays as it was", () => {
  const stories = { test: "node --test {files}", gate: ["true"] };
  const h = testHome([{ id: "demo", name: "Demo", roles: ["pm", "dev"], stories }]);
  try {
    const codeBefore = git(codeRoot, "status", "--porcelain");
    const run = (...args: string[]) => spawnSync(process.execPath, [bin, ...args], { encoding: "utf8", env: h.env, input: "" });

    assert.equal(run("whoami", "--home").stdout.trim(), `${h.home}  (AGENTSI_HOME)`);
    assert.equal(JSON.parse(run("whoami", "--cwd", h.repo).stdout).product, "demo");
    // Stories live in Postgres, so a story command writes nothing to the data root.
    assert.equal(git(h.home, "status", "--porcelain"), "");
    assert.equal(git(codeRoot, "status", "--porcelain"), codeBefore);
  } finally {
    h.cleanup();
  }
});

test("the code checkout does not ignore workspace/, so the default data root can still commit its data", () => {
  for (const path of ["workspace/demo/minutes/m.json", "agentsi.config.json"]) {
    assert.equal(spawnSync("git", ["check-ignore", "-q", path], { cwd: codeRoot }).status, 1, `${path} is ignored`);
  }
});
