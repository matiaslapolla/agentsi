//! The token side of Parakeet TDT: the vocabulary file, the greedy decoder and the
//! text from tokens. No ONNX Runtime is here, so the tests run with no model.
//!
//! The steps follow `NemoConformerTdt` of onnx-asr 0.12.0 (MIT).

use super::Error;
use std::collections::HashMap;

/// The decoder emits at most this many tokens at one frame before it moves on.
pub const MAX_TOKENS_PER_STEP: usize = 10;

/// `vocab.txt`: one `token id` pair on each line. The word start mark is U+2581.
pub struct Vocab {
    tokens: HashMap<i64, String>,
    pub blank: i64,
}

impl Vocab {
    pub fn parse(text: &str) -> Result<Vocab, Error> {
        let mut tokens = HashMap::new();
        let mut blank = None;
        for line in text.lines().filter(|l| !l.is_empty()) {
            let (token, id) = line
                .rsplit_once(' ')
                .ok_or_else(|| Error::Model(format!("bad vocabulary line {line:?}")))?;
            let id: i64 = id
                .parse()
                .map_err(|_| Error::Model(format!("bad vocabulary id in {line:?}")))?;
            if token == "<blk>" {
                blank = Some(id);
            }
            tokens.insert(id, token.replace('\u{2581}', " "));
        }
        let blank = blank.ok_or_else(|| Error::Model("the vocabulary has no <blk>".into()))?;
        Ok(Vocab { tokens, blank })
    }

    /// The number of tokens. The joint output holds this many token scores, then the duration scores.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The text of a token sequence.
    pub fn text(&self, ids: &[i64]) -> String {
        let joined: String = ids
            .iter()
            .filter_map(|id| self.tokens.get(id))
            .map(String::as_str)
            .collect();
        tidy(&joined)
    }
}

/// Removes the space at the start and the space before a mark. onnx-asr does it
/// with `\A\s|\s\B|(\s)\b`: a space stays only before a word character. This keeps it
/// also before an opening mark or a currency sign, so "Dije ¿qué" and "cost $5" stay apart.
fn tidy(text: &str) -> String {
    let word = |c: char| {
        c.is_alphanumeric()
            || matches!(
                c,
                '_' | '¿'
                    | '¡'
                    | '('
                    | '['
                    | '{'
                    | '$'
                    | '€'
                    | '£'
                    | '¥'
                    | '«'
                    | '“'
                    | '‘'
                    | '#'
                    | '@'
            )
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_whitespace() {
            out.push(c);
        } else if i > 0 && chars.get(i + 1).is_some_and(|&next| word(next)) {
            out.push(' ');
        }
    }
    out
}

/// What the joint network answers for one frame and one previous token.
pub struct Joint {
    /// The token scores, `Vocab::len` values.
    pub tokens: Vec<f32>,
    /// The duration scores: how many frames the decoder may skip.
    pub durations: Vec<f32>,
}

/// The decoder and joint network, with the state of the prediction network in it.
pub trait Joiner {
    /// Scores frame `t` after `previous` (`None` before the first token).
    fn step(&mut self, frame: usize, previous: Option<i64>) -> Result<Joint, Error>;
    /// Keeps the state of the last `step`. The decoder calls it only after a token.
    fn commit(&mut self);
}

fn argmax(values: &[f32]) -> usize {
    values
        .iter()
        .enumerate()
        .fold((0, f32::NEG_INFINITY), |best, (i, &v)| {
            if v > best.1 { (i, v) } else { best }
        })
        .0
}

/// Greedy TDT decoding of `frames` encoder frames.
pub fn greedy(joiner: &mut dyn Joiner, frames: usize, blank: i64) -> Result<Vec<i64>, Error> {
    let mut tokens: Vec<i64> = Vec::new();
    let (mut t, mut emitted) = (0, 0);
    while t < frames {
        let joint = joiner.step(t, tokens.last().copied())?;
        let token = argmax(&joint.tokens) as i64;
        let step = argmax(&joint.durations);
        if token != blank {
            joiner.commit();
            tokens.push(token);
            emitted += 1;
        }
        if step > 0 {
            t += step;
            emitted = 0;
        } else if token == blank || emitted == MAX_TOKENS_PER_STEP {
            t += 1;
            emitted = 0;
        }
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOCAB: &str = "<unk> 0\n<blk> 3\n\u{2581}hello 1\n\u{2581}wor 2\nld 4\n. 5\n";

    #[test]
    fn the_vocabulary_reads_tokens_and_the_blank() {
        let v = Vocab::parse(VOCAB).unwrap();
        assert_eq!(v.blank, 3);
        assert_eq!(v.len(), 5 + 1);
        assert!(Vocab::parse("a 1\n").is_err());
        assert!(Vocab::parse("<blk> x\n").is_err());
    }

    #[test]
    fn tokens_make_text_with_no_space_before_a_mark() {
        let v = Vocab::parse(VOCAB).unwrap();
        assert_eq!(v.text(&[1, 2, 4, 5]), "hello world.");
        assert_eq!(v.text(&[]), "");
    }

    #[test]
    fn tidy_keeps_a_space_only_before_a_word() {
        assert_eq!(tidy(" a  b ,c"), "a b,c");
        assert_eq!(tidy("¿ Qué"), "¿ Qué");
        assert_eq!(tidy("Dije ¿qué hora es?"), "Dije ¿qué hora es?");
        assert_eq!(tidy("cost $5 (ok) , done ."), "cost $5 (ok), done.");
    }

    /// Frame `t` answers with the token of `script[t]`; a duration of `skip` frames.
    struct Script {
        script: Vec<(i64, usize)>,
        committed: usize,
        stepped: usize,
    }

    impl Joiner for Script {
        fn step(&mut self, frame: usize, _: Option<i64>) -> Result<Joint, Error> {
            self.stepped += 1;
            let (token, skip) = self.script[frame.min(self.script.len() - 1)];
            let mut tokens = vec![0.0; 6];
            tokens[token as usize] = 1.0;
            let mut durations = vec![0.0; 5];
            durations[skip] = 1.0;
            Ok(Joint { tokens, durations })
        }

        fn commit(&mut self) {
            self.committed += 1;
        }
    }

    #[test]
    fn blanks_move_one_frame_and_tokens_use_their_duration() {
        // frame 0: token 1, skip 2 -> frame 2: blank -> frame 3: token 2, skip 1 -> frame 4 ends.
        let mut s = Script {
            script: vec![(1, 2), (0, 0), (3, 0), (2, 1)],
            committed: 0,
            stepped: 0,
        };
        let out = greedy(&mut s, 4, 3).unwrap();
        assert_eq!(out, vec![1, 2]);
        assert_eq!(s.committed, 2);
        assert_eq!(s.stepped, 3);
    }

    #[test]
    fn a_frame_that_emits_with_no_skip_stays_until_the_limit() {
        let mut s = Script {
            script: vec![(1, 0)],
            committed: 0,
            stepped: 0,
        };
        let out = greedy(&mut s, 1, 3).unwrap();
        assert_eq!(out.len(), MAX_TOKENS_PER_STEP);
    }
}
