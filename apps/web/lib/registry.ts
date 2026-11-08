import fs from "node:fs";
import path from "node:path";
import type { StoryConfigBlock } from "@agentsi/core";
import { ROOT, WORKSPACE } from "./env.ts";

export interface Product {
  id: string;
  name: string;
  paths: string[];
  status: "active" | "planned";
  roles: string[];
  /** Present only when Stories mode is on for the product. */
  stories?: StoryConfigBlock;
}

const DEMO: Product = {
  id: "_demo",
  name: "Demo",
  paths: [],
  status: "active",
  roles: ["pm", "social", "dev", "qa", "scout"],
};

export function readRegistry(): Product[] {
  let products: Product[] = [];
  try {
    const raw = JSON.parse(fs.readFileSync(path.join(ROOT, "agentsi.config.json"), "utf8"));
    products = Array.isArray(raw.products) ? raw.products : [];
  } catch {
    products = [];
  }
  if (fs.existsSync(path.join(WORKSPACE, DEMO.id))) products = [...products, DEMO];
  return products;
}

export function findProduct(id: string): Product | undefined {
  return readRegistry().find((p) => p.id === id);
}
