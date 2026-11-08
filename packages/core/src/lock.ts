import { closeSync, mkdirSync, openSync, rmSync, statSync, writeSync } from "node:fs";
import { dirname } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

/** A lock older than this belongs to a process that died while holding it. */
export const LOCK_STALE_MS = 60_000;
export const LOCK_WAIT_MS = 30_000;

function tryAcquire(file: string, staleMs: number): boolean {
  try {
    const fd = openSync(file, "wx");
    writeSync(fd, String(process.pid));
    closeSync(fd);
    return true;
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code !== "EEXIST") throw err;
    try {
      if (Date.now() - statSync(file).mtimeMs > staleMs) rmSync(file, { force: true });
    } catch {
      // released between the open and the stat
    }
    return false;
  }
}

/** Runs `fn` while holding `file`, created with O_EXCL. Not reentrant: `fn` must not take the same lock. */
export async function withLock<T>(file: string, fn: () => T | Promise<T>, o: { staleMs?: number; waitMs?: number } = {}): Promise<T> {
  mkdirSync(dirname(file), { recursive: true });
  const deadline = Date.now() + (o.waitMs ?? LOCK_WAIT_MS);
  while (!tryAcquire(file, o.staleMs ?? LOCK_STALE_MS)) {
    if (Date.now() > deadline) throw new Error(`Timed out waiting for the lock ${file}`);
    await sleep(25);
  }
  try {
    return await fn();
  } finally {
    rmSync(file, { force: true });
  }
}
