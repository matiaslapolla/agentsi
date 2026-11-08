//! The stream of one attached pane: its queue, its credit window and its pump.
//!
//! The reader thread of the terminal puts the live bytes in the queue and
//! never waits. The pump task sends them as frames while the window of the
//! pane is open. If a pane does not read, its queue stops at `PENDING_CAP`:
//! the core drops the queued bytes and sends a new first paint when the pane
//! reads again. Thus a pane costs the core at most the first paint, the cap
//! and the window.

use crate::state::lock;
use agentsi_proto::frame::{Frame, Kind, STREAM_WINDOW};
use std::collections::VecDeque;
use std::sync::Mutex;
use tokio::sync::Notify;

/// The live bytes that wait for one pane.
const PENDING_CAP: usize = 1 << 20;
/// The largest DATA frame of a stream.
pub(crate) const FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Resize(u16, u16),
    Data(Vec<u8>),
    /// The last item. The payload is a JSON reason.
    Close(String),
}

#[derive(Debug)]
struct Queue {
    items: VecDeque<Item>,
    /// The bytes of `Data` items, without a first paint.
    live: usize,
    /// The bytes that the core may still send before the next CREDIT.
    window: usize,
    /// The core dropped bytes. The pane needs a new first paint.
    resync: bool,
    /// The Close item is queued: later pushes do nothing.
    closed: bool,
    /// The pane went away: the stream ends without a CLOSE frame.
    detached: bool,
}

/// One pane on one terminal.
#[derive(Debug)]
pub struct Attach {
    /// Unique in the core.
    pub id: u64,
    pub channel: u32,
    pub observe: bool,
    queue: Mutex<Queue>,
    ready: Notify,
}

/// What the pump does next.
pub enum Next {
    /// Send these frames, then ask again. The last frame can be a CLOSE.
    Send(Vec<Frame>),
    /// The pane needs a new first paint. Call `resync` with the screen locked.
    Resync,
    /// Nothing to send: wait for `ready`.
    Wait,
    /// The stream ended.
    End,
}

impl Attach {
    pub fn new(id: u64, channel: u32, observe: bool) -> Attach {
        Attach {
            id,
            channel,
            observe,
            queue: Mutex::new(Queue {
                items: VecDeque::new(),
                live: 0,
                window: STREAM_WINDOW,
                resync: false,
                closed: false,
                detached: false,
            }),
            ready: Notify::new(),
        }
    }

    /// Queues a first paint: the size, then the bytes. It replaces the bytes in
    /// the queue, and comes before a queued Close.
    pub fn paint(&self, cols: u16, rows: u16, bytes: Vec<u8>) {
        let mut q = lock(&self.queue);
        if q.detached {
            return;
        }
        q.items.retain(|i| matches!(i, Item::Close(_)));
        q.items.push_front(Item::Data(bytes));
        q.items.push_front(Item::Resize(cols, rows));
        q.live = 0;
        q.resync = false;
        drop(q);
        self.ready.notify_one();
    }

    /// Live bytes of the program. Past the cap, the queue drops its bytes.
    pub fn push_bytes(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut q = lock(&self.queue);
        if q.closed || q.resync {
            return;
        }
        if q.live + bytes.len() > PENDING_CAP {
            q.items.retain(|i| !matches!(i, Item::Data(_)));
            q.live = 0;
            q.resync = true;
        } else {
            q.live += bytes.len();
            match q.items.back_mut() {
                Some(Item::Data(last)) if last.len() < FRAME_BYTES => last.extend_from_slice(bytes),
                _ => q.items.push_back(Item::Data(bytes.to_vec())),
            }
        }
        drop(q);
        self.ready.notify_one();
    }

    pub fn push_resize(&self, cols: u16, rows: u16) {
        let mut q = lock(&self.queue);
        if q.closed || q.resync {
            return;
        }
        q.items.push_back(Item::Resize(cols, rows));
        drop(q);
        self.ready.notify_one();
    }

    /// The last item of the stream. Later pushes do nothing.
    pub fn push_close(&self, reason: String) {
        let mut q = lock(&self.queue);
        if q.closed {
            return;
        }
        q.closed = true;
        // A pane that must resync gets the last screen first, then the Close.
        q.items.push_back(Item::Close(reason));
        drop(q);
        self.ready.notify_one();
    }

    /// The pane went away. The stream ends without a CLOSE frame.
    pub fn detach(&self) {
        let mut q = lock(&self.queue);
        q.closed = true;
        q.detached = true;
        q.items.clear();
        q.live = 0;
        drop(q);
        self.ready.notify_one();
    }

    /// The pane read `bytes` bytes.
    pub fn credit(&self, bytes: u32) {
        let mut q = lock(&self.queue);
        q.window = q.window.saturating_add(bytes as usize).min(STREAM_WINDOW);
        drop(q);
        self.ready.notify_one();
    }

    pub fn needs_resync(&self) -> bool {
        lock(&self.queue).resync
    }

    pub async fn ready(&self) {
        self.ready.notified().await
    }

    /// The next frames to send within the window.
    pub fn next(&self) -> Next {
        let mut guard = lock(&self.queue);
        let q = &mut *guard;
        if q.detached {
            return Next::End;
        }
        if q.resync {
            return if q.window > 0 {
                Next::Resync
            } else {
                Next::Wait
            };
        }
        let mut frames = Vec::new();
        while let Some(item) = q.items.front_mut() {
            match item {
                Item::Resize(cols, rows) => {
                    frames.push(Frame::resize(self.channel, *cols, *rows));
                    q.items.pop_front();
                }
                Item::Close(reason) => {
                    frames.push(Frame::close(self.channel, reason));
                    q.items.clear();
                    q.live = 0;
                    return Next::Send(frames);
                }
                Item::Data(bytes) => {
                    let n = bytes.len().min(q.window).min(FRAME_BYTES);
                    if n == 0 {
                        break;
                    }
                    let rest = bytes.split_off(n);
                    let chunk = std::mem::replace(bytes, rest);
                    if bytes.is_empty() {
                        q.items.pop_front();
                    }
                    q.window -= n;
                    q.live = q.live.saturating_sub(n);
                    frames.push(Frame::new(self.channel, Kind::DATA, chunk));
                    if frames.len() >= 16 {
                        break;
                    }
                }
            }
        }
        match (frames.is_empty(), q.closed && q.items.is_empty()) {
            (false, _) => Next::Send(frames),
            (true, true) => Next::End,
            (true, false) => Next::Wait,
        }
    }

    /// The bytes that wait in the queue.
    #[cfg(test)]
    fn queued(&self) -> usize {
        lock(&self.queue)
            .items
            .iter()
            .map(|i| match i {
                Item::Data(b) => b.len(),
                _ => 0,
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(a: &Attach) -> Vec<Frame> {
        let mut all = Vec::new();
        while let Next::Send(frames) = a.next() {
            all.extend(frames);
        }
        all
    }

    fn data(frames: &[Frame]) -> usize {
        frames
            .iter()
            .filter(|f| f.kind == Kind::DATA)
            .map(|f| f.payload.len())
            .sum()
    }

    #[test]
    fn the_core_sends_at_most_the_window_before_a_credit() {
        let a = Attach::new(1, 3, false);
        a.paint(80, 24, b"paint".to_vec());
        for _ in 0..10 {
            a.push_bytes(&[b'x'; 100_000]);
        }
        let mut frames = sent(&a);
        assert_eq!(frames[0].as_resize(), Some((80, 24)));
        a.push_bytes(&[b'x'; 100_000]);
        frames.extend(sent(&a));
        assert_eq!(data(&frames), STREAM_WINDOW);
        assert!(!a.needs_resync());
        assert!(matches!(a.next(), Next::Wait));
        a.credit(1000);
        assert_eq!(data(&sent(&a)), 1000);
    }

    #[test]
    fn a_pane_that_does_not_read_costs_at_most_the_cap() {
        let a = Attach::new(1, 3, false);
        a.paint(80, 24, Vec::new());
        sent(&a);
        a.push_bytes(&vec![b'x'; STREAM_WINDOW]);
        sent(&a);
        for _ in 0..1000 {
            a.push_bytes(&[b'y'; 100_000]);
            assert!(a.queued() <= PENDING_CAP);
        }
        assert!(a.needs_resync());
        assert!(matches!(a.next(), Next::Wait));
        a.credit(10);
        assert!(matches!(a.next(), Next::Resync));
        a.paint(100, 30, b"fresh".to_vec());
        a.push_bytes(b"after");
        let frames = sent(&a);
        assert_eq!(frames[0].as_resize(), Some((100, 30)));
        assert_eq!(frames[1].payload, b"freshafter");
    }

    #[test]
    fn close_comes_after_the_queued_bytes_and_ends_the_stream() {
        let a = Attach::new(1, 3, false);
        a.push_bytes(b"last words");
        a.push_close(r#"{"reason":"exit","code":0}"#.into());
        a.push_bytes(b"ignored");
        let frames = sent(&a);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].payload, b"last words");
        assert_eq!(frames[1].kind, Kind::CLOSE);
        assert!(matches!(a.next(), Next::End));
    }

    #[test]
    fn a_pane_that_must_resync_gets_the_last_screen_before_the_close() {
        let a = Attach::new(1, 3, false);
        a.push_bytes(&vec![b'x'; STREAM_WINDOW]);
        sent(&a);
        a.push_bytes(&vec![b'y'; PENDING_CAP + 1]);
        a.push_close("{}".into());
        assert!(matches!(a.next(), Next::Wait));
        a.credit(100);
        assert!(matches!(a.next(), Next::Resync));
        a.paint(80, 24, b"last".to_vec());
        let frames = sent(&a);
        let kinds: Vec<Kind> = frames.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, [Kind::RESIZE, Kind::DATA, Kind::CLOSE]);
        assert_eq!(frames[1].payload, b"last");
        assert!(matches!(a.next(), Next::End));
    }

    #[test]
    fn a_detached_pane_ends_at_once() {
        let a = Attach::new(1, 3, false);
        a.push_bytes(b"never sent");
        a.detach();
        assert!(matches!(a.next(), Next::End));
    }
}
