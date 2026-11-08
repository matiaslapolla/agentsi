//! Full-text search on FTS5, the port of `packages/core/src/search.ts`.

use crate::error::Result;
use crate::rank;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub scope: Option<String>,
    pub title: Option<String>,
    pub rank: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOptions<'a> {
    /// Only this kind. Without it, each kind except `memory`.
    pub kind: Option<&'a str>,
    /// 10 when `None`.
    pub limit: Option<u32>,
    /// Match any word of the query, in place of each word.
    pub any_term: bool,
}

/// The documents of `product` that match `query`, best first. FTS5 finds
/// the matches, and `rank::ts_rank` orders them as the Postgres search did:
/// rank, then the newest `updated_at`.
pub fn search(
    c: &Connection,
    product: &str,
    query: &str,
    opts: &SearchOptions,
) -> Result<Vec<SearchHit>> {
    let parsed = if opts.any_term {
        parse_any_term(query)
    } else {
        parse_web(query)
    };
    let Some(parsed) = parsed else {
        return Ok(Vec::new());
    };
    let mut stmt = c.prepare_cached(
        "select d.id, d.kind, d.ref, d.scope, d.title, coalesce(d.title, ''), coalesce(d.body, ''), d.updated_at
         from documents_fts join documents d on d.id = documents_fts.rowid
         where documents_fts match ?1 and d.product = ?2
           and (d.kind = ?3 or (?3 is null and d.kind <> 'memory'))",
    )?;
    let mut ranked: Vec<(f32, Option<String>, i64, SearchHit)> = stmt
        .query_map(params![parsed.fts, product, opts.kind], |r| {
            let title: String = r.get(5)?;
            let body: String = r.get(6)?;
            let rank = rank::ts_rank(&title, &body, &parsed.terms, parsed.and);
            let hit = SearchHit {
                kind: r.get(1)?,
                reference: r.get(2)?,
                scope: r.get(3)?,
                title: r.get(4)?,
                rank: f64::from(rank),
                snippet: String::new(),
            };
            Ok((rank, r.get(7)?, r.get(0)?, hit))
        })?
        .collect::<rusqlite::Result<_>>()?;
    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    ranked.truncate(opts.limit.unwrap_or(10) as usize);
    let mut snippet = c.prepare_cached(
        "select coalesce(snippet(documents_fts, 1, '', '', '', 20), '') from documents_fts
         where documents_fts match ?1 and rowid = ?2",
    )?;
    ranked
        .into_iter()
        .map(|(_, _, id, mut hit)| {
            hit.snippet = snippet.query_row(params![parsed.fts, id], |r| r.get(0))?;
            Ok(hit)
        })
        .collect()
}

/// A query in its two forms: the FTS5 expression that finds the matches,
/// and the unique lexemes with the top operator, for the rank.
struct Parsed {
    fts: String,
    terms: Vec<String>,
    and: bool,
}

fn unique(words: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for w in words {
        if !seen.contains(&w) {
            seen.push(w);
        }
    }
    seen
}

/// An FTS5 string: the tokenizer splits it into words, as a phrase.
fn quoted(word: &str) -> Option<String> {
    word.chars()
        .any(char::is_alphanumeric)
        .then(|| format!("\"{}\"", word.replace('"', "\"\"")))
}

/// The FTS5 form of `websearch_to_tsquery`: words are AND, `"..."` is a
/// phrase, `or` between two operands is OR, and a leading `-` is NOT. An
/// `or` in any other place is a word, as in Postgres. `None` when no word is left.
fn parse_web(query: &str) -> Option<Parsed> {
    enum Tok {
        Term(String),
        Not(String),
        Or,
    }
    let mut raw = Vec::new();
    let mut rest = query;
    while let Some(start) = rest.find(|c: char| !c.is_whitespace()) {
        rest = &rest[start..];
        let negated = rest.starts_with('-');
        let body = if negated { &rest[1..] } else { rest };
        let (text, phrase, after) = match body.strip_prefix('"') {
            Some(inner) => match inner.find('"') {
                Some(end) => (&inner[..end], true, &inner[end + 1..]),
                None => (inner, true, ""),
            },
            None => {
                let end = body.find(char::is_whitespace).unwrap_or(body.len());
                (&body[..end], false, &body[end..])
            }
        };
        rest = after;
        if quoted(text).is_some() {
            raw.push((text, phrase, negated));
        }
    }
    let mut toks = Vec::new();
    let mut words = Vec::new();
    for (k, &(text, phrase, negated)) in raw.iter().enumerate() {
        let after_operand = matches!(toks.last(), Some(Tok::Term(_) | Tok::Not(_)));
        let operator = !phrase && !negated && text.eq_ignore_ascii_case("or");
        if operator && after_operand && k + 1 < raw.len() {
            toks.push(Tok::Or);
            continue;
        }
        let q = quoted(text).expect("raw keeps only quotable words");
        words.extend(rank::tokens(text));
        toks.push(if negated { Tok::Not(q) } else { Tok::Term(q) });
    }
    let mut positive = String::new();
    let mut joiner = "";
    let mut has_or = false;
    let mut negative = Vec::new();
    for tok in toks {
        match tok {
            Tok::Or => joiner = " OR ",
            Tok::Term(q) => {
                if !positive.is_empty() {
                    has_or |= !joiner.is_empty();
                    positive.push_str(if joiner.is_empty() { " AND " } else { joiner });
                }
                positive.push_str(&q);
                joiner = "";
            }
            Tok::Not(q) => negative.push(q),
        }
    }
    if positive.is_empty() {
        return None;
    }
    let fts = if negative.is_empty() {
        positive
    } else {
        format!("({positive}) NOT {}", negative.join(" NOT "))
    };
    let terms = unique(words.into_iter());
    // The top operator of websearch_to_tsquery is OR when the query has an
    // `or`, because AND binds closer. Else it is AND or a phrase.
    Some(Parsed {
        fts,
        and: !has_or && terms.len() >= 2,
        terms,
    })
}

/// Any word of the query, the `anyTerm` option of the TypeScript search.
fn parse_any_term(query: &str) -> Option<Parsed> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter_map(quoted)
        .collect();
    let terms = unique(rank::tokens(query).into_iter());
    (!words.is_empty()).then(|| Parsed {
        fts: words.join(" OR "),
        terms,
        and: false,
    })
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Doc {
    pub product: String,
    pub scope: Option<String>,
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub updated_at: Option<String>,
}

/// One document. For a commit, a hex ref of 7 to 40 characters also matches
/// as a prefix, and an exact ref wins.
pub fn get_doc(c: &Connection, product: &str, kind: &str, reference: &str) -> Result<Option<Doc>> {
    let hex =
        (7..=40).contains(&reference.len()) && reference.chars().all(|ch| ch.is_ascii_hexdigit());
    let prefix = (kind == "commit" && hex).then(|| format!("{}%", reference.to_lowercase()));
    Ok(c.query_row(
        "select product, scope, kind, ref, title, body, updated_at from documents
         where product = ?1 and kind = ?2 and (ref = ?3 or ref like ?4)
         order by ref = ?3 desc, updated_at desc limit 1",
        params![product, kind, reference, prefix],
        |r| {
            Ok(Doc {
                product: r.get(0)?,
                scope: r.get(1)?,
                kind: r.get(2)?,
                reference: r.get(3)?,
                title: r.get(4)?,
                body: r.get(5)?,
                updated_at: r.get(6)?,
            })
        },
    )
    .optional()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn web_query(query: &str) -> Option<String> {
        parse_web(query).map(|p| p.fts)
    }

    fn any_term_query(query: &str) -> Option<String> {
        parse_any_term(query).map(|p| p.fts)
    }

    #[test]
    fn web_query_maps_the_websearch_syntax() {
        // Postgres 17: 'voice' & 'mode' & 'or', 'or' & 'voice' & 'mode', 'voice' | 'or' & 'mode'.
        let tail = parse_web("voice mode or").unwrap();
        assert_eq!(
            (tail.fts.as_str(), tail.and),
            ("\"voice\" AND \"mode\" AND \"or\"", true)
        );
        let head = parse_web("or voice mode").unwrap();
        assert_eq!(
            (head.fts.as_str(), head.and),
            ("\"or\" AND \"voice\" AND \"mode\"", true)
        );
        let twice = parse_web("voice or or mode").unwrap();
        assert_eq!(
            (twice.fts.as_str(), twice.and),
            ("\"voice\" OR \"or\" AND \"mode\"", false)
        );
        assert!(!parse_web("voice or mode").unwrap().and);
        assert_eq!(
            web_query("voice mode").as_deref(),
            Some("\"voice\" AND \"mode\"")
        );
        assert_eq!(
            web_query("\"voice mode\" pill").as_deref(),
            Some("\"voice mode\" AND \"pill\"")
        );
        assert_eq!(
            web_query("voice or pill").as_deref(),
            Some("\"voice\" OR \"pill\"")
        );
        assert_eq!(
            web_query("voice -pill").as_deref(),
            Some("(\"voice\") NOT \"pill\"")
        );
        assert_eq!(
            web_query("say \"x\"\"").as_deref(),
            Some("\"say\" AND \"x\"")
        );
        assert_eq!(web_query("-pill"), None);
        assert_eq!(web_query("  ?? "), None);
        assert_eq!(
            any_term_query("data-import, rust").as_deref(),
            Some("\"data\" OR \"import\" OR \"rust\"")
        );
    }
}
