//! Cuts a stream of 16 kHz audio into pieces of speech.
//!
//! A piece starts at the first frame of speech, with a short lead before it. It ends after a
//! pause of the length that the caller sets, with a short tail of the pause. A piece never
//! passes [`MAX_PIECE`]: at that length the cutter cuts it at its quietest frame of the last
//! [`CUT_WINDOW`].

use super::{Classifier, Error, FRAME, THRESHOLD, frames_in};
use std::collections::VecDeque;
use std::time::Duration;

/// The longest piece.
pub const MAX_PIECE: Duration = Duration::from_secs(30);
/// A cut looks for the quietest frame in this span at the end of the piece.
pub const CUT_WINDOW: Duration = Duration::from_secs(10);
/// Frames before the first frame of speech that stay in the piece (384 ms). The detector marks
/// a frame a little after the speech starts, and Parakeet drops the first word of a clip that
/// has less than about 300 ms of silence before the speech.
const LEAD: usize = 12;
/// Frames of the pause that stay in the piece (160 ms).
const TAIL: usize = 5;
/// A piece with fewer frames of speech is a click or a breath, and the cutter drops it (96 ms).
const MIN_SPEECH: usize = 3;

struct Open {
    samples: Vec<f32>,
    speech: Vec<bool>,
    energy: Vec<f32>,
}

impl Open {
    fn frames(&self) -> usize {
        self.speech.len()
    }

    fn trailing_quiet(&self) -> usize {
        self.speech.iter().rev().take_while(|s| !**s).count()
    }

    fn push(&mut self, frame: &[f32], speech: bool) {
        self.samples.extend_from_slice(frame);
        self.speech.push(speech);
        self.energy.push(rms(frame));
    }

    /// The first `frames` frames as a piece, or `None` when they hold too little speech.
    fn piece(&self, frames: usize) -> Option<Vec<f32>> {
        let speech = self.speech[..frames].iter().filter(|s| **s).count();
        (speech >= MIN_SPEECH).then(|| self.samples[..frames * FRAME].to_vec())
    }

    /// The last `n` frames, one vector each.
    fn last_frames(&self, n: usize) -> VecDeque<Vec<f32>> {
        let from = self.samples.len() - n * FRAME;
        self.samples[from..]
            .chunks(FRAME)
            .map(<[f32]>::to_vec)
            .collect()
    }

    /// Drops the first `frames` frames.
    fn skip(&mut self, frames: usize) {
        self.samples.drain(..frames * FRAME);
        self.speech.drain(..frames);
        self.energy.drain(..frames);
    }
}

fn rms(frame: &[f32]) -> f32 {
    (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt()
}

pub struct Cutter<C> {
    classifier: C,
    /// Frames of pause that end a piece.
    pause: usize,
    /// Frames of speech that force a cut.
    longest: usize,
    /// Samples that do not fill a frame yet.
    pending: Vec<f32>,
    /// The last frames before the speech, which a new piece takes.
    lead: VecDeque<Vec<f32>>,
    open: Option<Open>,
    /// Samples since the last frame of speech, or since the start.
    quiet: usize,
}

impl<C: Classifier> Cutter<C> {
    /// A cutter that ends a piece after `pause` seconds with no speech.
    pub fn new(classifier: C, pause: f32) -> Cutter<C> {
        Cutter {
            classifier,
            pause: frames_in(pause).max(1),
            longest: frames_in_floor(MAX_PIECE),
            pending: Vec::new(),
            lead: VecDeque::new(),
            open: None,
            quiet: 0,
        }
    }

    /// Changes the pause that ends a piece. It acts from the next frame.
    pub fn set_pause(&mut self, seconds: f32) {
        self.pause = frames_in(seconds).max(1);
    }

    /// Changes the longest piece, which the cutter cuts at its quietest frame. The default is
    /// [`MAX_PIECE`]. A span under [`CUT_WINDOW`] is raised to it.
    pub fn set_longest(&mut self, span: Duration) {
        self.longest = frames_in_floor(span.max(CUT_WINDOW));
    }

    /// Takes audio and gives the pieces that it closes, in order.
    pub fn feed(&mut self, audio: &[f32]) -> Result<Vec<Vec<f32>>, Error> {
        self.pending.extend_from_slice(audio);
        let mut pieces = Vec::new();
        let mut at = 0;
        while self.pending.len() - at >= FRAME {
            let frame = self.pending[at..at + FRAME].to_vec();
            at += FRAME;
            if let Err(e) = self.frame(&frame, &mut pieces) {
                self.pending.drain(..at);
                return Err(e);
            }
        }
        self.pending.drain(..at);
        Ok(pieces)
    }

    /// Ends the stream: the open piece, if it holds speech. The cutter starts a new stream.
    pub fn finish(&mut self) -> Vec<Vec<f32>> {
        let mut pieces = Vec::new();
        if let Some(open) = self.open.take() {
            let frames = open.frames() - open.trailing_quiet().saturating_sub(TAIL);
            pieces.extend(open.piece(frames));
        }
        self.pending.clear();
        self.lead.clear();
        self.quiet = 0;
        self.classifier.reset();
        pieces
    }

    /// Drops the open piece and the stream, with no piece.
    pub fn reset(&mut self) {
        self.open = None;
        self.pending.clear();
        self.lead.clear();
        self.quiet = 0;
        self.classifier.reset();
    }

    /// Samples since the last frame of speech, or since the start.
    pub fn quiet_samples(&self) -> usize {
        self.quiet
    }

    fn frame(&mut self, frame: &[f32], pieces: &mut Vec<Vec<f32>>) -> Result<(), Error> {
        let speech = self.classifier.speech(frame)? > THRESHOLD;
        self.quiet = if speech { 0 } else { self.quiet + FRAME };
        let Some(open) = self.open.as_mut() else {
            if speech {
                let mut open = Open {
                    samples: Vec::new(),
                    speech: Vec::new(),
                    energy: Vec::new(),
                };
                for before in self.lead.drain(..) {
                    open.push(&before, false);
                }
                open.push(frame, true);
                self.open = Some(open);
            } else {
                self.lead.push_back(frame.to_vec());
                if self.lead.len() > LEAD {
                    self.lead.pop_front();
                }
            }
            return Ok(());
        };
        open.push(frame, speech);
        let quiet = open.trailing_quiet();
        if quiet >= self.pause {
            let frames = open.frames() - quiet.saturating_sub(TAIL);
            pieces.extend(open.piece(frames));
            // The pause goes on, and its last frames are the lead of the next piece.
            self.lead = open.last_frames(quiet.min(LEAD));
            self.open = None;
        } else if open.frames() >= self.longest {
            let from = open
                .frames()
                .saturating_sub(frames_in(CUT_WINDOW.as_secs_f32()));
            // The last of the quietest frames, so a flat stretch gives the longest piece.
            let cut = (from..open.frames())
                .rev()
                .min_by(|a, b| open.energy[*a].total_cmp(&open.energy[*b]))
                .unwrap_or(open.frames() - 1);
            pieces.extend(open.piece(cut + 1));
            open.skip(cut + 1);
            if open.frames() == 0 {
                self.open = None;
            }
        }
        Ok(())
    }
}

/// Whole frames in a span: a piece of that many frames is never longer than the span.
fn frames_in_floor(span: Duration) -> usize {
    (span.as_secs_f32() * super::RATE as f32 / FRAME as f32).floor() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech where the frame holds a sample above 0.1.
    struct Loud;

    impl Classifier for Loud {
        fn speech(&mut self, frame: &[f32]) -> Result<f32, Error> {
            Ok(if frame.iter().any(|s| s.abs() > 0.1) {
                0.9
            } else {
                0.1
            })
        }

        fn reset(&mut self) {}
    }

    const SECOND: usize = 16_000;

    fn speech(seconds: f32, level: f32) -> Vec<f32> {
        (0..(seconds * SECOND as f32) as usize)
            .map(|i| level * if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect()
    }

    fn quiet(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * SECOND as f32) as usize]
    }

    fn cutter() -> Cutter<Loud> {
        Cutter::new(Loud, 0.6)
    }

    fn all(cutter: &mut Cutter<Loud>, audio: &[f32]) -> Vec<Vec<f32>> {
        let mut pieces = cutter.feed(audio).unwrap();
        pieces.extend(cutter.finish());
        pieces
    }

    fn seconds(piece: &[f32]) -> f32 {
        piece.len() as f32 / SECOND as f32
    }

    #[test]
    fn a_pause_of_0_6_s_ends_a_piece_and_a_shorter_pause_does_not() {
        let audio = [
            speech(1.0, 0.5),
            quiet(0.4),
            speech(1.0, 0.5),
            quiet(0.7),
            speech(1.0, 0.5),
            quiet(1.0),
        ]
        .concat();
        let pieces = all(&mut cutter(), &audio);
        assert_eq!(pieces.len(), 2);
        // The first holds both seconds of speech, the short pause and a tail.
        assert!(
            (seconds(&pieces[0]) - 2.56).abs() < 0.05,
            "{}",
            seconds(&pieces[0])
        );
        // The second has a lead of 12 frames and a tail of 5, and the speech starts inside a frame.
        assert!(
            (seconds(&pieces[1]) - 1.6).abs() < 0.05,
            "{}",
            seconds(&pieces[1])
        );
    }

    #[test]
    fn a_turn_of_conversation_mode_needs_a_pause_of_1_2_s() {
        let audio = [
            speech(1.0, 0.5),
            quiet(1.0),
            speech(1.0, 0.5),
            quiet(1.4),
            speech(1.0, 0.5),
            quiet(2.0),
        ]
        .concat();
        let mut c = cutter();
        c.set_pause(1.2);
        let pieces = all(&mut c, &audio);
        // The pause of 1.0 s joins the first two speeches. The pause of 1.4 s ends the turn.
        assert_eq!(pieces.len(), 2);
        assert!(seconds(&pieces[0]) > 3.0, "{}", seconds(&pieces[0]));
    }

    #[test]
    fn a_longer_limit_keeps_a_long_turn_in_one_piece() {
        let audio = speech(60.0, 0.5);
        let mut c = cutter();
        c.set_longest(Duration::from_secs(120));
        let pieces = all(&mut c, &audio);
        assert_eq!(pieces.len(), 1);
        // A limit under the cut window is raised to it.
        let mut c = cutter();
        c.set_longest(Duration::from_secs(1));
        assert_eq!(all(&mut c, &speech(30.0, 0.5)).len(), 3);
    }

    #[test]
    fn a_piece_keeps_a_short_lead_and_tail_and_nothing_else() {
        let audio = [quiet(3.0), speech(0.5, 0.5), quiet(5.0)].concat();
        let pieces = all(&mut cutter(), &audio);
        assert_eq!(pieces.len(), 1);
        assert!(
            (seconds(&pieces[0]) - 1.09).abs() < 0.05,
            "{}",
            seconds(&pieces[0])
        );
    }

    #[test]
    fn the_open_piece_comes_out_at_the_end_of_the_stream() {
        let mut c = cutter();
        assert!(c.feed(&speech(2.0, 0.5)).unwrap().is_empty());
        let rest = c.finish();
        assert_eq!(rest.len(), 1);
        assert!((seconds(&rest[0]) - 2.0).abs() < 0.05);
        // A new stream starts clean.
        assert!(c.finish().is_empty());
    }

    #[test]
    fn a_click_is_not_a_piece() {
        let audio = [quiet(1.0), speech(0.03, 0.5), quiet(2.0)].concat();
        assert!(all(&mut cutter(), &audio).is_empty());
    }

    #[test]
    fn the_size_of_the_chunks_does_not_change_the_pieces() {
        let audio = [
            speech(1.0, 0.5),
            quiet(1.0),
            speech(2.5, 0.4),
            quiet(1.0),
            speech(0.7, 0.3),
        ]
        .concat();
        let whole = all(&mut cutter(), &audio);
        let mut c = cutter();
        let mut chunked = Vec::new();
        for chunk in audio.chunks(777) {
            chunked.extend(c.feed(chunk).unwrap());
        }
        chunked.extend(c.finish());
        assert_eq!(whole.len(), 3);
        assert_eq!(whole, chunked);
    }

    #[test]
    fn speech_with_no_pause_is_cut_at_its_quietest_frame_before_30_s() {
        // 40 s of speech. A dip in the level at 24 s is the quietest frame of the window.
        let mut audio = speech(40.0, 0.5);
        let dip = 24 * SECOND;
        for s in &mut audio[dip..dip + FRAME] {
            *s = s.signum() * 0.2;
        }
        let mut c = cutter();
        let mut pieces = c.feed(&audio).unwrap();
        pieces.extend(c.finish());
        assert_eq!(pieces.len(), 2);
        let first = seconds(&pieces[0]);
        assert!(first <= 30.0 && (first - 24.03).abs() < 0.05, "{first}");
        assert!(pieces.iter().all(|p| seconds(p) <= 30.0));
        // No sample is lost.
        let total: usize = pieces.iter().map(Vec::len).sum();
        assert_eq!(total, audio.len() / FRAME * FRAME);
    }

    #[test]
    fn three_minutes_of_speech_with_pauses_give_ordered_pieces_under_30_s() {
        // Phrases of 7 to 45 s with a pause of 0.8 s after each. The level names the phrase.
        let mut audio = Vec::new();
        let mut phrases = Vec::new();
        let mut total = 0.0;
        let mut n = 0u32;
        while total < 180.0 {
            let length = 7.0 + (n * 13 % 39) as f32;
            let level = 0.2 + 0.01 * (n % 50) as f32;
            audio.extend(speech(length, level));
            audio.extend(quiet(0.8));
            phrases.push(level);
            total += length + 0.8;
            n += 1;
        }
        let mut c = cutter();
        let mut pieces = Vec::new();
        for chunk in audio.chunks(2_400) {
            pieces.extend(c.feed(chunk).unwrap());
        }
        pieces.extend(c.finish());
        assert!(pieces.len() >= phrases.len());
        assert!(pieces.iter().all(|p| seconds(p) <= 30.0));
        // The levels of the pieces never go back to an earlier phrase.
        let level = |p: &Vec<f32>| p[p.len() / 2].abs();
        let mut seen = 0;
        for piece in &pieces {
            let at = phrases
                .iter()
                .position(|l| (l - level(piece)).abs() < 1e-4)
                .expect("a piece holds one phrase");
            assert!(at >= seen, "the pieces are out of order");
            seen = at;
        }
        assert_eq!(seen, phrases.len() - 1);
    }

    #[test]
    fn the_quiet_time_counts_from_the_last_speech() {
        let mut c = cutter();
        c.feed(&[speech(1.0, 0.5), quiet(2.0)].concat()).unwrap();
        // 3 s are 93 whole frames. The first 32 hold speech, as the 32nd holds its end.
        assert_eq!(c.quiet_samples(), 61 * FRAME);
    }
}
