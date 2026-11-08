//! Removes the queries that the screen of the core answered from the bytes
//! for the panes. Thus no query gets two answers.
//!
//! The filter feeds the screen in runs. A sequence that can be a query goes
//! to the screen alone. If the screen made a reply during it, the panes do
//! not get it. A query that the screen does not answer stays in the stream,
//! and the screen of a pane answers it.

/// The longest CSI sequence that the filter holds back at the end of a chunk.
const CSI_HOLD: usize = 256;
/// The longest OSC, DCS or APC string that the filter holds back.
const STRING_HOLD: usize = 4096;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const ENQ: u8 = 0x05;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    EscIntermediate,
    Csi,
    /// An OSC, DCS, APC, PM or SOS string. `bel_ends` is true for OSC.
    Str {
        bel_ends: bool,
    },
    /// ESC inside a string: `\` ends the string.
    StrEsc {
        bel_ends: bool,
    },
}

#[derive(Default)]
pub struct Filter {
    /// The start of a sequence that the last chunk did not finish.
    held: Vec<u8>,
    /// A sequence longer than the hold limits goes through the filter
    /// unchanged. Its state stays across chunks.
    long: Option<State>,
}

/// One sequence in the buffer.
struct Seq {
    start: usize,
    state: State,
    /// The first byte after the introducer, for a private CSI prefix.
    private: bool,
}

impl Filter {
    /// Feeds `input` to the screen through `feed`, and appends the bytes for
    /// the panes to `out`. `feed` returns true if the screen made a reply.
    pub fn push(&mut self, input: &[u8], feed: &mut dyn FnMut(&[u8]) -> bool, out: &mut Vec<u8>) {
        let held = std::mem::take(&mut self.held);
        let joined;
        let buf: &[u8] = if held.is_empty() {
            input
        } else {
            joined = [held.as_slice(), input].concat();
            &joined
        };
        let mut flushed = 0;
        let pass =
            |from: usize, to: usize, out: &mut Vec<u8>, feed: &mut dyn FnMut(&[u8]) -> bool| {
                if to > from {
                    feed(&buf[from..to]);
                    out.extend_from_slice(&buf[from..to]);
                }
            };

        // A long sequence from the last chunk: its bytes pass unchanged up to its end.
        let mut i = 0;
        if let Some(mut state) = self.long.take() {
            while i < buf.len() && state != State::Ground {
                let b = buf[i];
                match state {
                    // ESC ends the long sequence and starts a new one: the parser below takes it.
                    State::Csi | State::StrEsc { .. } if b == ESC => break,
                    // The ESC before this byte ends the string and starts a new sequence.
                    State::StrEsc { .. } if b != b'\\' && i > 0 => {
                        i -= 1;
                        break;
                    }
                    _ => {
                        state = step_long(state, b);
                        i += 1;
                    }
                }
            }
            if state != State::Ground && i == buf.len() {
                self.long = Some(state);
            }
        }

        let mut seq: Option<Seq> = None;
        while i < buf.len() {
            let b = buf[i];
            let Some(s) = &mut seq else {
                match b {
                    ESC => {
                        seq = Some(Seq {
                            start: i,
                            state: State::Esc,
                            private: false,
                        })
                    }
                    ENQ => {
                        pass(flushed, i, out, feed);
                        if !feed(&buf[i..=i]) {
                            out.push(ENQ);
                        }
                        flushed = i + 1;
                    }
                    _ => {}
                }
                i += 1;
                continue;
            };
            if b == ESC && matches!(s.state, State::Esc | State::EscIntermediate | State::Csi) {
                // ESC ends an unfinished sequence and starts a new one. The old bytes stay in the run.
                *s = Seq {
                    start: i,
                    state: State::Esc,
                    private: false,
                };
                i += 1;
                continue;
            }
            let done = match s.state {
                State::Esc => match b {
                    b'[' => {
                        s.state = State::Csi;
                        None
                    }
                    b']' => {
                        s.state = State::Str { bel_ends: true };
                        None
                    }
                    b'P' | b'_' | b'^' | b'X' => {
                        s.state = State::Str { bel_ends: false };
                        None
                    }
                    0x20..=0x2f => {
                        s.state = State::EscIntermediate;
                        None
                    }
                    // DECID asks for the device attributes.
                    b'Z' => Some(true),
                    _ => Some(false),
                },
                State::EscIntermediate => match b {
                    0x20..=0x2f => None,
                    _ => Some(false),
                },
                State::Csi => match b {
                    b'?' | b'>' | b'=' | b'<' if i == s.start + 2 => {
                        s.private = true;
                        None
                    }
                    0x20..=0x3f => None,
                    0x40..=0x7e => Some(s.private || b"cnpqtuxS".contains(&b)),
                    CAN | SUB => Some(false),
                    // Other C0 bytes run inside the sequence.
                    0x00..=0x1f => None,
                    _ => Some(false),
                },
                State::Str { bel_ends } => match b {
                    BEL if bel_ends => Some(true),
                    ESC => {
                        s.state = State::StrEsc { bel_ends };
                        None
                    }
                    CAN | SUB => Some(false),
                    _ => None,
                },
                State::StrEsc { .. } => match b {
                    b'\\' => Some(true),
                    // Another ESC sequence ends the string and starts at this ESC.
                    _ => {
                        let end = i - 1;
                        pass(flushed, s.start, out, feed);
                        let answered = feed(&buf[s.start..end]);
                        if !answered {
                            out.extend_from_slice(&buf[s.start..end]);
                        }
                        flushed = end;
                        seq = Some(Seq {
                            start: end,
                            state: State::Esc,
                            private: false,
                        });
                        continue;
                    }
                },
                State::Ground => unreachable!("a sequence is never at ground"),
            };
            i += 1;
            let Some(candidate) = done else { continue };
            let s = seq.take().expect("a sequence");
            if candidate {
                pass(flushed, s.start, out, feed);
                if !feed(&buf[s.start..i]) {
                    out.extend_from_slice(&buf[s.start..i]);
                }
                flushed = i;
            }
        }

        // A sequence that the chunk did not finish.
        let mut end = buf.len();
        if let Some(s) = seq {
            let limit = match s.state {
                State::Str { .. } | State::StrEsc { .. } => STRING_HOLD,
                _ => CSI_HOLD,
            };
            if buf.len() - s.start <= limit {
                end = s.start;
                self.held = buf[s.start..].to_vec();
            } else {
                self.long = Some(s.state);
            }
        }
        pass(flushed, end, out, feed);
    }

    /// Gives the held bytes to the screen and to the panes, at the end of the output.
    pub fn finish(&mut self, feed: &mut dyn FnMut(&[u8]) -> bool, out: &mut Vec<u8>) {
        let held = std::mem::take(&mut self.held);
        if !held.is_empty() {
            feed(&held);
            out.extend_from_slice(&held);
        }
    }
}

/// The state after one byte of a sequence that passes unchanged. It ends at ground.
fn step_long(state: State, b: u8) -> State {
    match state {
        State::Str { bel_ends } => match b {
            BEL if bel_ends => State::Ground,
            ESC => State::StrEsc { bel_ends },
            CAN | SUB => State::Ground,
            _ => state,
        },
        State::StrEsc { bel_ends } => match b {
            b'\\' => State::Ground,
            ESC => State::StrEsc { bel_ends },
            // Another sequence starts: it goes through the filter like the rest.
            _ => State::Ground,
        },
        State::Csi => match b {
            0x40..=0x7e | CAN | SUB => State::Ground,
            0x80..=0xff => State::Ground,
            _ => state,
        },
        _ => State::Ground,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake screen that answers the sequences in `answers`.
    fn run(chunks: &[&[u8]], answers: &[&[u8]]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut f = Filter::default();
        let mut fed: Vec<Vec<u8>> = Vec::new();
        let mut out = Vec::new();
        let mut feed = |b: &[u8]| {
            fed.push(b.to_vec());
            answers.contains(&b)
        };
        for c in chunks {
            f.push(c, &mut feed, &mut out);
        }
        f.finish(&mut feed, &mut out);
        (out, fed)
    }

    #[test]
    fn an_answered_query_goes_to_the_screen_alone_and_not_to_the_panes() {
        let (out, fed) = run(
            &[b"ab\x1b[cde\x1b]7501;?\x1b\\f"],
            &[b"\x1b[c", b"\x1b]7501;?\x1b\\"],
        );
        assert_eq!(out, b"abdef");
        assert_eq!(
            fed,
            [&b"ab"[..], b"\x1b[c", b"de", b"\x1b]7501;?\x1b\\", b"f"]
        );
    }

    #[test]
    fn a_query_without_an_answer_stays_in_the_stream() {
        let (out, _) = run(&[b"x\x1b[>qy\x05z"], &[]);
        assert_eq!(out, b"x\x1b[>qy\x05z");
    }

    #[test]
    fn styles_and_moves_pass_in_one_run() {
        let text = b"\x1b[1;31mred\x1b[0m\x1b[2;3H\x1b[K\x1b(B\x1b7";
        let (out, fed) = run(&[text], &[]);
        assert_eq!(out, text);
        assert_eq!(fed, [text.to_vec()]);
    }

    #[test]
    fn a_query_split_over_chunks_is_held_and_still_removed() {
        let stream = b"one\x1b]7501;?\x1b\\two\x1b[6ntri\x1b[?u";
        let answers: [&[u8]; 3] = [b"\x1b]7501;?\x1b\\", b"\x1b[6n", b"\x1b[?u"];
        for split in 0..stream.len() {
            let (out, fed) = run(&[&stream[..split], &stream[split..]], &answers);
            assert_eq!(out, b"onetwotri", "split at {split}");
            assert_eq!(fed.concat(), stream, "split at {split}");
        }
        // One byte at a time.
        let bytes: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(run(&bytes, &answers).0, b"onetwotri");
    }

    #[test]
    fn a_long_string_passes_unchanged_and_the_filter_goes_on_after_it() {
        let mut big = b"\x1b]52;c;".to_vec();
        big.extend(std::iter::repeat_n(b'A', STRING_HOLD + 10));
        big.extend(b"\x07");
        let (a, b) = big.split_at(STRING_HOLD + 5);
        let (out, _) = run(&[a, b, b"x\x1b[cy"], &[b"\x1b[c"]);
        let mut expected = big.clone();
        expected.extend(b"xy");
        assert_eq!(out, expected);
    }

    #[test]
    fn an_esc_inside_a_string_ends_it_and_starts_a_new_sequence() {
        let (out, fed) = run(&[b"\x1b]0;title\x1b[c!"], &[b"\x1b[c"]);
        assert_eq!(out, b"\x1b]0;title!");
        assert!(fed.contains(&b"\x1b]0;title".to_vec()));
    }

    #[test]
    fn an_esc_inside_an_unfinished_csi_starts_a_new_sequence() {
        let (out, fed) = run(&[b"a\x1b[1\x1b\x1b[cb"], &[b"\x1b[c"]);
        assert_eq!(out, b"a\x1b[1\x1bb");
        assert!(fed.contains(&b"\x1b[c".to_vec()), "{fed:?}");
    }

    /// Review B4: ESC ends a long unfinished CSI, and the query after it is still removed.
    #[test]
    fn a_query_after_a_long_unfinished_csi_is_removed() {
        let mut long = b"\x1b[".to_vec();
        long.extend(std::iter::repeat_n(b'1', CSI_HOLD + 10));
        let (out, _) = run(&[&long, b"\x1b[cx"], &[b"\x1b[c"]);
        let mut expected = long.clone();
        expected.push(b'x');
        assert_eq!(out, expected);
    }

    #[test]
    fn the_end_of_the_output_flushes_a_held_sequence() {
        let (out, fed) = run(&[b"a\x1b]0;unfinished"], &[]);
        assert_eq!(out, b"a\x1b]0;unfinished");
        assert_eq!(fed.concat(), b"a\x1b]0;unfinished");
    }
}
