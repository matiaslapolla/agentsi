import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { appendAllowed, removeAllowed, writeAllowed } from "./workspace-write.ts";

function withRoot(fn: (root: string) => void): void {
  const root = mkdtempSync(join(tmpdir(), "agentsi-ww-"));
  try {
    fn(root);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

test("writes the allowlisted workspace paths and creates their folders", () => {
  withRoot((root) => {
    for (const path of [
      "workspace/p/memory/dev.json",
      "workspace/p/memory/dev.backend.json",
      "workspace/p/agents/dev.md",
      "workspace/p/brief.json",
      "workspace/p/reflections.jsonl",
    ]) {
      writeAllowed(path, "x", { root });
      assert.equal(readFileSync(join(root, path), "utf8"), "x");
    }
    writeAllowed(join(root, "workspace/p/brief.json"), "y", { root });
    assert.equal(readFileSync(join(root, "workspace/p/brief.json"), "utf8"), "y");
  });
});

test("refuses check code, skills, minutes and anything outside the root", () => {
  withRoot((root) => {
    for (const path of [
      "packages/core/src/check.ts",
      "plugin/skills/agentsi/SKILL.md",
      "workspace/p/minutes/x.json",
      "workspace/p/meetings.jsonl",
      "workspace/p/memory",
      "workspace/p/brief.json/x",
      "workspace/memory/dev.json",
      "workspace/../packages/core/src/check.ts",
      "../outside",
      "agentsi.config.json",
      join(tmpdir(), "elsewhere.txt"),
    ]) {
      assert.throws(() => writeAllowed(path, "x", { root }), /allowlist/, path);
      assert.throws(() => removeAllowed(path, { root }), /allowlist/, path);
    }
    assert.equal(existsSync(join(root, "packages")), false);
  });
});

test("plugin agents need allowPlugin and only *.md directly under plugin/agents", () => {
  withRoot((root) => {
    assert.throws(() => writeAllowed("plugin/agents/dev.md", "x", { root }), /allowlist/);
    writeAllowed("plugin/agents/dev.md", "x", { root, allowPlugin: true });
    assert.equal(readFileSync(join(root, "plugin/agents/dev.md"), "utf8"), "x");
    for (const path of ["plugin/agents/dev.txt", "plugin/agents/sub/dev.md", "plugin/agents/_template.md", "plugin/skills/agentsi/SKILL.md", "plugin/agents"]) {
      assert.throws(() => writeAllowed(path, "x", { root, allowPlugin: true }), /allowlist/, path);
    }
  });
});

test("a symlink cannot lead a write out of the allowlist", () => {
  withRoot((root) => {
    mkdirSync(join(root, "workspace/p"), { recursive: true });
    mkdirSync(join(root, "packages/core/src"), { recursive: true });
    symlinkSync(join(root, "packages/core/src"), join(root, "workspace/p/memory"));
    assert.throws(() => writeAllowed("workspace/p/memory/check.ts", "x", { root }), /allowlist/);
    assert.equal(existsSync(join(root, "packages/core/src/check.ts")), false);

    writeFileSync(join(root, "packages/core/src/check.ts"), "keep");
    symlinkSync(join(root, "packages/core/src/check.ts"), join(root, "workspace/p/brief.json"));
    assert.throws(() => writeAllowed("workspace/p/brief.json", "x", { root }), /allowlist/);
    assert.throws(() => removeAllowed("workspace/p/brief.json", { root }), /allowlist/);
    assert.equal(readFileSync(join(root, "packages/core/src/check.ts"), "utf8"), "keep");
  });
});

test("removeAllowed deletes an allowed file and ignores a missing one", () => {
  withRoot((root) => {
    writeAllowed("workspace/p/agents/dev.md", "x", { root });
    removeAllowed("workspace/p/agents/dev.md", { root });
    assert.equal(existsSync(join(root, "workspace/p/agents/dev.md")), false);
    removeAllowed("workspace/p/agents/dev.md", { root });
  });
});

test("the stories folder is outside the allowlist: stories live in Postgres", () => {
  withRoot((root) => {
    for (const path of [
      "workspace/p/stories/S001-ver-resultado.json",
      "workspace/p/stories/T012-limpiar.md",
      "workspace/p/stories/S001.log",
      "workspace/p/stories/../x",
      "workspace/p/stories",
    ]) {
      assert.throws(() => writeAllowed(path, "x", { root }), /allowlist/, path);
      assert.throws(() => appendAllowed(path, "x", { root }), /allowlist/, path);
    }
  });
});
