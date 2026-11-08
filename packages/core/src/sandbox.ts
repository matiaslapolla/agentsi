import { existsSync, readdirSync, realpathSync, statSync, type Dirent } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, sep } from "node:path";
import { run, type ExecResult } from "./exec.ts";
import { dataRoot } from "./paths.ts";

export const BWRAP = "bwrap";

/** The `node_modules` folders of a checkout, without the ones nested in another `node_modules`. */
export function nodeModulesDirs(dir: string): string[] {
  const found: string[] = [];
  const walk = (d: string) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      if (!e.isDirectory() || e.name === ".git") continue;
      if (e.name === "node_modules") found.push(join(d, e.name));
      else walk(join(d, e.name));
    }
  };
  if (existsSync(dir)) walk(dir);
  return found.sort();
}

/** Folders of the home dir that hold the owner's keys, tokens and tool settings. */
const SECRET_DIRS = [".ssh", ".aws", ".gnupg", ".config", ".claude"];
/** Folders inside the secret folders that commands still read: git takes the commit identity from `~/.config/git`. */
const KEPT_DIRS = [".config/git"];

/** Paths that a sandboxed command must not read, and the paths inside them that it still reads. */
export interface Mask {
  hide: string[];
  keep: string[];
}

export const NO_MASK: Mask = { hide: [], keep: [] };

const kind = (p: string): "dir" | "file" | null => {
  const s = statSync(p, { throwIfNoEntry: false });
  return s?.isDirectory() ? "dir" : s ? "file" : null;
};

/** The entries of `dir`, or none when the owner of the process cannot read it. */
function entries(dir: string): Dirent[] {
  try {
    return readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }
}

/**
 * The `.env*` files of a checkout, outside `node_modules`, `.git` and `.worktrees`. A `.env*` symlink gives its own
 * path and the file it points to: bwrap cannot mount on a symlink, so `bwrapArgs` hides the target.
 */
export function envFiles(dir: string): string[] {
  const found = new Set<string>();
  const walk = (d: string) => {
    for (const e of entries(d)) {
      const path = join(d, e.name);
      if (e.isDirectory() && !["node_modules", ".git", ".worktrees"].includes(e.name)) walk(path);
      else if (e.isFile() && e.name.startsWith(".env")) found.add(path);
      else if (e.isSymbolicLink() && e.name.startsWith(".env") && kind(path) === "file") found.add(path).add(realpathSync(path));
    }
  };
  if (existsSync(dir)) walk(dir);
  return [...found].sort();
}

/** True when `path` is `dir` or one of its parent folders. */
const holds = (path: string, dir: string): boolean => dir === path || dir.startsWith(path.endsWith(sep) ? path : path + sep);

/**
 * The mask of a story on `repo`: the secret folders, the data root and the `.env*` files of `repo`.
 * A path that is `repo`, holds it or holds its worktrees stays visible: a worktree reads the git data of `repo`
 * through its `.git` file.
 */
export function storyMask(repo: string, home = homedir(), root = dataRoot): Mask {
  const hide = [...SECRET_DIRS.map((d) => join(home, d)), root, ...envFiles(repo)];
  return {
    hide: hide.filter((p) => !holds(p, repo) && !holds(p, join(repo, ".worktrees"))),
    keep: KEPT_DIRS.map((d) => join(home, d)),
  };
}

/**
 * bwrap arguments that run `command` with only `worktree` writable: a read-only root, the `readOnly` folders
 * bound read-only again inside the worktree, a private /tmp, /dev and /proc, and no network.
 * Each hidden folder that exists reads as an empty read-only folder. Each hidden file is bound to /dev/null on a
 * nodev mount, so a read of it fails with permission denied. bwrap cannot mount on a symlink, so each masked path
 * goes to bwrap as its real path.
 * /run is hidden too: it holds the sockets of docker, dbus and the owner's agents, and a read-only bind
 * does not stop a connect to a socket.
 */
export function bwrapArgs(worktree: string, command: string, readOnly: string[] = [], mask: Mask = NO_MASK): string[] {
  const real = (paths: string[]) => [...new Set(paths.map((p) => realpathSync(p)))];
  const dirs = real(mask.hide.filter((p) => kind(p) === "dir"));
  const files = real(mask.hide.filter((p) => kind(p) === "file"));
  const keep = real(mask.keep.filter((p) => kind(p) !== null));
  return [
    "--ro-bind", "/", "/",
    "--dev", "/dev",
    "--proc", "/proc",
    "--tmpfs", "/tmp",
    "--tmpfs", "/run",
    ...dirs.flatMap((d) => ["--tmpfs", d]),
    ...files.flatMap((f) => ["--ro-bind", "/dev/null", f]),
    ...keep.flatMap((d) => ["--ro-bind", d, d]),
    // After the masks, so that a worktree inside a hidden folder stays visible.
    "--bind", worktree, worktree,
    ...readOnly.flatMap((d) => ["--ro-bind", d, d]),
    // The remount is not recursive, so the binds inside a hidden folder keep their own access.
    ...dirs.flatMap((d) => ["--remount-ro", d]),
    "--unshare-net",
    "--die-with-parent",
    "--new-session",
    "--chdir", worktree,
    "--", "sh", "-c", command,
  ];
}

/** Runs `command` in the worktree sandbox; its `node_modules` folders stay read-only, and the paths that `mask` hides read as empty. */
export function sandboxed(worktree: string, command: string, timeoutMs: number, mask: Mask = NO_MASK): Promise<ExecResult> {
  return run(BWRAP, bwrapArgs(worktree, command, nodeModulesDirs(worktree), mask), worktree, timeoutMs);
}

/** True when bwrap is on PATH and can start a sandbox with no network. */
export async function sandboxWorks(): Promise<boolean> {
  return (await run(BWRAP, bwrapArgs(tmpdir(), "true"), tmpdir())).ok;
}
