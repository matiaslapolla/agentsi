//! A blocking client in framed mode, for the terminal tests.

use agentsi_proto::frame::{Frame, Kind, read_frame};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Framed {
    pub stream: UnixStream,
    next_id: i64,
    /// Frames that came while the client waited for a reply.
    pending: VecDeque<Frame>,
}

impl Framed {
    pub fn connect(socket: &Path) -> Framed {
        let mut stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        writeln!(
            stream,
            r#"{{"jsonrpc":"2.0","id":0,"method":"core.hello","params":{{"framed":true,"client":"test"}}}}"#
        )
        .unwrap();
        // The reply is one line. Read it byte by byte: frames follow it.
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        while b[0] != b'\n' {
            stream.read_exact(&mut b).unwrap();
            line.push(b[0]);
        }
        let hello: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(hello["result"]["protocol"], 1, "{hello}");
        Framed {
            stream,
            next_id: 1,
            pending: VecDeque::new(),
        }
    }

    pub fn send(&mut self, frame: &Frame) {
        self.stream.write_all(&frame.encode()).unwrap();
    }

    /// Calls a method. Returns the whole response: `result` or `error`.
    pub fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&Frame::message(&request.to_string()));
        loop {
            let f = read_frame(&mut self.stream).unwrap().expect("a frame");
            if f.channel == 0 && f.kind == Kind::DATA {
                let v: Value = serde_json::from_slice(&f.payload).unwrap();
                if v["id"] == id {
                    return v;
                }
            }
            self.pending.push_back(f);
        }
    }

    /// Calls a method and returns its result. It fails on an error.
    pub fn ok(&mut self, method: &str, params: Value) -> Value {
        let v = self.call(method, params);
        assert!(v.get("error").is_none(), "{method}: {v}");
        v["result"].clone()
    }

    pub fn next_frame(&mut self) -> Frame {
        if let Some(f) = self.pending.pop_front() {
            return f;
        }
        read_frame(&mut self.stream).unwrap().expect("a frame")
    }

    /// The next notification on channel 0 with this method.
    pub fn next_event(&mut self, method: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            assert!(Instant::now() < deadline, "no {method}");
            let f = self.next_frame();
            if f.channel == 0 && f.kind == Kind::DATA {
                let v: Value = serde_json::from_slice(&f.payload).unwrap();
                if v["method"] == method {
                    return v["params"].clone();
                }
            }
        }
    }
}

/// What a pane reads from its channel: the frames, and a screen fed with the bytes.
pub struct Pane {
    pub channel: u32,
    pub screen: agentsi_vt::Terminal,
    pub resizes: Vec<(u16, u16)>,
    pub bytes: Vec<u8>,
    pub close: Option<Value>,
    /// Return a CREDIT for each DATA frame.
    pub credit: bool,
}

impl Pane {
    pub fn new(channel: u32) -> Pane {
        Pane {
            channel,
            screen: agentsi_vt::Terminal::new(agentsi_vt::Options::default()).unwrap(),
            resizes: Vec::new(),
            bytes: Vec::new(),
            close: None,
            credit: true,
        }
    }

    /// Takes one frame of the channel. Returns false for a frame of another channel.
    pub fn take(&mut self, client: &mut Framed, f: &Frame) -> bool {
        if f.channel != self.channel {
            return false;
        }
        match f.kind {
            Kind::DATA => {
                self.screen.write(&f.payload);
                self.screen.take_events();
                self.bytes.extend_from_slice(&f.payload);
                if self.credit {
                    client.send(&Frame::credit(self.channel, f.payload.len() as u32));
                }
            }
            Kind::RESIZE => {
                let (cols, rows) = f.as_resize().unwrap();
                self.screen.resize(cols, rows, 0, 0).unwrap();
                self.resizes.push((cols, rows));
            }
            Kind::CLOSE => self.close = Some(serde_json::from_slice(&f.payload).unwrap()),
            _ => {}
        }
        true
    }

    /// Reads frames until `done` holds.
    pub fn read_until(
        &mut self,
        client: &mut Framed,
        what: &str,
        mut done: impl FnMut(&mut Pane) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !done(self) {
            if Instant::now() >= deadline {
                self.fail(what, "timed out");
            }
            let f = match client.pending.pop_front() {
                Some(f) => f,
                None => match read_frame(&mut client.stream) {
                    Ok(Some(f)) => f,
                    other => self.fail(what, &format!("{other:?}")),
                },
            };
            self.take(client, &f);
        }
    }

    fn fail(&mut self, what: &str, why: &str) -> ! {
        let screen = self.text();
        panic!(
            "{what}: {why}. Got {} bytes, resizes {:?}, close {:?}. Screen: {screen:?}",
            self.bytes.len(),
            self.resizes,
            self.close,
        )
    }

    pub fn text(&mut self) -> String {
        self.screen.grid().unwrap().text_lines().join("\n")
    }
}

/// The resident memory of a process in KiB.
pub fn rss_kib(pid: i32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap()
}
