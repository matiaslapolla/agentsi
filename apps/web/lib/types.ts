import type { Source } from "@agentsi/core";

export type MarkValue = "accepted" | "changed" | "rejected";

export type { Source };

export interface Priority {
  title: string;
  why: string;
  sources: Source[];
  owner_role?: string;
  objective?: string | null;
  mark?: MarkValue | null;
  is_new?: boolean;
}

export interface RoleAnswer {
  model?: string;
  answer?: string;
  priorities?: Priority[];
  concerns?: string[];
  memory_notes?: string[];
}

export interface Minutes {
  id: string;
  product: string;
  scope?: string | null;
  question: string;
  created_at?: string;
  roles?: Record<string, RoleAnswer>;
  priorities?: Priority[];
  disagreements?: { topic: string; positions: { role: string; view: string }[] }[];
  decisions?: { text: string; why?: string; action?: string; status?: "pending" | "approved" | "rejected"; decided_at?: string }[];
  script?: { role: string; text: string }[];
  check?: { passed: boolean; failures?: string[]; repaired?: boolean };
  status?: "valid" | "invalid";
  /** Set by the list when the minutes is in the archive record. */
  archived?: boolean;
}

export interface UsageRow {
  role: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_read: number;
  cache_write: number;
  cost_usd: number;
  duration_ms?: number;
}

export interface Timing {
  idx: number;
  role: string;
  text: string;
  start: number;
  end: number;
}

export interface MarkSummary {
  accepted: number;
  changed: number;
  rejected: number;
  unmarked: number;
  fresh: number;
}

export interface UsageTotals {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cost: number;
  rows: number;
}
