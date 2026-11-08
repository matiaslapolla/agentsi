//! Text to phonemes with the `espeak-ng` program.
//!
//! espeak-ng is GPL-3.0, so it runs as its own process and no crate links it.
//! The result must equal what kokoro-onnx 0.6.1 gets from phonemizer 3.4 with
//! `preserve_punctuation` and `with_stress`. phonemizer is GPL-3.0 too, so this
//! file copies none of its code. It gives the same output, and the tests check that:
//!
//! 1. Each line is cut at the runs of punctuation and white space.
//! 2. espeak-ng turns each piece into IPA.
//! 3. The punctuation runs go back between the pieces.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// The marks that stay in the phonemes. Kokoro has a token for most of them.
const MARKS: &str = ";:,.!?¡¿—…\"«»“”(){}[]";

/// The variable that names another phonemizer program, for tests and odd installs.
pub const PROGRAM_VAR: &str = "AGENTSI_ESPEAK";
const PROGRAM: &str = "espeak-ng";

/// Turns pieces of text, with no punctuation run, into IPA.
pub trait G2p {
    fn ipa(&self, piece: &str) -> Result<String, String>;

    /// The IPA of each piece, in order. A program may run them at the same time.
    fn ipa_all(&self, pieces: &[&str]) -> Result<Vec<String>, String> {
        pieces.iter().map(|p| self.ipa(p)).collect()
    }
}

/// The `espeak-ng` program.
///
/// The program and phonemizer's library call differ in one way: the program gives a
/// primary stress to a clause that has none, for example a lone "the" between two
/// marks. So each piece also runs after a stressed dummy word. When that output has
/// no primary stress after the dummy, it is the library's answer. In a check of
/// 2796 pieces this matched the library 2794 times. A lone "what" and "when" still differ.
#[derive(Debug, Clone)]
pub struct Espeak {
    program: PathBuf,
    voice: String,
}

/// A noun whose own stress keeps the program from adding one to the piece after it.
const DUMMY: &str = "zebra";
/// The start of the IPA of the dummy word, which ends in ə or ɚ.
const DUMMY_IPA: &str = "zˈiːbɹ";
const PRIMARY: char = 'ˈ';
/// Each piece holds 2 processes and about 6 file handles until it ends. A sentence with
/// hundreds of marks, like a list of file names, would pass a limit of 1024 handles.
const BATCH: usize = 16;

impl Espeak {
    /// `$AGENTSI_ESPEAK`, else `espeak-ng` on the `PATH`. `None` when neither exists.
    pub fn find(voice: &str) -> Option<Espeak> {
        let program = match std::env::var_os(PROGRAM_VAR).filter(|v| !v.is_empty()) {
            Some(p) => PathBuf::from(p),
            None => on_path(PROGRAM)?,
        };
        Some(Espeak::with_program(program, voice))
    }

    pub fn with_program(program: impl Into<PathBuf>, voice: &str) -> Espeak {
        Espeak {
            program: program.into(),
            voice: voice.to_owned(),
        }
    }

    /// Starts one run. The text goes on stdin, so a piece that starts with `-` is not an option.
    fn start(&self, text: &str) -> Result<Child, String> {
        let mut child = Command::new(&self.program)
            .args(["-q", "--ipa", "-v", &self.voice])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", self.program.display()))?;
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let written = stdin.write_all(text.as_bytes());
        drop(stdin);
        if let Err(e) = written {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{}: {e}", self.program.display()));
        }
        Ok(child)
    }

    fn finish(&self, child: Child) -> Result<String, String> {
        let out = child
            .wait_with_output()
            .map_err(|e| format!("{}: {e}", self.program.display()))?;
        if !out.status.success() {
            return Err(format!(
                "{} failed ({}): {}",
                self.program.display(),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        String::from_utf8(out.stdout).map_err(|_| "espeak-ng wrote text that is not UTF-8".into())
    }
}

/// The IPA of the piece from the run after the dummy, when the program kept the dummy apart
/// and the piece has no primary stress there. Else the plain run.
fn choose(plain: String, after_dummy: &str) -> String {
    let words: Vec<&str> = after_dummy.split_whitespace().collect();
    match words.split_first() {
        Some((first, rest))
            if first.starts_with(DUMMY_IPA)
                && !rest.is_empty()
                && !rest.iter().any(|w| w.contains(PRIMARY)) =>
        {
            rest.join(" ")
        }
        _ => plain,
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

impl G2p for Espeak {
    fn ipa(&self, piece: &str) -> Result<String, String> {
        Ok(self.ipa_all(&[piece])?.remove(0))
    }

    /// Starts the runs of up to `BATCH` pieces, two for each piece, before it waits for any.
    fn ipa_all(&self, pieces: &[&str]) -> Result<Vec<String>, String> {
        let mut out = Vec::with_capacity(pieces.len());
        for batch in pieces.chunks(BATCH) {
            out.extend(self.ipa_batch(batch)?);
        }
        Ok(out)
    }
}

impl Espeak {
    fn ipa_batch(&self, pieces: &[&str]) -> Result<Vec<String>, String> {
        let mut children = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let plain = self.start(piece);
            let dummy = self.start(&format!("{DUMMY} {piece}"));
            children.push((plain, dummy));
        }
        let mut out = Vec::with_capacity(pieces.len());
        let mut first_error = None;
        for (plain, dummy) in children {
            let plain = plain.and_then(|c| self.finish(c));
            let dummy = dummy.and_then(|c| self.finish(c));
            match (plain, dummy) {
                (Ok(p), Ok(d)) => out.push(choose(p, &d)),
                (Err(e), _) | (_, Err(e)) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }
}

/// Python's `str.isspace`, which phonemizer's `\s` follows. It also counts U+001C to U+001F.
fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// A `.` or `,` between two ASCII digits is a decimal point and not a mark.
fn is_mark(prev: Option<char>, c: char, next: Option<char>) -> bool {
    let digit = |x: Option<char>| x.is_some_and(|x| x.is_ascii_digit());
    match c {
        '.' | ',' => !(digit(prev) && digit(next)),
        _ => MARKS.contains(c),
    }
}

/// Where a punctuation run sits in its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// The run is the whole line.
    All,
    Begin,
    Inside,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    line: usize,
    text: String,
    place: Place,
}

/// The byte ranges of the punctuation runs of `line`: the longest stretches of
/// white space and marks that hold at least one mark.
fn runs(line: &str) -> Vec<(usize, usize)> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mark_at = |k: usize| {
            is_mark(
                k.checked_sub(1).map(|p| chars[p].1),
                chars[k].1,
                chars.get(k + 1).map(|n| n.1),
            )
        };
        if !is_space(chars[i].1) && !mark_at(i) {
            i += 1;
            continue;
        }
        let start = i;
        let mut has_mark = false;
        while i < chars.len() && (is_space(chars[i].1) || mark_at(i)) {
            has_mark |= mark_at(i);
            i += 1;
        }
        if has_mark {
            let end = chars.get(i).map_or(line.len(), |c| c.0);
            out.push((chars[start].0, end));
        }
    }
    out
}

/// Cuts one line into pieces and runs. A run text is looked up from the left of
/// the rest of the line, so a run text that also occurs earlier, like the `.` of
/// `3.14`, cuts there. phonemizer does the same, and the sound must not change.
fn cut(line: &str, index: usize, pieces: &mut Vec<String>, marks: &mut Vec<Run>) {
    let found = runs(line);
    if found.is_empty() {
        pieces.push(line.to_owned());
        return;
    }
    if found.len() == 1 && found[0] == (0, line.len()) {
        marks.push(Run {
            line: index,
            text: line.to_owned(),
            place: Place::All,
        });
        return;
    }
    let last = found.len() - 1;
    let mut rest = line;
    for (n, &(start, end)) in found.iter().enumerate() {
        let text = &line[start..end];
        let place = if n == 0 && start == 0 {
            Place::Begin
        } else if n == last && end == line.len() {
            Place::End
        } else {
            Place::Inside
        };
        marks.push(Run {
            line: index,
            text: text.to_owned(),
            place,
        });
        match rest.find(text) {
            Some(at) => {
                pieces.push(rest[..at].to_owned());
                rest = &rest[at + text.len()..];
            }
            None => {
                pieces.push(rest.to_owned());
                rest = "";
            }
        }
    }
    pieces.push(rest.to_owned());
}

/// The IPA of one piece as words, each followed by one space. Empty IPA stays empty.
fn words(ipa: &str) -> String {
    ipa.split_whitespace().map(|w| format!("{w} ")).collect()
}

/// Puts the runs back between the phonemized pieces. `pos` counts the pieces that
/// left the queue and is compared with the line number of the next run, as in phonemizer.
fn join(mut pieces: std::collections::VecDeque<String>, marks: Vec<Run>) -> Vec<String> {
    let mut marks: std::collections::VecDeque<Run> = marks.into();
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        match (pieces.is_empty(), marks.front()) {
            (true, None) => return out,
            (false, None) => {
                for mut piece in pieces.drain(..) {
                    if !piece.ends_with(' ') {
                        piece.push(' ');
                    }
                    out.push(piece);
                }
            }
            (true, Some(_)) => {
                out.push(marks.drain(..).map(|m| m.text).collect());
            }
            (false, Some(run)) if run.line != pos => {
                out.push(pieces.pop_front().expect("not empty"));
                pos += 1;
            }
            (false, Some(_)) => {
                let run = marks.pop_front().expect("not empty");
                let single = pieces.len() == 1;
                let first = pieces.front_mut().expect("not empty");
                if first.ends_with(' ') {
                    first.pop();
                }
                let space = if run.text.ends_with(' ') { "" } else { " " };
                match run.place {
                    Place::Begin => first.insert_str(0, &run.text),
                    Place::End => {
                        let piece = pieces.pop_front().expect("not empty");
                        out.push(format!("{piece}{}{space}", run.text));
                        pos += 1;
                    }
                    Place::All => {
                        out.push(format!("{}{space}", run.text));
                        pos += 1;
                    }
                    Place::Inside if single => first.push_str(&run.text),
                    Place::Inside => {
                        let head = pieces.pop_front().expect("not empty");
                        let next = pieces.front_mut().expect("two pieces");
                        next.insert_str(0, &format!("{head}{}", run.text));
                    }
                }
            }
        }
    }
}

/// The phonemes of `text`, before Kokoro drops the unknown ones. Lines are joined with `\n`.
pub fn phonemize(text: &str, g2p: &dyn G2p) -> Result<String, String> {
    let mut pieces = Vec::new();
    let mut marks = Vec::new();
    let lines = text
        .trim()
        .trim_matches('\n')
        .split('\n')
        .filter(|l| !l.trim().is_empty());
    for (index, line) in lines.enumerate() {
        cut(line, index, &mut pieces, &mut marks);
    }
    let pieces: Vec<&str> = pieces
        .iter()
        .map(String::as_str)
        .filter(|p| !p.is_empty())
        .collect();
    let ipa = g2p.ipa_all(&pieces)?.iter().map(|i| words(i)).collect();
    Ok(join(ipa, marks).join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes each word in capitals, so the tests see where the marks go.
    struct Upper;

    impl G2p for Upper {
        fn ipa(&self, piece: &str) -> Result<String, String> {
            Ok(piece.to_uppercase())
        }
    }

    fn ph(text: &str) -> String {
        phonemize(text, &Upper).unwrap()
    }

    #[test]
    fn a_line_with_no_marks_is_one_piece() {
        assert_eq!(ph("hello world"), "HELLO WORLD ");
    }

    #[test]
    fn marks_go_back_between_the_pieces() {
        assert_eq!(
            ph("Hello world, this is a test. And another one"),
            "HELLO WORLD, THIS IS A TEST. AND ANOTHER ONE "
        );
        assert_eq!(ph("Wait... ok!"), "WAIT... OK! ");
        assert_eq!(ph("(aside) end"), "(ASIDE) END ");
    }

    #[test]
    fn a_decimal_point_is_not_a_mark_but_the_first_dot_cuts() {
        assert_eq!(runs("3.14"), vec![]);
        assert_eq!(runs("a 3, b"), vec![(3, 5)]);
        // phonemizer cuts at the first ".", inside "3.14".
        assert_eq!(ph("Pi is 3.14 and done."), "PI IS 3. \n14 AND DONE. ");
    }

    #[test]
    fn a_line_of_marks_only_stays() {
        assert_eq!(ph("..."), "...");
        assert_eq!(ph("a\n?!"), "A \n?!");
    }

    #[test]
    fn runs_hold_the_white_space_around_marks() {
        assert_eq!(runs("a , b"), vec![(1, 4)]);
        assert_eq!(runs("a  b"), vec![]);
        assert_eq!(runs("“x”"), vec![(0, 3), (4, 7)]);
    }

    #[test]
    fn an_empty_text_has_no_phonemes() {
        assert_eq!(ph("  \n "), "");
    }

    #[test]
    fn the_dummy_run_wins_only_when_it_has_no_primary_stress() {
        assert_eq!(choose("ðˈə\n".into(), "zˈiːbɹə ðə\n"), "ðə");
        assert_eq!(choose("ðˈɪs".into(), "zˈiːbɹə ðˈɪs"), "ðˈɪs");
        // The program joined the dummy with the piece, or wrote nothing for the piece.
        assert_eq!(choose("ˈeɪ".into(), "zˈiːbɹəɐ"), "ˈeɪ");
        assert_eq!(choose("x".into(), "zˈiːbɹə"), "x");
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let e = Espeak::with_program("/nonexistent/espeak-ng", "en-us")
            .ipa("x")
            .unwrap_err();
        assert!(e.contains("cannot run"), "{e}");
    }
}
