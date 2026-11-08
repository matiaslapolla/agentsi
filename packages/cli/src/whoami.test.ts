import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { testHome } from "./test-home.ts";

const bin = fileURLToPath(new URL("../bin/agentsi.mjs", import.meta.url));

test("whoami prints the default owner, then the owner block of the registry", () => {
  const h = testHome([{ id: "demo", name: "Demo", roles: ["pm"] }]);
  try {
    const whoami = () => JSON.parse(spawnSync(process.execPath, [bin, "whoami", "--cwd", h.repo], { encoding: "utf8", env: h.env }).stdout);
    assert.deepEqual(whoami().owner, { name: "the owner", language: "English" });

    const file = join(h.home, "agentsi.config.json");
    const registry = JSON.parse(readFileSync(file, "utf8"));
    writeFileSync(file, JSON.stringify({ owner: { name: "Ana", language: "Rioplatense Spanish" }, ...registry }));
    assert.deepEqual(whoami().owner, { name: "Ana", language: "Rioplatense Spanish" });
  } finally {
    h.cleanup();
  }
});
