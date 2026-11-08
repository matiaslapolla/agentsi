/** The tab values are URL hashes that already exist, so they keep their Spanish spelling. */
export const DECISIONS_TAB = "decisiones";
export const USAGE_TAB = "uso";

export const MINUTES_TABS = ["priorities", "disagreements", DECISIONS_TAB, "roles", USAGE_TAB] as const;
export type MinutesTab = (typeof MINUTES_TABS)[number];

/** Item anchors (`#p-2`, `#d-0`) come from the meeting progress bar. They select the tab that holds the item. */
const ITEM_PREFIX: Record<string, MinutesTab> = { p: "priorities", d: DECISIONS_TAB };

/** The tab a URL hash selects, or null when the hash names no tab on this page. */
export function tabForHash(hash: string, available: readonly MinutesTab[]): MinutesTab | null {
  const key = hash.replace(/^#/, "");
  const item = /^([a-z])-\d+$/.exec(key);
  const tab = item ? ITEM_PREFIX[item[1]] : (MINUTES_TABS as readonly string[]).includes(key) ? (key as MinutesTab) : undefined;
  return tab && available.includes(tab) ? tab : null;
}
