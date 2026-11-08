import { readFileSync, writeFileSync } from "node:fs";
import type { Product, Registry } from "./registry.ts";

/** JSON on one line in the registry format: `{ "k": v }` and `["a", "b"]`; undefined values drop out. */
const oneLine = (v: unknown): string =>
  Array.isArray(v)
    ? `[${v.map(oneLine).join(", ")}]`
    : v !== null && typeof v === "object"
      ? `{ ${Object.entries(v)
          .filter(([, x]) => x !== undefined)
          .map(([k, x]) => `${JSON.stringify(k)}: ${oneLine(x)}`)
          .join(", ")} }`
      : JSON.stringify(v);

function checked(text: string, expected: Registry): string {
  const actual = JSON.stringify(JSON.parse(text));
  if (actual !== JSON.stringify(expected)) throw new Error("Registry edit did not produce the expected products.");
  return text;
}

/** Rewrites the product's own line with `patch` applied; other keys keep their order, every other line stays. */
export function updateProduct(text: string, id: string, patch: Partial<Omit<Product, "id">>): string {
  const registry = JSON.parse(text) as Registry;
  const product = registry.products.find((p) => p.id === id);
  if (!product) throw new Error(`Unknown product "${id}"`);
  const updated: Product = JSON.parse(JSON.stringify({ ...product, ...patch }));
  const lines = text.split("\n");
  const at = lines.findIndex((l) => l.includes(`"id": ${JSON.stringify(id)}`));
  const line = at < 0 ? undefined : /^(\s*)\{ "id":.*\}(,?)\s*$/.exec(lines[at]!);
  if (!line) throw new Error(`Product "${id}" is not one line in the registry.`);
  lines[at] = `${line[1]}${oneLine(updated)}${line[2]}`;
  return checked(lines.join("\n"), { ...registry, products: registry.products.map((p) => (p.id === id ? updated : p)) });
}

/** Appends a product after the last one, in the one-product-per-line format. */
export function addProduct(text: string, product: Product): string {
  const registry = JSON.parse(text) as Registry;
  if (registry.products.some((p) => p.id === product.id)) throw new Error(`Product "${product.id}" is already registered.`);
  const lines = text.split("\n");
  const last = lines.findLastIndex((l) => /^\s*\{ "id":/.test(l));
  if (last < 0) throw new Error("Registry has no one-line products to append to.");
  const indent = /^\s*/.exec(lines[last]!)![0];
  lines[last] = lines[last]!.replace(/,?\s*$/, ",");
  lines.splice(last + 1, 0, `${indent}${oneLine(product)}`);
  return checked(lines.join("\n"), { ...registry, products: [...registry.products, JSON.parse(JSON.stringify(product))] });
}

export function editRegistryFile(file: string, edit: (text: string) => string): void {
  writeFileSync(file, edit(readFileSync(file, "utf8")));
}
