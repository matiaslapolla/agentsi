import { execFile } from "node:child_process";

export interface ExecResult {
  ok: boolean;
  stdout: string;
  stderr: string;
}

export function run(cmd: string, args: string[], cwd: string, timeoutMs = 20_000): Promise<ExecResult> {
  return new Promise((resolve) => {
    execFile(
      cmd,
      args,
      { cwd, timeout: timeoutMs, maxBuffer: 64 * 1024 * 1024, env: { ...process.env, GIT_TERMINAL_PROMPT: "0" } },
      (err, stdout, stderr) => resolve({ ok: !err, stdout: String(stdout), stderr: String(stderr) }),
    );
  });
}

export const git = (cwd: string, ...args: string[]) => run("git", args, cwd);

export interface CommitResult {
  committed: boolean;
  error?: string;
}

/**
 * Commits only `paths` (relative to `cwd`) of a code repo or a story worktree; nothing else staged goes in.
 * Never used for the data root: its data lives in Postgres.
 */
export async function commitFiles(cwd: string, paths: string[], message: string): Promise<CommitResult> {
  const add = await git(cwd, "add", "--", ...paths);
  if (!add.ok) return { committed: false, error: add.stderr.trim() };
  const staged = await git(cwd, "diff", "--cached", "--quiet", "--", ...paths);
  if (staged.ok) return { committed: false };
  const res = await git(cwd, "commit", "-m", message, "--", ...paths);
  return res.ok ? { committed: true } : { committed: false, error: (res.stderr || res.stdout).trim() };
}
