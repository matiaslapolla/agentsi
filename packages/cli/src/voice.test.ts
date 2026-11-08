import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { testHome } from "./test-home.ts";

const bin = fileURLToPath(new URL("../bin/agentsi.mjs", import.meta.url));

test("voice on and off write and remove the state file of the current session only", () => {
  const h = testHome([{ id: "demo", name: "Demo", roles: ["pm"] }]);
  try {
    const voice = (action: string, session?: string) =>
      spawnSync(process.execPath, [bin, "voice", action], { encoding: "utf8", env: { ...h.env, CLAUDE_CODE_SESSION_ID: session ?? "" } });
    const file = (session: string) => join(h.home, "voice", `${session}.json`);

    assert.equal(voice("on").status, 1, "a shell outside Claude Code has no session");
    const on = voice("on", "session-a");
    assert.equal(on.status, 0, on.stderr);
    assert.match(on.stdout, /on for this session/);
    assert.ok(JSON.parse(readFileSync(file("session-a"), "utf8")).code, "the state file names the code checkout");
    assert.equal(existsSync(file("session-b")), false);
    assert.match(voice("status", "session-b").stdout, /off for this session/);

    assert.equal(voice("off", "session-a").status, 0);
    assert.equal(existsSync(file("session-a")), false);
    assert.match(voice("sideways", "session-a").stderr, /agentsi voice on\|off\|status/);
  } finally {
    h.cleanup();
  }
});
