//! The state of one connection. A handler asks for a change, and the
//! connection loop makes it after the reply.

use crate::state::{Core, Subscriber, lock};
use crate::terminal::Term;
use crate::terminal::stream::Attach;
use agentsi_proto::core_api::Topic;
use agentsi_proto::frame::{Frame, Kind};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task::AbortHandle;

/// One stream channel of a connection.
struct Stream {
    attach: Arc<Attach>,
    term: Arc<Term>,
    pump: Option<AbortHandle>,
}

#[derive(Default)]
pub struct Session {
    subscriber: Mutex<Option<Arc<Subscriber>>>,
    /// A first `core.subscribe` made the subscriber. The loop must start to send its events.
    start_events: AtomicBool,
    forwarder: Mutex<Option<AbortHandle>>,
    framed: AtomicBool,
    /// The connection uses frames now.
    in_frames: AtomicBool,
    shutdown: AtomicBool,
    /// The requests that the connection has begun and not yet answered.
    in_flight: AtomicUsize,
    next_channel: AtomicU32,
    streams: Mutex<HashMap<u32, Stream>>,
}

/// Counts one request of a connection while it lives.
pub struct InFlight<'a>(&'a AtomicUsize);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Session {
    pub fn begin_request(&self) -> InFlight<'_> {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        InFlight(&self.in_flight)
    }

    pub fn requests_in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// Adds topics and gives all the topics of the connection. The events
    /// wait in the queue of the connection from this call on.
    pub fn add_topics(&self, core: &Core, topics: &[Topic]) -> Vec<Topic> {
        let mut subscriber = lock(&self.subscriber);
        if subscriber.is_none() && !topics.is_empty() {
            *subscriber = Some(core.subscriber());
            self.start_events.store(true, Ordering::SeqCst);
        }
        subscriber
            .as_ref()
            .map(|s| s.add_topics(topics))
            .unwrap_or_default()
    }

    pub fn take_new_events(&self) -> Option<Arc<Subscriber>> {
        if !self.start_events.swap(false, Ordering::SeqCst) {
            return None;
        }
        lock(&self.subscriber).clone()
    }

    pub fn set_forwarder(&self, handle: AbortHandle) {
        *lock(&self.forwarder) = Some(handle);
    }

    pub fn stop_forwarder(&self) {
        if let Some(f) = lock(&self.forwarder).take() {
            f.abort();
        }
    }

    pub fn ask_framed(&self) {
        self.framed.store(true, Ordering::SeqCst);
    }

    pub fn take_framed(&self) -> bool {
        self.framed.swap(false, Ordering::SeqCst)
    }

    pub fn set_in_frames(&self) {
        self.in_frames.store(true, Ordering::SeqCst);
    }

    pub fn in_frames(&self) -> bool {
        self.in_frames.load(Ordering::SeqCst)
    }

    pub fn next_channel(&self) -> u32 {
        self.next_channel.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Adds a stream. Its pump starts after the reply of its request.
    pub fn add_stream(&self, attach: Arc<Attach>, term: Arc<Term>) {
        let channel = attach.channel;
        lock(&self.streams).insert(
            channel,
            Stream {
                attach,
                term,
                pump: None,
            },
        );
    }

    pub fn stream(&self, channel: u32) -> Option<(Arc<Attach>, Arc<Term>)> {
        lock(&self.streams)
            .get(&channel)
            .map(|s| (s.attach.clone(), s.term.clone()))
    }

    pub fn set_pump(&self, channel: u32, pump: AbortHandle) {
        match lock(&self.streams).get_mut(&channel) {
            Some(s) => s.pump = Some(pump),
            None => pump.abort(),
        }
    }

    /// Ends a stream: the pump stops and the terminal forgets the pane.
    pub fn close_stream(&self, channel: u32) {
        let stream = lock(&self.streams).remove(&channel);
        if let Some(s) = stream {
            if let Some(p) = s.pump {
                p.abort();
            }
            s.term.detach(&s.attach);
        }
    }

    /// Ends each stream, when the connection ends.
    pub fn stop_streams(&self) {
        let channels: Vec<u32> = lock(&self.streams).keys().copied().collect();
        for c in channels {
            self.close_stream(c);
        }
    }

    /// A frame from the client on a stream channel. It never waits: past the
    /// input budget of the terminal, the core drops the input.
    pub fn stream_frame(&self, frame: Frame) {
        let Some((attach, term)) = self.stream(frame.channel) else {
            return;
        };
        match frame.kind {
            Kind::CREDIT => {
                if let Some(n) = frame.as_credit() {
                    attach.credit(n);
                }
            }
            Kind::CLOSE => self.close_stream(frame.channel),
            _ if attach.observe => {}
            Kind::DATA => {
                if let Err(e) = term.input(&frame.payload) {
                    eprintln!("core: input on channel {} dropped: {e}", frame.channel);
                }
            }
            Kind::AUTO => {
                if let Err(e) = term.auto(attach.id, &frame.payload) {
                    eprintln!("core: AUTO on channel {} dropped: {e}", frame.channel);
                }
            }
            Kind::RESIZE => {
                if let Some((cols, rows)) = frame.as_resize()
                    && let Err(e) = term.resize(cols, rows, frame.as_cell_size())
                {
                    eprintln!("core: RESIZE on channel {}: {e}", frame.channel);
                }
            }
            Kind::FOCUS => {
                if let Some(focused) = frame.as_focus() {
                    term.focus(attach.id, focused);
                }
            }
            _ => {}
        }
    }

    pub fn ask_shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    pub fn take_shutdown(&self) -> bool {
        self.shutdown.swap(false, Ordering::SeqCst)
    }
}
