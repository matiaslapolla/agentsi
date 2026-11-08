//! Cuts a phoneme string into batches of at most 510 phonemes, the context of the model.
//! It follows `chunker.py` of kokoro-onnx 0.6.1 (MIT): cut at a sentence mark, then
//! a clause mark, then a space, and as a last resort inside a word. Batches are balanced.

/// The longest batch that the model reads.
pub const MAX_PHONEMES: usize = 510;
const SENTENCE_MARKS: &str = ".!?…";
const CLAUSE_MARKS: &str = ",;:";

/// Splits at white space that follows one of `marks`, or at any white space when `marks` is `None`.
fn split_at<'a>(text: &'a str, marks: Option<&str>) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let after_mark = marks.is_none_or(|m| prev.is_some_and(|p| m.contains(p)));
        if c.is_whitespace() && after_mark {
            let mut end = i + c.len_utf8();
            while let Some(&(j, n)) = chars.peek() {
                if !n.is_whitespace() {
                    break;
                }
                end = j + n.len_utf8();
                chars.next();
            }
            out.push(&text[start..i]);
            start = end;
            prev = None;
            continue;
        }
        prev = Some(c);
    }
    out.push(&text[start..]);
    out
}

fn atoms(text: &str, max: usize, level: usize, out: &mut Vec<String>) {
    if text.chars().count() <= max {
        if !text.is_empty() {
            out.push(text.to_owned());
        }
        return;
    }
    let levels = [Some(SENTENCE_MARKS), Some(CLAUSE_MARKS), None];
    for (index, marks) in levels.iter().enumerate().skip(level) {
        let pieces = split_at(text, *marks);
        if pieces.len() > 1 {
            for piece in pieces {
                atoms(piece.trim(), max, index + 1, out);
            }
            return;
        }
    }
    let chars: Vec<char> = text.chars().collect();
    for slice in chars.chunks(max) {
        out.push(slice.iter().collect());
    }
}

/// Groups atoms into ranges whose joined length stays within `limit`.
fn pack(lengths: &[usize], limit: usize) -> Vec<(usize, usize)> {
    let mut batches = Vec::new();
    let (mut start, mut size) = (0, 0);
    for (index, &length) in lengths.iter().enumerate() {
        let candidate = if index == start {
            length
        } else {
            size + 1 + length
        };
        if candidate > limit && index > start {
            batches.push((start, index));
            start = index;
            size = length;
        } else {
            size = candidate;
        }
    }
    if !lengths.is_empty() {
        batches.push((start, lengths.len()));
    }
    batches
}

/// The batches of `phonemes`, each at most `max` phonemes long.
pub fn split(phonemes: &str, max: usize) -> Vec<String> {
    let mut list = Vec::new();
    atoms(phonemes.trim(), max, 0, &mut list);
    if list.is_empty() {
        return Vec::new();
    }
    let lengths: Vec<usize> = list.iter().map(|a| a.chars().count()).collect();
    let fewest = pack(&lengths, max).len();
    let (mut low, mut high) = (*lengths.iter().max().expect("not empty"), max);
    while low < high {
        let middle = (low + high) / 2;
        if pack(&lengths, middle).len() <= fewest {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    pack(&lengths, low)
        .into_iter()
        .map(|(s, e)| list[s..e].join(" "))
        .collect()
}

/// The silence, in seconds, after a batch that ends with `phonemes`.
pub fn pause_after(phonemes: &str, sentence: f32, clause: f32) -> f32 {
    match phonemes.trim_end().chars().last() {
        Some(c) if SENTENCE_MARKS.contains(c) => sentence,
        Some(c) if CLAUSE_MARKS.contains(c) => clause,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_text_is_one_batch() {
        assert_eq!(split(" ab cd. ", 510), vec!["ab cd."]);
        assert!(split("   ", 510).is_empty());
    }

    #[test]
    fn a_long_text_cuts_at_sentences_and_balances() {
        let text = "aaaa. bbbb. cccc. dddd.";
        assert_eq!(split(text, 12), vec!["aaaa. bbbb.", "cccc. dddd."]);
    }

    #[test]
    fn a_text_with_no_marks_cuts_at_spaces_then_inside_words() {
        assert_eq!(split("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(split("abcdefgh", 3), vec!["abc", "def", "gh"]);
    }

    #[test]
    fn each_batch_fits() {
        let text = "ðə kwˈɪk, bɹˈaʊn fˈɑːks. ".repeat(60);
        for batch in split(&text, MAX_PHONEMES) {
            assert!(batch.chars().count() <= MAX_PHONEMES);
        }
    }

    #[test]
    fn the_pause_follows_the_last_mark() {
        assert_eq!(pause_after("a.", 0.25, 0.1), 0.25);
        assert_eq!(pause_after("a, ", 0.25, 0.1), 0.1);
        assert_eq!(pause_after("a", 0.25, 0.1), 0.0);
    }
}
