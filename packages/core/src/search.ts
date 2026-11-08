import { getPool } from "./db.ts";

export interface SearchHit {
  kind: string;
  ref: string;
  scope: string | null;
  title: string;
  rank: number;
  snippet: string;
}

export interface SearchOptions {
  kind?: string;
  limit?: number;
  anyTerm?: boolean;
}

export async function search(product: string, query: string, opts: SearchOptions = {}): Promise<SearchHit[]> {
  const tsquery = opts.anyTerm
    ? "to_tsquery('simple', array_to_string(tsvector_to_array(to_tsvector('simple', $2)), ' | '))"
    : "websearch_to_tsquery('simple', $2)";
  const { rows } = await getPool().query(
    `select d.kind, d.ref, d.scope, d.title, ts_rank(d.tsv, q) as rank,
            ts_headline('simple', coalesce(d.body, ''), q, 'StartSel="", StopSel="", MaxFragments=1, MaxWords=20, MinWords=8') as snippet
     from documents d, ${tsquery} q
     where d.product = $1 and d.tsv @@ q and (d.kind = $3 or ($3::text is null and d.kind <> 'memory'))
     order by rank desc, d.updated_at desc
     limit $4`,
    [product, query, opts.kind ?? null, opts.limit ?? 10],
  );
  return rows.map((r) => ({ ...r, rank: Number(r.rank) }));
}

export interface DocRecord {
  product: string;
  scope: string | null;
  kind: string;
  ref: string;
  title: string;
  body: string;
  updated_at: Date;
}

export async function getDoc(product: string, kind: string, ref: string): Promise<DocRecord | null> {
  const prefix = kind === "commit" && /^[0-9a-f]{7,40}$/i.test(ref) ? `${ref.toLowerCase()}%` : null;
  const { rows } = await getPool().query<DocRecord>(
    `select product, scope, kind, ref, title, body, updated_at from documents
     where product = $1 and kind = $2 and (ref = $3 or ref like $4)
     order by ref = $3 desc, updated_at desc limit 1`,
    [product, kind, ref, prefix],
  );
  return rows[0] ?? null;
}
