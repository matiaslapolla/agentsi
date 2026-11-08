import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { decision, isGuarded } from "./push-guard.mjs";

test("asks for git push and gh pr create, in the forms an agent writes", () => {
  for (const c of [
    "git push",
    "git push origin main",
    "git -C /tmp/repo push --force",
    "git -c core.editor=true push",
    "cd app && git push -u origin feat/x",
    "pnpm test; git push",
    "FOO=1 git push",
    "bash -c 'git push origin main'",
    "gh pr create --title x --body y",
    "gh -R org/repo pr create",
    'bash -lc "git push"',
    "(git push)",
    "{ git push; }",
    "if true; then git push; fi",
    "time git push",
    "timeout 5 git push origin main",
    "env -i git push",
    "xargs git push",
    'eval "git push"',
    "git \\\npush",
    'echo "$(git push)"',
    "echo $(git push)",
    "echo `git push`",
    'echo "`git push origin main`"',
    'echo "$(echo $(git push))"',
    'echo "$(echo `git push`)"',
    'echo "a $(cd x && git push) b"',
    "echo '$(date)' $(git push)",
    "echo $(gh pr create --title x)",
    `echo "it's $(git push)"`,
    'echo "it\'s `git push`"',
    `echo "x'" "$(git push)"`,
    'echo "$(echo ")"; git push)"',
    "git $'push'",
  ]) assert.equal(isGuarded(c), true, c);
});

test("does not ask for other commands, or for a push named inside a string", () => {
  for (const c of [
    "git status",
    "git commit -m 'git push later'",
    "git log --oneline",
    'echo "run git push when ready"',
    "gh pr view 12",
    "gh pr list",
    "git pull",
    "pnpm push-notes",
    "echo hi # git push",
    "# git push",
    "echo '$(git push)'",
    "echo '`git push`'",
    'echo "git pushing"',
    'echo "$(git status)"',
    "echo $(date) `whoami`",
    "",
  ]) assert.equal(isGuarded(c), false, c);
});

test("the answer is an ask decision with a reason, and null when nothing is guarded", () => {
  const d = decision("git push");
  assert.equal(d?.hookSpecificOutput.permissionDecision, "ask");
  assert.match(d?.hookSpecificOutput.permissionDecisionReason ?? "", /push/);
  assert.equal(decision("git status"), null);
});

test("the hook script reads the payload from stdin and prints the decision", () => {
  const run = (command) =>
    spawnSync(process.execPath, [new URL("./push-guard.mjs", import.meta.url).pathname], {
      input: JSON.stringify({ tool_name: "Bash", tool_input: { command } }),
      encoding: "utf8",
    });
  assert.equal(JSON.parse(run("git push origin main").stdout).hookSpecificOutput.permissionDecision, "ask");
  assert.equal(run("git status").stdout, "");
  const bad = spawnSync(process.execPath, [new URL("./push-guard.mjs", import.meta.url).pathname], { input: "not json", encoding: "utf8" });
  assert.equal(bad.status, 0);
  assert.equal(bad.stdout, "");
});

test("the hook script works when the plugin directory is a symlink", () => {
  const dir = mkdtempSync(join(tmpdir(), "push-guard-"));
  try {
    symlinkSync(new URL(".", import.meta.url).pathname, join(dir, "hooks"));
    const r = spawnSync(process.execPath, [join(dir, "hooks", "push-guard.mjs")], {
      input: JSON.stringify({ tool_input: { command: "git push" } }),
      encoding: "utf8",
    });
    assert.equal(JSON.parse(r.stdout).hookSpecificOutput.permissionDecision, "ask");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
