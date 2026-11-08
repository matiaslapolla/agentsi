export type DecisionStatus = "pending" | "approved" | "rejected";

export type Mark = "accepted" | "changed" | "rejected";

export interface Source {
  type: "commit" | "issue" | "path" | "url" | "story";
  ref: string;
}

export interface RolePriority {
  title: string;
  why: string;
  sources: Source[];
}

export interface RoleAnswer {
  model: string;
  answer: string;
  priorities: RolePriority[];
  concerns: string[];
  memory_notes: string[];
}

export interface Priority {
  title: string;
  why: string;
  sources: Source[];
  owner_role: string;
  /** Id of the brief objective the priority serves; null when the product has no brief. */
  objective?: string | null;
  mark: Mark | null;
  is_new: boolean;
}

export interface Minutes {
  id: string;
  product: string;
  scope: string | null;
  question: string;
  created_at: string;
  roles: Record<string, RoleAnswer>;
  priorities: Priority[];
  disagreements: { topic: string; positions: { role: string; view: string }[] }[];
  decisions: { text: string; why: string; action?: string; status?: DecisionStatus; decided_at?: string; priorities?: number[] }[];
  script: { role: string; text: string }[];
  check: { passed: boolean; failures: string[]; first_failures?: string[]; repaired: boolean };
  status: "valid" | "invalid";
}

export interface UsageRow {
  role: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_read: number;
  cache_write: number;
  cost_usd: number;
  duration_ms: number;
}
