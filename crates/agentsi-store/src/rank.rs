//! A port of `ts_rank` with the default weights and no normalization
//! (`src/backend/utils/adt/tsrank.c` of Postgres 17). FTS5 finds the
//! matches. This rank orders them as the Postgres search did, because the
//! `bm25` of FTS5 puts short commits before long docs.
//!
//! The tokenizer is close to the `simple` parser: lowercase runs of letters
//! and digits. A hyphenated word gives the whole word, then each part, as
//! the parser does.

use std::collections::HashMap;

/// The weight of a title position (`A`) and of a body position (`B`).
const TITLE: f32 = 1.0;
const BODY: f32 = 0.4;
/// The largest position of a tsvector, and the most positions of one lexeme.
const MAXENTRYPOS: u32 = (1 << 14) - 1;
const MAXNUMPOS: usize = 255;

/// The lexemes of `text`, in order.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '-')) {
        let parts: Vec<&str> = word.split('-').filter(|p| !p.is_empty()).collect();
        let compound = parts.len() > 1
            && !word.starts_with('-')
            && !word.ends_with('-')
            && !word.contains("--");
        if compound {
            out.push(word.to_lowercase());
        }
        out.extend(parts.iter().map(|p| p.to_lowercase()));
    }
    out
}

type Positions = Vec<(u32, f32)>;

/// The positions of each of `terms` in `to_tsvector(title) || to_tsvector(body)`.
fn positions(title: &str, body: &str, terms: &[String]) -> Vec<Positions> {
    let mut found: Vec<Positions> = vec![Vec::new(); terms.len()];
    let index: HashMap<&str, usize> = terms
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i))
        .collect();
    let title_tokens = tokens(title);
    // The body positions start after the largest title position.
    let shift = title_tokens.len().min(MAXENTRYPOS as usize) as u32;
    for (part, weight, offset) in [(title_tokens, TITLE, 0u32), (tokens(body), BODY, shift)] {
        let mut per: Vec<usize> = vec![0; terms.len()];
        for (n, tok) in part.iter().enumerate() {
            let Some(&i) = index.get(tok.as_str()) else {
                continue;
            };
            if per[i] >= MAXNUMPOS {
                continue;
            }
            let pos = (n as u32 + 1 + offset).min(MAXENTRYPOS);
            if found[i].last().is_some_and(|&(p, _)| p == pos) {
                continue;
            }
            found[i].push((pos, weight));
            per[i] += 1;
        }
    }
    found
}

fn word_distance(w: u32) -> f32 {
    if w > 100 {
        return 1e-30;
    }
    (1.0 / (1.005 + 0.05 * ((w as f32 / 1.5 - 2.0) as f64).exp())) as f32
}

fn rank_or(pos: &[Positions]) -> f32 {
    let mut res: f32 = 0.0;
    for p in pos.iter().filter(|p| !p.is_empty()) {
        let mut resj: f32 = 0.0;
        let mut wjm: f32 = -1.0;
        let mut jm = 0usize;
        for (j, &(_, w)) in p.iter().enumerate() {
            resj += w / ((j + 1) * (j + 1)) as f32;
            if w > wjm {
                wjm = w;
                jm = j;
            }
        }
        res += (wjm + resj - wjm / ((jm + 1) * (jm + 1)) as f32) / 1.644_934;
    }
    if !pos.is_empty() {
        res /= pos.len() as f32;
    }
    res
}

fn rank_and(pos: &[Positions]) -> f32 {
    let mut res: f32 = -1.0;
    for i in 0..pos.len() {
        for k in 0..i {
            for &(pi, wi) in &pos[i] {
                for &(pk, wk) in &pos[k] {
                    let dist = pi.abs_diff(pk);
                    if dist == 0 {
                        continue;
                    }
                    let curw = ((wi * wk * word_distance(dist)) as f64).sqrt() as f32;
                    res = if res < 0.0 {
                        curw
                    } else {
                        (1.0 - (1.0 - res as f64) * (1.0 - curw as f64)) as f32
                    };
                }
            }
        }
    }
    res
}

/// `ts_rank` of a document for the unique query `terms`. `and` is true when
/// the top operator of the query is AND or a phrase.
pub fn ts_rank(title: &str, body: &str, terms: &[String], and: bool) -> f32 {
    if terms.is_empty() {
        return 0.0;
    }
    let pos = positions(title, body, terms);
    let res = if and && terms.len() >= 2 {
        rank_and(&pos)
    } else {
        rank_or(&pos)
    };
    if res < 0.0 { 1e-20 } else { res }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tokens_follow_the_simple_parser() {
        assert_eq!(
            tokens("AgentSI web-UI, día 2"),
            vec!["agentsi", "web-ui", "web", "ui", "día", "2"]
        );
    }

    // The values come from Postgres 17: select ts_rank(setweight(to_tsvector('simple', t), 'A')
    // || setweight(to_tsvector('simple', b), 'B'), q).
    #[test]
    fn ts_rank_gives_the_values_of_postgres() {
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        let r = ts_rank(
            "Voice mode",
            "The pill speaks each reply",
            &terms(&["voice"]),
            false,
        );
        assert!(near(r, 0.607_927_1), "{r}");
        let r = ts_rank("Voice mode", "voice voice", &terms(&["voice"]), false);
        assert!(near(r, 0.695_738_8), "{r}");
        let r = ts_rank("Web UI", "the web ui", &terms(&["ui", "web"]), true);
        assert!(near(r, 0.999_188_5), "{r}");
    }
}
