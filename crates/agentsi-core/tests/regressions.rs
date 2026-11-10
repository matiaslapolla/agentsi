//! Regression tests for the bugs of the U3 code review (B1 to B7), on the
//! real `agentsi-core` program.

mod common;

use agentsi_proto::frame::{self, Frame, MAX_PAYLOAD};
use common::{BIN, CoreProc, TempDir, wait_until};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const FRAMED_HELLO: &str =
    r#"{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{"framed":true}}"#;

/// A connection in framed mode, after the reply to `core.hello`.
fn framed(core: &CoreProc) -> (UnixStream, BufReader<UnixStream>) {
    let mut stream = UnixStream::connect(&core.socket.path).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    writeln!(stream, "{FRAMED_HELLO}").unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains(r#""protocol":1"#), "{line}");
    (stream, reader)
}

fn rss_kib(pid: i32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

/// A JSON-RPC request of exactly `len` bytes, without the newline.
fn padded(method: &str, id: u32, len: usize) -> String {
    let head = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","pad":""#);
    let tail = r#""}"#;
    format!("{head}{}{tail}", "x".repeat(len - head.len() - tail.len()))
}

/// B1: at the end of input, the core finishes the requests in flight, and
/// then closes the connection.
#[test]
fn a_framed_connection_closes_after_its_requests_when_the_input_ends() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let (mut stream, mut reader) = framed(&core);
    for id in 2..402 {
        let request = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"rpc.discover"}}"#);
        stream
            .write_all(&Frame::message(&request).encode())
            .unwrap();
    }
    let subscribe =
        r#"{"jsonrpc":"2.0","id":402,"method":"core.subscribe","params":{"topics":["terminal"]}}"#;
    stream
        .write_all(&Frame::message(subscribe).encode())
        .unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut replies = 0;
    while let Some(f) = frame::read_frame(&mut reader).expect("the core closes the connection") {
        assert_eq!(f.channel, 0);
        replies += 1;
    }
    assert_eq!(replies, 401);
}

/// B3: `core.shutdown` stops the core even if the client closes before the reply.
#[test]
fn shutdown_stops_the_core_when_the_client_closes_at_once() {
    let dir = TempDir::new();
    let mut core = CoreProc::start(&dir);
    let mut stream = UnixStream::connect(&core.socket.path).unwrap();
    writeln!(stream, "{}", common::SHUTDOWN).unwrap();
    drop(stream);
    let child = core.child.as_mut().unwrap();
    wait_until("the core stops", || child.try_wait().unwrap().is_some());
}

/// B4: a client that sends large frames and never reads cannot fill the
/// memory of the core.
#[test]
fn a_framed_client_that_never_reads_cannot_fill_the_memory_of_the_core() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let pid = core.pid();
    let before = rss_kib(pid);
    let (mut stream, _reader) = framed(&core);
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut sent = 0;
    for id in 2..602 {
        if stream
            .write_all(&Frame::message(&padded("rpc.discover", id, 1 << 20)).encode())
            .is_err()
        {
            break;
        }
        sent += 1;
    }
    let after = rss_kib(pid);
    // By design the core keeps up to 256 replies for a client that does not read (`OUTBOX`
    // in `server.rs`), and one more in its writer. Each reply of `rpc.discover` is the
    // catalog, so the limit grows with the catalog. The pretty schema file is larger than a
    // reply. A core that kept the request frames would hold more than 291 MiB.
    let catalog = include_str!("../../../schema/core.openrpc.json").len() as u64;
    let limit = 257 * catalog / 1024 + 32 * 1024;
    assert!(
        after < before + limit,
        "RSS {before} KiB became {after} KiB after {sent} frames of 1 MiB (limit +{limit} KiB)"
    );
}

/// B5: a core that stops frees its lock after it removes the socket. A
/// client that starts a core in that moment starts one again.
#[test]
fn a_client_starts_the_core_again_when_the_lock_frees_late() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let _guard = CoreProc::adopt(socket.clone());
    agentsi_core::server::prepare_folder(&socket).unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(socket.lock())
        .unwrap();
    lock.lock().unwrap();
    let client = {
        let socket = socket.clone();
        std::thread::spawn(move || {
            agentsi_proto::client::connect_or_start(&socket, std::path::Path::new(BIN))
        })
    };
    std::thread::sleep(Duration::from_millis(500));
    drop(lock);
    let started = Instant::now();
    let stream = client
        .join()
        .unwrap()
        .expect("the client starts a core after the lock frees");
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(stream);
}

/// B3 of the U27 review: a core that finds the lock held stops with code 3 for as long as
/// the core that stops holds it (up to 10 s). The client waits that long, and does not start
/// a core in a tight loop.
#[test]
fn a_client_waits_for_a_long_stop_and_does_not_start_cores_in_a_loop() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let _guard = CoreProc::adopt(socket.clone());
    let calls = dir.0.join("calls");
    let released = dir.0.join("released");
    let fake = dir.0.join("core");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho x >> {calls}\n[ -e {released} ] && exec {BIN} \"$@\"\nexit 3\n",
            calls = calls.display(),
            released = released.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let client = {
        let (socket, fake) = (socket.clone(), fake.clone());
        std::thread::spawn(move || agentsi_proto::client::connect_or_start(&socket, &fake))
    };
    // Longer than the start timeout of 5 s.
    std::thread::sleep(Duration::from_secs(6));
    assert!(
        !client.is_finished(),
        "the client gave up while the core stops"
    );
    let spawned = std::fs::read_to_string(&calls).unwrap().lines().count();
    assert!(spawned <= 20, "{spawned} cores started in 6 s");
    std::fs::write(&released, "").unwrap();
    let stream = client
        .join()
        .unwrap()
        .expect("the client connects when the lock frees");
    drop(stream);
}

/// B6: a stream that ends inside a frame header is an error, not a clean end.
#[test]
fn a_stream_that_ends_inside_a_header_is_an_error() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let (mut stream, mut reader) = framed(&core);
    stream
        .write_all(&Frame::message("{}").encode()[..5])
        .unwrap();
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(reader.read(&mut [0u8; 16]).unwrap(), 0);
    wait_until("the core logs the error", || {
        core.stderr().contains("connection error")
    });
}

/// B7: line mode and framed mode take the same largest message: 16 MiB,
/// without the newline.
#[test]
fn line_mode_takes_a_message_of_16_mib_and_refuses_one_byte_more() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    for (len, accepted) in [(MAX_PAYLOAD, true), (MAX_PAYLOAD + 1, false)] {
        let stream = UnixStream::connect(&core.socket.path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let line = format!("{}\n", padded("core.hello", 7, len));
        let mut writer = stream.try_clone().unwrap();
        let sender = std::thread::spawn(move || {
            let _ = writer.write_all(line.as_bytes());
        });
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply).unwrap();
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(
            reply["result"]["protocol"].is_number(),
            accepted,
            "{len} bytes: {reply}"
        );
        drop(stream);
        sender.join().unwrap();
    }
}
