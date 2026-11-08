import assert from "node:assert/strict";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { git } from "./exec.ts";
import { dataRoot } from "./paths.ts";
import { bwrapArgs, envFiles, nodeModulesDirs, sandboxed, sandboxWorks, storyMask } from "./sandbox.ts";

test("the bwrap arguments bind only the worktree writable and cut the network", () => {
  const args = bwrapArgs("/r/.worktrees/S001-x", "pnpm lint", ["/r/.worktrees/S001-x/node_modules"]);
  const joined = args.join(" ");
  assert.match(joined, /^--ro-bind \/ \/ --dev \/dev --proc \/proc --tmpfs \/tmp --tmpfs \/run --bind \/r\/\.worktrees\/S001-x \/r\/\.worktrees\/S001-x /);
  assert.match(joined, /--ro-bind \/r\/\.worktrees\/S001-x\/node_modules \/r\/\.worktrees\/S001-x\/node_modules/);
  for (const flag of ["--unshare-net", "--die-with-parent", "--new-session"]) assert.ok(args.includes(flag), flag);
  assert.deepEqual(args.slice(-4), ["--", "sh", "-c", "pnpm lint"]);
  assert.ok(joined.indexOf("--bind /r/.worktrees/S001-x") < joined.indexOf("--ro-bind /r/.worktrees/S001-x/node_modules"));
});

test("nodeModulesDirs finds the node_modules of each package, not the nested ones", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-sandbox-nm-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const d of ["node_modules/a/node_modules", "packages/core/node_modules", "src"]) mkdirSync(join(dir, d), { recursive: true });
  assert.deepEqual(nodeModulesDirs(dir), [join(dir, "node_modules"), join(dir, "packages/core/node_modules")]);
});

test("a sandboxed command writes in the worktree only, cannot change node_modules and has no network", async (t) => {
  const wt = mkdtempSync(join(tmpdir(), "agentsi-sandbox-wt-"));
  const outside = join(homedir(), `.agentsi-sandbox-probe-${process.pid}`);
  t.after(() => {
    rmSync(wt, { recursive: true, force: true });
    rmSync(outside, { force: true });
  });
  if (!(await sandboxWorks())) return t.skip("bwrap is missing or cannot start a sandbox here");
  mkdirSync(join(wt, "node_modules"));
  writeFileSync(join(wt, "node_modules", "tool.js"), "original\n");

  assert.ok((await sandboxed(wt, "echo in > inside.txt", 10_000)).ok);
  assert.equal(readFileSync(join(wt, "inside.txt"), "utf8"), "in\n");

  const write = await sandboxed(wt, `echo out > ${outside}`, 10_000);
  assert.equal(write.ok, false);
  assert.match(write.stderr, /Read-only file system/);
  assert.ok(!existsSync(outside));

  assert.equal((await sandboxed(wt, "echo changed > node_modules/tool.js", 10_000)).ok, false);
  assert.equal(readFileSync(join(wt, "node_modules", "tool.js"), "utf8"), "original\n");

  const net = await sandboxed(wt, `node -e "require('net').connect(80, '1.1.1.1').on('connect', () => process.exit(0)).on('error', (e) => { console.error(e.code); process.exit(3); })"`, 10_000);
  assert.equal(net.ok, false);
  assert.match(net.stderr, /ENETUNREACH|EHOSTUNREACH|ECONNREFUSED/);
});

test("the bwrap arguments hide existing masked folders and files, keep the kept folders and skip missing paths", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-sandbox-mask-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const d of [".ssh", ".config/git"]) mkdirSync(join(dir, d), { recursive: true });
  writeFileSync(join(dir, ".env"), "TOKEN=x\n");
  const [ssh, config, kept, env, missing] = [".ssh", ".config", ".config/git", ".env", ".aws"].map((p) => join(dir, p));
  const args = bwrapArgs("/r/wt", "true", [], { hide: [ssh!, config!, env!, missing!], keep: [kept!] }).join(" ");
  assert.ok(args.includes(`--tmpfs ${ssh} --tmpfs ${config} --ro-bind /dev/null ${env} --ro-bind ${kept} ${kept} --bind /r/wt /r/wt`));
  assert.ok(args.includes(`--remount-ro ${ssh} --remount-ro ${config} --unshare-net`));
  assert.ok(!args.includes(missing!));
});

test("the story mask hides the secret folders of the home dir, the data root and the .env files of the repo", (t) => {
  const repo = mkdtempSync(join(tmpdir(), "agentsi-sandbox-env-"));
  t.after(() => rmSync(repo, { recursive: true, force: true }));
  for (const f of [".env", "apps/web/.env.local", "node_modules/x/.env", ".worktrees/S001-x/.env", "src/env.ts"]) {
    mkdirSync(join(repo, f, ".."), { recursive: true });
    writeFileSync(join(repo, f), "");
  }
  assert.deepEqual(envFiles(repo), [join(repo, ".env"), join(repo, "apps/web/.env.local")]);
  const mask = storyMask(repo);
  const home = homedir();
  for (const path of [join(home, ".ssh"), join(home, ".config"), dataRoot, ...envFiles(repo)]) assert.ok(mask.hide.includes(path), path);
  assert.deepEqual(mask.keep, [join(home, ".config/git")]);
});

test("envFiles skips a folder that it cannot read", (t) => {
  const repo = mkdtempSync(join(tmpdir(), "agentsi-sandbox-env-"));
  const locked = join(repo, "locked");
  t.after(() => {
    chmodSync(locked, 0o700);
    rmSync(repo, { recursive: true, force: true });
  });
  writeFileSync(join(repo, ".env"), "");
  mkdirSync(locked);
  writeFileSync(join(locked, ".env"), "");
  chmodSync(locked, 0o000);
  assert.deepEqual(envFiles(repo), [join(repo, ".env")]);
});

test("the story mask never hides the repo or a folder that holds it", (t) => {
  const base = mkdtempSync(join(tmpdir(), "agentsi-sandbox-hold-"));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const home = join(base, "home");
  const repo = join(home, ".config", "product");
  mkdirSync(repo, { recursive: true });
  for (const root of [repo, base, join(repo, ".worktrees")]) {
    const mask = storyMask(repo, home, root);
    for (const hidden of mask.hide) assert.ok(hidden !== repo && !repo.startsWith(`${hidden}/`) && hidden !== join(repo, ".worktrees"), `${root} hides ${hidden}`);
    assert.ok(!mask.hide.includes(join(home, ".config")));
    assert.ok(mask.hide.includes(join(home, ".ssh")));
  }
});

test("envFiles gives a .env symlink and the file it points to", (t) => {
  const base = mkdtempSync(join(tmpdir(), "agentsi-sandbox-link-"));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const repo = join(base, "repo");
  mkdirSync(repo);
  writeFileSync(join(base, "secret.env"), "SECRET=1\n");
  symlinkSync(join(base, "secret.env"), join(repo, ".env"));
  symlinkSync(join(base, "missing.env"), join(repo, ".env.dangling"));
  assert.deepEqual(envFiles(repo), [realpathSync(join(base, "secret.env")), join(repo, ".env")].sort());
});

/**
 * A main repo with a story worktree, under the home dir: the sandbox hides /tmp, so a probe there would
 * prove nothing.
 */
async function probeRepo(t: { after: (fn: () => void) => void }) {
  const base = mkdtempSync(join(homedir(), ".agentsi-sandbox-probe-"));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const repo = join(base, "repo");
  mkdirSync(repo);
  writeFileSync(join(repo, ".gitignore"), ".worktrees/\n");
  for (const args of [["init", "-q", "-b", "main"], ["add", "."], ["-c", "user.name=T", "-c", "user.email=t@example.com", "commit", "-qm", "init"]]) {
    assert.ok((await git(repo, ...args)).ok, args.join(" "));
  }
  const wt = join(repo, ".worktrees", "S001-x");
  assert.ok((await git(repo, "worktree", "add", "-q", "-b", "story/S001", wt)).ok);
  return { base, repo, wt };
}

test("a sandboxed command cannot write to the main .git, through ../ or through a symlink out of the worktree", async (t) => {
  if (!(await sandboxWorks())) return t.skip("bwrap is missing or cannot start a sandbox here");
  const { base, repo, wt } = await probeRepo(t);
  const config = readFileSync(join(repo, ".git", "config"), "utf8");

  const gitWrite = await sandboxed(wt, `echo x >> ${join(repo, ".git", "config")}; touch ${join(repo, ".git", "probe")}`, 10_000);
  assert.equal(gitWrite.ok, false);
  assert.match(gitWrite.stderr, /Read-only file system/);
  assert.equal(readFileSync(join(repo, ".git", "config"), "utf8"), config);
  assert.ok(!existsSync(join(repo, ".git", "probe")));

  const up = await sandboxed(wt, "echo x > ../escape.txt", 10_000);
  assert.equal(up.ok, false);
  assert.match(up.stderr, /Read-only file system/);
  assert.ok(!existsSync(join(repo, ".worktrees", "escape.txt")));

  const link = await sandboxed(wt, `ln -s ${base} out && echo x > out/escape.txt`, 10_000);
  assert.equal(link.ok, false);
  assert.match(link.stderr, /Read-only file system/);
  assert.ok(!existsSync(join(base, "escape.txt")));
});

test("a sandboxed command cannot connect to a unix socket in the runtime dir of the owner", async (t) => {
  if (!(await sandboxWorks())) return t.skip("bwrap is missing or cannot start a sandbox here");
  const runtime = process.env.XDG_RUNTIME_DIR;
  if (!runtime || !existsSync(runtime)) return t.skip("XDG_RUNTIME_DIR is not set");
  const wt = mkdtempSync(join(tmpdir(), "agentsi-sandbox-wt-"));
  const path = join(runtime, `agentsi-sandbox-probe-${process.pid}.sock`);
  let connections = 0;
  const server = createServer((s) => (connections++, s.end()));
  await new Promise<void>((resolve) => server.listen(path, resolve));
  t.after(async () => {
    await new Promise((resolve) => server.close(resolve));
    rmSync(wt, { recursive: true, force: true });
  });

  const probe = `node -e "require('net').connect('${path}').on('connect', () => process.exit(0)).on('error', (e) => { console.error(e.code); process.exit(3); })"`;
  const r = await sandboxed(wt, probe, 10_000);
  assert.equal(r.ok, false);
  assert.match(r.stderr, /ENOENT/);
  assert.equal(connections, 0);
});

test("a sandboxed command sees ~/.ssh and the data root as empty folders, cannot read a .env file and still reads ~/.config/git", async (t) => {
  if (!(await sandboxWorks())) return t.skip("bwrap is missing or cannot start a sandbox here");
  const { base, repo, wt } = await probeRepo(t);
  const home = join(base, "home");
  const root = join(base, "data");
  for (const [file, content] of [[".ssh/id_ed25519", "KEY"], [".config/gh/hosts.yml", "TOKEN"], [".config/git/config", "[user]\n"], ["../data/agentsi.config.json", "{}"]] as const) {
    mkdirSync(join(home, file, ".."), { recursive: true });
    writeFileSync(join(home, file), content);
  }
  writeFileSync(join(repo, ".env"), "SECRET=1\n");
  const mask = storyMask(repo, home, root);

  for (const [dir, listing] of [[join(home, ".ssh"), ""], [root, ""], [join(home, ".config"), "git\n"]]) {
    const ls = await sandboxed(wt, `ls -A ${dir}`, 10_000, mask);
    assert.ok(ls.ok, ls.stderr);
    assert.equal(ls.stdout, listing, dir);
  }

  const env = await sandboxed(wt, `cat ${join(repo, ".env")}`, 10_000, mask);
  assert.equal(env.ok, false);
  assert.doesNotMatch(env.stdout, /SECRET/);

  const kept = await sandboxed(wt, `cat ${join(home, ".config/git/config")}`, 10_000, mask);
  assert.equal(kept.stdout, "[user]\n");

  assert.equal((await sandboxed(wt, `touch ${join(home, ".ssh", "x")}`, 10_000, mask)).ok, false);
  assert.ok(!existsSync(join(home, ".ssh", "x")));
});

test("with the data root equal to the repo, git still works in the worktree of a sandboxed command", async (t) => {
  if (!(await sandboxWorks())) return t.skip("bwrap is missing or cannot start a sandbox here");
  const { base, repo, wt } = await probeRepo(t);
  const r = await sandboxed(wt, "git status --short", 10_000, storyMask(repo, join(base, "home"), repo));
  assert.ok(r.ok, r.stderr);
});

test("a sandboxed command cannot read a .env symlink or the file it points to", async (t) => {
  if (!(await sandboxWorks())) return t.skip("bwrap is missing or cannot start a sandbox here");
  const { base, repo, wt } = await probeRepo(t);
  const secret = join(base, "secret.env");
  writeFileSync(secret, "SECRET=1\n");
  symlinkSync(secret, join(repo, ".env"));
  symlinkSync("../secret.env", join(repo, ".env.relative"));
  const mask = storyMask(repo, join(base, "home"), join(base, "data"));
  for (const path of [join(repo, ".env"), join(repo, ".env.relative"), secret]) {
    const r = await sandboxed(wt, `cat ${path}`, 10_000, mask);
    assert.equal(r.ok, false, path);
    assert.doesNotMatch(r.stdout, /SECRET/, path);
  }
});
