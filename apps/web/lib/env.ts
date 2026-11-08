import path from "node:path";
import { DEFAULT_DATABASE_URL, resolveDataRoot } from "@agentsi/core";

export const CODE_ROOT = path.resolve(process.cwd(), "../..");
export const ROOT = resolveDataRoot(CODE_ROOT).root;
export const DATABASE_URL = process.env.AGENTSI_DATABASE_URL ?? DEFAULT_DATABASE_URL;
export const CLI_BIN = path.join(CODE_ROOT, "packages/cli/bin/agentsi.mjs");
export const WORKSPACE = path.join(ROOT, "workspace");
