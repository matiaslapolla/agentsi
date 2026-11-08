import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { codeRoot, pluginAgentsDir, resolveDataRoot } from "./paths.ts";

test("resolveDataRoot takes AGENTSI_HOME, then the home file, then the data folder", () => {
  const xdg = mkdtempSync(join(tmpdir(), "agentsi-xdg-"));
  try {
    const env = (extra: NodeJS.ProcessEnv = {}): NodeJS.ProcessEnv => ({ XDG_CONFIG_HOME: xdg, ...extra });
    assert.deepEqual(resolveDataRoot("/code", env()), { root: "/code/data", from: "data folder" });
    mkdirSync(join(xdg, "agentsi"));
    writeFileSync(join(xdg, "agentsi", "home"), "\n");
    assert.deepEqual(resolveDataRoot("/code", env()), { root: "/code/data", from: "data folder" }, "an empty home file is skipped");
    writeFileSync(join(xdg, "agentsi", "home"), "  /data/agentsi  \nignored\n");
    assert.deepEqual(resolveDataRoot("/code", env()), { root: "/data/agentsi", from: "home file" });
    writeFileSync(join(xdg, "agentsi", "home"), "~/agentsi-data\n");
    assert.equal(resolveDataRoot("/code", env()).root, join(homedir(), "agentsi-data"));
    assert.deepEqual(resolveDataRoot("/code", env({ AGENTSI_HOME: "/env/root" })), { root: "/env/root", from: "AGENTSI_HOME" });
    assert.equal(resolveDataRoot("/code", env({ AGENTSI_HOME: "  " })).from, "home file", "an empty AGENTSI_HOME is skipped");
    writeFileSync(join(xdg, "agentsi", "home"), "agentsi-data\n");
    assert.equal(resolveDataRoot("/code", env()).root, join(homedir(), "agentsi-data"), "a relative path is relative to the home dir, not the cwd");
    assert.equal(resolveDataRoot("/code", env({ AGENTSI_HOME: "rel/data" })).root, join(homedir(), "rel/data"));
  } finally {
    rmSync(xdg, { recursive: true, force: true });
  }
});

test("a checkout with its registry at the top and no data folder keeps the old data root", () => {
  const xdg = mkdtempSync(join(tmpdir(), "agentsi-xdg-"));
  const code = mkdtempSync(join(tmpdir(), "agentsi-code-"));
  try {
    writeFileSync(join(code, "agentsi.config.json"), "{}");
    assert.deepEqual(resolveDataRoot(code, { XDG_CONFIG_HOME: xdg }), { root: code, from: "code checkout" });
    mkdirSync(join(code, "data"));
    assert.deepEqual(resolveDataRoot(code, { XDG_CONFIG_HOME: xdg }), { root: join(code, "data"), from: "data folder" });
  } finally {
    rmSync(xdg, { recursive: true, force: true });
    rmSync(code, { recursive: true, force: true });
  }
});

test("pluginAgentsDir uses a root's own global roles, else the code checkout's", () => {
  const root = mkdtempSync(join(tmpdir(), "agentsi-data-"));
  try {
    assert.equal(pluginAgentsDir(root), join(codeRoot, "plugin", "agents"));
    mkdirSync(join(root, "plugin", "agents"), { recursive: true });
    assert.equal(pluginAgentsDir(root), join(root, "plugin", "agents"));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
