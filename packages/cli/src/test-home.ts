import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Product } from "@agentsi/core";

/** A throwaway data root, a git repo with a registry, so CLI tests never read the owner's data. */
export function testHome(products: Omit<Product, "paths" | "status">[]) {
  const base = mkdtempSync(join(tmpdir(), "agentsi-home-"));
  const home = join(base, "data");
  const repo = join(base, "product");
  mkdirSync(repo);
  const registry = products.map((p) => ({ id: p.id, name: p.name, paths: [repo], status: "active", roles: p.roles, ...(p.stories ? { stories: p.stories } : {}) }));
  mkdirSync(home);
  writeFileSync(join(home, "agentsi.config.json"), `{\n  "products": [\n${registry.map((p) => `    ${JSON.stringify(p)}`).join(",\n")}\n  ]\n}\n`);
  for (const p of products) mkdirSync(join(home, "workspace", p.id), { recursive: true });
  const user = { GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t" };
  execFileSync("git", ["init", "-q"], { cwd: home });
  execFileSync("git", ["add", "."], { cwd: home });
  execFileSync("git", ["commit", "-qm", "init"], { cwd: home, env: { ...process.env, ...user } });
  return {
    home,
    repo,
    env: { ...process.env, ...user, AGENTSI_HOME: home, AGENTSI_DATABASE_URL: "postgres://x:x@127.0.0.1:1/x" },
    cleanup: () => rmSync(base, { recursive: true, force: true }),
  };
}
