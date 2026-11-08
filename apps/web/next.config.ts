import path from "node:path";
import type { NextConfig } from "next";

const config: NextConfig = {
  // `make guide` and `smoke:demo` run a second web app on demo data beside the owner's one.
  distDir: process.env.AGENTSI_WEB_DIST_DIR ?? ".next",
  devIndicators: process.env.AGENTSI_WEB_DIST_DIR ? false : undefined,
  serverExternalPackages: ["pg"],
  transpilePackages: ["@agentsi/core"],
  agentRules: false,
  turbopack: { root: path.resolve(import.meta.dirname, "../..") },
};

export default config;
