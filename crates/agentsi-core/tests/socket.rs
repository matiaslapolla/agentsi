//! The real `agentsi-core` program on a socket in a temp folder.

mod common;

use agentsi_proto::frame::{self, Frame, Kind};
use common::{BIN, CoreProc, HELLO, TempDir, alive, exchange, wait_until};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn json(line: &str) -> Value {
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn hello_gives_protocol_1_on_the_default_socket_with_modes_700_and_600() {
    let dir = TempDir::new();
    let runtime = dir.0.join("run");
    std::fs::create_dir(&runtime).unwrap();
    let socket = runtime.join("agentsi").join("core.sock");
    let mut child = Command::new(BIN)
        .arg("serve")
        .env_remove("AGENTSI_CORE_SOCKET")
        .env("XDG_RUNTIME_DIR", &runtime)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let core = CoreProc {
        child: None,
        socket: agentsi_proto::socket::SocketPath {
            path: socket.clone(),
            default: true,
        },
        log: dir.0.join("none"),
    };
    wait_until("the core listens", || UnixStream::connect(&socket).is_ok());

    let reply = json(exchange(&socket, &[HELLO]).trim());
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["result"]["protocol"], 1);
    assert_eq!(reply["result"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        reply["result"]["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "framed")
    );
    assert_eq!(mode(socket.parent().unwrap()), 0o700);
    assert_eq!(mode(&socket), 0o600);

    drop(core);
    assert!(child.wait().unwrap().success());
    assert!(
        !socket.exists(),
        "the core removes its socket when it stops"
    );
}

#[test]
fn a_second_core_finds_the_socket_alive_and_stops_with_exit_code_3() {
    let dir = TempDir::new();
    let first = CoreProc::start(&dir);
    let started = Instant::now();
    let status = CoreProc::command(&first.socket)
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3));
    assert!(started.elapsed() < Duration::from_secs(2));
    // The first core still answers.
    assert!(exchange(&first.socket.path, &[HELLO]).contains(r#""protocol":1"#));
}

#[test]
fn the_socket_of_a_killed_core_is_replaced() {
    let dir = TempDir::new();
    let mut killed = CoreProc::start(&dir);
    let child = killed.child.as_mut().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        killed.socket.path.exists(),
        "SIGKILL leaves the socket file"
    );
    let core = CoreProc::start(&dir);
    assert!(exchange(&core.socket.path, &[HELLO]).contains(r#""protocol":1"#));
}

#[test]
fn shutdown_replies_then_stops_the_core_and_removes_the_socket() {
    let dir = TempDir::new();
    let mut core = CoreProc::start(&dir);
    let out = exchange(&core.socket.path, &[common::SHUTDOWN]);
    assert_eq!(json(out.trim())["result"], serde_json::json!({}));
    let child = core.child.as_mut().unwrap();
    wait_until("the core stops", || child.try_wait().unwrap().is_some());
    assert!(!core.socket.path.exists());
}

#[test]
fn a_core_stops_when_its_socket_folder_is_deleted() {
    let dir = TempDir::new();
    let mut core = CoreProc::start(&dir);
    std::fs::remove_dir_all(&dir.0).unwrap();
    let child = core.child.as_mut().unwrap();
    wait_until("the core stops", || child.try_wait().unwrap().is_some());
}

#[test]
fn sigterm_stops_the_core_and_removes_the_socket() {
    let dir = TempDir::new();
    let mut core = CoreProc::start(&dir);
    // SAFETY: the pid is the core that this test started.
    unsafe { libc::kill(core.pid(), libc::SIGTERM) };
    let child = core.child.as_mut().unwrap();
    wait_until("the core stops", || child.try_wait().unwrap().is_some());
    assert!(!core.socket.path.exists());
}

#[test]
fn connect_relays_stdin_and_stdout_to_a_running_core() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let direct = exchange(&core.socket.path, &[HELLO]);
    let mut relay = Command::new(BIN)
        .arg("connect")
        .env("AGENTSI_CORE_SOCKET", &core.socket.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(relay.stdin.take().unwrap(), "{HELLO}").unwrap();
    let out = relay.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap(), direct);
}

#[test]
fn connect_starts_a_core_when_none_runs() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let core = CoreProc::adopt(socket.clone());
    let mut relay = Command::new(BIN)
        .arg("connect")
        .env("AGENTSI_CORE_SOCKET", &socket.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let relay_pid = relay.id() as i32;
    writeln!(relay.stdin.take().unwrap(), "{HELLO}").unwrap();
    let out = relay.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        json(String::from_utf8(out.stdout).unwrap().trim())["result"]["protocol"],
        1
    );

    // The started core runs on after the relay, in its own session.
    let pid = core.pid();
    assert_ne!(pid, relay_pid);
    assert!(alive(pid));
    // SAFETY: getsid only reads the session id of a process.
    assert_eq!(
        unsafe { libc::getsid(pid) },
        pid,
        "the core leads a new session"
    );
    assert!(
        std::fs::read_to_string(socket.log())
            .unwrap()
            .contains("listening on")
    );
    drop(core);
    assert!(!alive(pid), "the guard stops the started core");
}

#[test]
fn hello_with_framed_changes_the_connection_to_frames_after_the_reply() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut stream = UnixStream::connect(&core.socket.path).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{{"framed":true}}}}"#
    )
    .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(json(&line)["result"]["protocol"], 1);

    // Frames on a stream channel and of unknown kinds are ignored.
    stream
        .write_all(&Frame::new(5, Kind::DATA, b"keys".to_vec()).encode())
        .unwrap();
    stream
        .write_all(&Frame::new(0, Kind(200), b"?".to_vec()).encode())
        .unwrap();
    stream
        .write_all(&Frame::message(r#"{"jsonrpc":"2.0","id":2,"method":"rpc.discover"}"#).encode())
        .unwrap();
    let reply = frame::read_frame(&mut reader).unwrap().unwrap();
    assert_eq!((reply.channel, reply.kind), (0, Kind::DATA));
    let reply: Value = serde_json::from_slice(&reply.payload).unwrap();
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["result"]["info"]["x-stability"], "unstable until M2");

    stream.shutdown(std::net::Shutdown::Write).unwrap();
    assert_eq!(frame::read_frame(&mut reader).unwrap(), None);
}

#[test]
fn a_frame_over_16_mib_closes_the_connection() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut stream = UnixStream::connect(&core.socket.path).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    writeln!(
        stream,
        r#"{{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{{"framed":true}}}}"#
    )
    .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    reader.read_line(&mut String::new()).unwrap();
    let mut header = Frame::message("").encode();
    header[5..9].copy_from_slice(&((frame::MAX_PAYLOAD + 1) as u32).to_le_bytes());
    stream.write_all(&header).unwrap();
    assert_eq!(reader.read(&mut [0u8; 16]).unwrap(), 0);
    // The core still serves other connections.
    assert!(exchange(&core.socket.path, &[HELLO]).contains(r#""protocol":1"#));
}

/// Root in a user namespace whose ids map to the subordinate ids of this user.
const NS: &[&str] = &[
    "--map-users=auto",
    "--map-groups=auto",
    "--setuid",
    "0",
    "--setgid",
    "0",
];

/// Runs a client with another uid: root in a user namespace is a
/// subordinate uid of this user outside it. The test opens the modes of the folder and
/// the socket, so only the peer check can stop the client.
#[test]
fn a_connection_from_another_uid_is_closed_before_any_reply() {
    let unshare = |args: &[&str]| Command::new("unshare").args(NS).args(args).output();
    if !unshare(&["true"]).is_ok_and(|o| o.status.success())
        || Command::new("socat").arg("-V").output().is_err()
    {
        eprintln!("skipped: needs unshare --map-users=auto (newuidmap, /etc/subuid) and socat");
        return;
    }
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let folder = core.socket.path.parent().unwrap();
    std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o711)).unwrap();
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o711)).unwrap();
    std::fs::set_permissions(&core.socket.path, std::fs::Permissions::from_mode(0o666)).unwrap();
    let parents_open = dir
        .0
        .ancestors()
        .skip(1)
        .all(|p| std::fs::metadata(p).is_ok_and(|m| m.mode() & 0o001 != 0));
    if !parents_open {
        eprintln!(
            "skipped: the temp folder {} is closed to other users",
            dir.0.display()
        );
        return;
    }

    let mut client = Command::new("unshare")
        .args(NS)
        .args(["socat", "-t", "2", "-"])
        .arg(format!("UNIX-CONNECT:{}", core.socket.path.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(client.stdin.take().unwrap(), "{HELLO}").unwrap();
    let out = client.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "",
        "no reply: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let own = agentsi_core::server::euid();
    wait_until("the core logs the refused uid", || {
        core.stderr().contains("closed a connection from uid")
    });
    let log = core.stderr();
    let refused: u32 = log
        .split("closed a connection from uid ")
        .nth(1)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_ne!(refused, own, "{log}");
    // The owner still gets replies.
    assert!(exchange(&core.socket.path, &[HELLO]).contains(r#""protocol":1"#));
}
