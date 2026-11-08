import { appendFileSync, existsSync, mkdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { dataRoot } from "./paths.ts";

export interface WriteOptions {
  /** Repo root the allowlist is relative to. Defaults to this checkout. */
  root?: string;
  /** Allows `plugin/agents/*.md`; only an approved reflection decision sets it. */
  allowPlugin?: boolean;
}

const PRODUCT = /^[A-Za-z0-9_][\w-]*$/;

function isAllowed(segments: string[], allowPlugin: boolean): boolean {
  const [top, product, area, ...rest] = segments;
  if (top === "workspace" && product && PRODUCT.test(product)) {
    if (area === "memory" || area === "agents") return rest.length > 0;
    return (area === "brief.json" || area === "reflections.jsonl") && rest.length === 0;
  }
  const [, agents, file, ...extra] = segments;
  return allowPlugin && top === "plugin" && agents === "agents" && extra.length === 0 && !!file && file.endsWith(".md") && !file.startsWith("_");
}

/** Resolves symlinks of the deepest existing ancestor, so a link cannot lead a write out of the allowlist. */
function realPath(path: string): string {
  const tail: string[] = [];
  let head = path;
  while (!existsSync(head)) {
    const parent = dirname(head);
    if (parent === head) break;
    tail.unshift(basename(head));
    head = parent;
  }
  return join(realpathSync(head), ...tail);
}

/** The absolute path to touch, or a throw when it falls outside the allowlist. */
function allowedPath(path: string, opts: WriteOptions): string {
  const root = realpathSync(opts.root ?? dataRoot);
  const target = isAbsolute(path) ? resolve(path) : resolve(root, path);
  const rel = relative(root, realPath(target));
  if (rel === "" || rel.startsWith("..") || isAbsolute(rel) || !isAllowed(rel.split(sep), opts.allowPlugin ?? false)) {
    throw new Error(`Path "${path}" is outside the workspace write allowlist.`);
  }
  return target;
}

export function writeAllowed(path: string, content: string, opts: WriteOptions = {}): void {
  const target = allowedPath(path, opts);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, content);
}

export function appendAllowed(path: string, content: string, opts: WriteOptions = {}): void {
  const target = allowedPath(path, opts);
  mkdirSync(dirname(target), { recursive: true });
  appendFileSync(target, content);
}

export function removeAllowed(path: string, opts: WriteOptions = {}): void {
  rmSync(allowedPath(path, opts), { force: true });
}
