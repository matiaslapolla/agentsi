//! The `terminal.*` methods on a real core: the first paint, the live bytes,
//! the query filter, the state events, flow control, and the paths of many,
//! slow and lost panes.

mod common;

use agentsi_proto::frame::{Frame, Kind};
use common::framed::{Framed, Pane, rss_kib};
use common::{CoreProc, TempDir, wait_until};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

fn sh(script: &str) -> Value {
    json!({"cwd": std::env::temp_dir(), "argv": ["sh", "-c", script]})
}

fn create(c: &mut Framed, script: &str) -> String {
    c.ok("terminal.create", sh(script))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn attach(c: &mut Framed, id: &str, observe: bool) -> Pane {
    let r = c.ok("terminal.attach", json!({"id": id, "observe": observe}));
    Pane::new(r["channel"].as_u64().unwrap() as u32)
}

fn subscribe(c: &mut Framed) {
    c.ok("core.subscribe", json!({"topics": ["terminal"]}));
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn a_pane_gets_the_first_paint_then_the_live_bytes_then_the_close() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let id = create(
        &mut c,
        "printf 'before attach\\n'; read x; printf 'got:%s\\n' \"$x\"; exit 7",
    );
    // The first paint shows the output from before the attach.
    std::thread::sleep(Duration::from_millis(200));
    let mut pane = attach(&mut c, &id, false);
    pane.read_until(&mut c, "the first paint", |p| !p.bytes.is_empty());
    assert_eq!(pane.resizes, [(80, 24)]);
    assert!(pane.bytes.starts_with(b"\x1bc"));
    pane.read_until(&mut c, "the old output", |p| {
        p.screen.grid().unwrap().text_lines()[0] == "before attach"
    });

    c.ok(
        "terminal.input",
        json!({"id": id, "text": "hello", "submit": true}),
    );
    pane.read_until(&mut c, "the close", |p| p.close.is_some());
    assert!(pane.text().contains("got:hello"), "{}", pane.text());
    assert_eq!(pane.close, Some(json!({"reason": "exit", "code": 7})));

    let list = c.ok("terminal.list", json!({}));
    let t = &list["terminals"][0];
    assert_eq!(
        (t["running"].clone(), t["code"].clone()),
        (json!(false), json!(7))
    );
    // A stopped terminal stays until kill.
    c.ok("terminal.kill", json!({"id": id}));
    assert_eq!(c.ok("terminal.list", json!({}))["terminals"], json!([]));
}

#[test]
fn the_child_gets_the_ids_and_not_the_claude_code_variables() {
    let dir = TempDir::new();
    let socket = dir.socket();
    let mut cmd = CoreProc::command(&socket);
    cmd.env("CLAUDECODE", "1")
        .env("CLAUDE_CODE_ENTRYPOINT", "cli")
        .env("CLAUDE_JOB_DIR", "/x")
        .env("CLAUDE_BG_X", "1")
        .stderr(std::process::Stdio::null());
    let child = cmd.spawn().unwrap();
    let core = CoreProc {
        child: Some(child),
        socket: socket.clone(),
        log: dir.0.join("none"),
    };
    wait_until("the core listens", || {
        std::os::unix::net::UnixStream::connect(&socket.path).is_ok()
    });
    let mut c = Framed::connect(&core.socket.path);
    let r = c.ok(
        "terminal.create",
        json!({"cwd": "/", "argv": ["sh", "-c", "echo \"[$AGENTSI_TERMINAL][$AGENTSI_CORE_SOCKET][${CLAUDECODE-}${CLAUDE_CODE_ENTRYPOINT-}${CLAUDE_JOB_DIR-}${CLAUDE_BG_X-}][$MINE][$(pwd)]\"; sleep 5"],
               "env": {"MINE": "yes"}, "product": "agentsi", "cols": 100, "rows": 30}),
    );
    assert_eq!(r["product"], "agentsi");
    assert_eq!(
        (r["cols"].clone(), r["rows"].clone()),
        (json!(100), json!(30))
    );
    let id = r["id"].as_str().unwrap().to_string();
    let mut pane = attach(&mut c, &id, true);
    let expected = format!("[{id}][{}][][yes][/]", socket.path.display());
    pane.read_until(&mut c, "the environment line", |p| {
        String::from_utf8_lossy(&p.bytes).contains(&expected)
    });
    assert_eq!(pane.resizes, [(100, 30)]);
}

#[test]
fn the_core_answers_the_queries_in_order_and_the_pane_never_sees_them() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    // The child reads the replies in raw mode and prints them in hex.
    let id = create(
        &mut c,
        "stty raw -echo; printf 'start\\033]7501;?\\033\\\\\\033[c'; \
         timeout --foreground 1 dd bs=1 count=200 >/dev/null 2>&1; \
         printf '\\033[cend\\r\\n'; stty sane; sleep 0.5",
    );
    let mut pane = attach(&mut c, &id, false);
    pane.read_until(&mut c, "the end", |p| find(&p.bytes, b"end").is_some());
    assert!(
        find(&pane.bytes, b"\x1b[c").is_none(),
        "a DA1 query reached the pane"
    );
    assert!(
        find(&pane.bytes, b"\x1b]7501;?").is_none(),
        "a 7501 query reached the pane"
    );
    assert!(find(&pane.bytes, b"start").is_some());
}

#[test]
fn the_replies_reach_the_program_with_7501_before_da1() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let id = create(
        &mut c,
        "stty raw -echo; printf '\\033]7501;?\\033\\\\\\033[c'; \
         timeout --foreground 1 dd bs=1 count=400 2>/dev/null | od -An -tx1 | tr -d ' \\n'; printf '\\r\\nEOF\\r\\n'; stty sane; sleep 1",
    );
    let mut pane = attach(&mut c, &id, true);
    pane.read_until(&mut c, "the hex of the replies", |p| {
        find(&p.bytes, b"EOF").is_some()
    });
    let text = String::from_utf8_lossy(&pane.bytes).to_string();
    let status = text.find("1b5d37353031").expect("a 7501 reply"); // ESC ] 7501
    let da1 = text.find("1b5b3f").expect("a DA1 reply"); // ESC [ ?
    assert!(status < da1, "{text}");
}

#[test]
fn a_subscriber_sees_the_program_state_and_the_exit() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    subscribe(&mut c);
    let msg = "QWxsb3cgcm0/"; // "Allow rm?" in base64
    let id = create(
        &mut c,
        &format!(
            "printf '\\033]7501;state=working:app=test\\007'; sleep 0.2; \
             printf '\\033]7501;state=working:app=test\\007'; \
             printf '\\033]0;my title\\007'; \
             printf '\\033]7501;state=blocked:app=test:kind=permission:msg={msg}\\007'; sleep 0.2; exit 3"
        ),
    );
    let working = c.next_event("terminal.state");
    assert_eq!(working, json!({"id": id, "state": "working"}));
    let title = c.next_event("terminal.title");
    assert_eq!(title, json!({"id": id, "title": "my title"}));
    let blocked = c.next_event("terminal.state");
    assert_eq!(
        blocked,
        json!({"id": id, "state": "blocked", "kind": "permission", "msg": "Allow rm?"})
    );
    let t = &c.ok("terminal.list", json!({}))["terminals"][0];
    assert_eq!(
        (t["state"].clone(), t["kind"].clone()),
        (json!("blocked"), json!("permission"))
    );
    // At the exit, the blocked state does not hold.
    let cleared = c.next_event("terminal.state");
    assert_eq!(cleared, json!({"id": id, "state": "idle"}));
    let exit = c.next_event("terminal.exit");
    assert_eq!(exit, json!({"id": id, "code": 3}));
}

#[test]
fn a_test_pane_prints_100_mb_with_no_reader_and_the_core_grows_less_than_4_mb() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut control = Framed::connect(&core.socket.path);
    subscribe(&mut control);
    let id = create(
        &mut control,
        "read go; yes 'a line of output from a busy program 0123456789' | head -c 100000000; echo",
    );
    // The pane attaches and then reads nothing and returns no credit.
    let mut pane = Framed::connect(&core.socket.path);
    pane.ok("terminal.attach", json!({"id": id}));
    std::thread::sleep(Duration::from_millis(300));
    let before = rss_kib(core.pid());
    let start = Instant::now();
    control.ok("terminal.input", json!({"id": id, "text": "go\n"}));
    let exit = control.next_event("terminal.exit");
    let after = rss_kib(core.pid());
    let grown = after.saturating_sub(before);
    eprintln!(
        "100 MB in {:?}: RSS {before} KiB -> {after} KiB, grown {grown} KiB",
        start.elapsed()
    );
    assert_eq!(exit["code"], 0);
    assert!(grown < 4 * 1024, "the core grew {grown} KiB");
    // The core still serves the pane connection.
    assert!(pane.ok("terminal.list", json!({}))["terminals"].is_array());
}

#[test]
fn a_slow_pane_gets_a_new_first_paint_and_the_end_of_the_output() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    subscribe(&mut c);
    let id = create(&mut c, "read go; seq 1 300000; sleep 0.5");
    let mut pc = Framed::connect(&core.socket.path);
    let mut pane = attach(&mut pc, &id, false);
    pane.credit = false;
    pane.read_until(&mut pc, "the first paint", |p| !p.bytes.is_empty());
    c.ok("terminal.input", json!({"id": id, "text": "go\n"}));
    c.next_event("terminal.exit");
    // The pane did not read during the output. Now it reads, and returns its credit.
    pane.credit = true;
    pc.send(&Frame::credit(
        pane.channel,
        agentsi_proto::frame::STREAM_WINDOW as u32,
    ));
    pane.read_until(&mut pc, "the close", |p| p.close.is_some());
    let lines = pane.screen.grid().unwrap().text_lines();
    assert!(lines.iter().any(|l| l == "300000"), "{lines:?}");
    // The output was 2 MB. The pane got a resync, not every byte.
    assert!(pane.resizes.len() >= 2, "no resync: {:?}", pane.resizes);
}

#[test]
fn many_panes_on_one_terminal_get_the_same_screen() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut clients: Vec<(Framed, Pane)> = (0..8)
        .map(|_| Framed::connect(&core.socket.path))
        .collect::<Vec<_>>()
        .into_iter()
        .map(|c| (c, Pane::new(0)))
        .collect();
    let id = create(
        &mut clients[0].0,
        "read go; seq 1 20000; printf '\\033[1;31mred\\033[0m'; sleep 0.5",
    );
    for (i, (c, p)) in clients.iter_mut().enumerate() {
        *p = attach(c, &id, i % 2 == 1);
    }
    clients[0]
        .0
        .ok("terminal.input", json!({"id": id, "text": "go\n"}));
    let mut screens = Vec::new();
    for (c, p) in clients.iter_mut() {
        p.read_until(c, "the close", |p| p.close.is_some());
        screens.push(p.screen.grid().unwrap());
    }
    assert!(screens[0].text_lines().iter().any(|l| l.ends_with("red")));
    for s in &screens[1..] {
        assert_eq!(s, &screens[0]);
    }
}

#[test]
fn a_pane_that_goes_away_mid_stream_does_not_stop_the_terminal() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let id = create(
        &mut c,
        "read go; seq 1 200000; read again; echo second; sleep 0.3",
    );
    {
        let mut gone = Framed::connect(&core.socket.path);
        let mut p = attach(&mut gone, &id, false);
        c.ok("terminal.input", json!({"id": id, "text": "go\n"}));
        p.read_until(&mut gone, "some output", |p| p.bytes.len() > 10_000);
        // Dropping the client closes its socket in the middle of the stream.
    }
    let mut pane = attach(&mut c, &id, false);
    c.ok("terminal.input", json!({"id": id, "text": "x\n"}));
    pane.read_until(&mut c, "the close", |p| p.close.is_some());
    assert!(pane.text().contains("second"));
    assert_eq!(pane.close, Some(json!({"reason": "exit", "code": 0})));
}

#[test]
fn a_pane_that_attaches_while_the_program_exits_always_gets_a_close() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    for round in 0..30 {
        let id = create(&mut c, "printf x; exit 5");
        if round % 3 == 0 {
            std::thread::sleep(Duration::from_millis(round as u64));
        }
        let mut pane = attach(&mut c, &id, false);
        pane.read_until(&mut c, "the close", |p| p.close.is_some());
        assert_eq!(pane.close, Some(json!({"reason": "exit", "code": 5})));
        c.ok("terminal.kill", json!({"id": id}));
    }
    assert_eq!(c.ok("terminal.list", json!({}))["terminals"], json!([]));
}

#[test]
fn kill_stops_the_program_and_removes_the_terminal() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    subscribe(&mut c);
    // The program ignores SIGHUP and SIGTERM: SIGKILL ends it after the grace period.
    let id = create(
        &mut c,
        "trap '' HUP TERM; echo ready; while :; do sleep 0.1; done",
    );
    let mut pane = attach(&mut c, &id, false);
    pane.read_until(&mut c, "ready", |p| find(&p.bytes, b"ready").is_some());
    let start = Instant::now();
    c.ok("terminal.kill", json!({"id": id}));
    let exit = c.next_event("terminal.exit");
    assert_eq!(exit, json!({"id": id, "code": null}));
    assert!(start.elapsed() >= Duration::from_millis(1500));
    pane.read_until(&mut c, "the close", |p| p.close.is_some());
    assert_eq!(pane.close, Some(json!({"reason": "exit", "code": null})));
    wait_until("the terminal leaves the list", || {
        c.ok("terminal.list", json!({}))["terminals"] == json!([])
    });
}

#[test]
fn resize_and_focus_frames_reach_the_program_and_the_other_panes() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let id = create(
        &mut c,
        "printf '\\033[?1004h'; stty raw -echo; \
         timeout --foreground 2 dd bs=1 count=3 2>/dev/null | od -An -c | tr -d ' \\n'; \
         stty sane; printf '\\nsize:'; stty size; sleep 1",
    );
    let mut main = attach(&mut c, &id, false);
    let mut observer = attach(&mut c, &id, true);
    main.read_until(&mut c, "the paint", |p| !p.bytes.is_empty());
    std::thread::sleep(Duration::from_millis(300));
    // The frames of the observer do nothing: its FOCUS 0 would write ESC [ O.
    c.send(&Frame::focus(observer.channel, false));
    c.send(&Frame::resize(observer.channel, 50, 10));
    c.send(&Frame::focus(main.channel, true));
    c.send(&Frame::resize_px(main.channel, 120, 40, 9, 18));
    let deadline = Instant::now() + Duration::from_secs(10);
    while main.close.is_none() || observer.close.is_none() {
        assert!(Instant::now() < deadline);
        let f = c.next_frame();
        if !main.take(&mut c, &f) {
            observer.take(&mut c, &f);
        }
    }
    let text = main.text();
    assert!(text.contains("033[I"), "{text}");
    assert!(text.contains("size:40 120"), "{text}");
    assert_eq!(observer.resizes, [(80, 24), (120, 40)]);
    assert_eq!(main.screen.grid().unwrap(), observer.screen.grid().unwrap());
}

#[test]
fn the_errors_have_the_stable_kinds() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let kind = |v: Value| v["error"]["data"]["kind"].clone();
    assert_eq!(
        kind(c.call("terminal.input", json!({"id": "t99", "text": "x"}))),
        "not_found"
    );
    assert_eq!(
        kind(c.call("terminal.attach", json!({"id": "nope"}))),
        "not_found"
    );
    assert_eq!(
        kind(c.call("terminal.kill", json!({"id": "t0"}))),
        "not_found"
    );
    assert_eq!(
        kind(c.call("terminal.create", json!({"cwd": "/", "argv": []}))),
        "invalid"
    );
    assert_eq!(
        kind(c.call(
            "terminal.create",
            json!({"cwd": "/no/such/dir", "argv": ["sh"]})
        )),
        "invalid"
    );
    assert_eq!(
        kind(c.call(
            "terminal.create",
            json!({"cwd": "/", "argv": ["/no/such/program"]})
        )),
        "invalid"
    );
    let id = create(&mut c, "exit 0");
    assert_eq!(
        kind(c.call("terminal.resize", json!({"id": id, "cols": 0, "rows": 5}))),
        "invalid"
    );
    wait_until("the exit", || {
        c.ok("terminal.list", json!({}))["terminals"][0]["running"] == json!(false)
    });
    assert_eq!(
        kind(c.call("terminal.input", json!({"id": id, "text": "x"}))),
        "conflict"
    );

    // attach needs framed mode.
    let out = common::exchange(
        &core.socket.path,
        &[r#"{"jsonrpc":"2.0","id":1,"method":"terminal.attach","params":{"id":"t1"}}"#],
    );
    let v: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(kind(v), "invalid");
}

#[test]
fn an_unknown_channel_and_a_client_close_are_harmless() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let id = create(&mut c, "read x; echo after; sleep 0.3");
    c.send(&Frame::new(77, Kind::DATA, b"nobody".to_vec()));
    c.send(&Frame::credit(77, 5));
    let mut first = attach(&mut c, &id, false);
    first.read_until(&mut c, "the paint", |p| !p.bytes.is_empty());
    c.send(&Frame::close(first.channel, "{}"));
    let mut second = attach(&mut c, &id, false);
    c.ok("terminal.input", json!({"id": id, "text": "go\n"}));
    while second.close.is_none() {
        let f = c.next_frame();
        if !second.take(&mut c, &f) {
            first.take(&mut c, &f);
        }
    }
    assert!(second.text().contains("after"));
    // The first pane got nothing after its CLOSE.
    assert!(find(&first.bytes, b"after").is_none());
    assert_eq!(first.close, None);
}

#[test]
fn a_stream_frame_never_comes_before_the_reply_that_names_its_channel() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut setup = Framed::connect(&core.socket.path);
    let id = create(&mut setup, "echo ready; sleep 5");
    for _ in 0..20 {
        let mut c = Framed::connect(&core.socket.path);
        // Two requests in flight at the same time.
        for n in [1, 2] {
            let request = json!({"jsonrpc": "2.0", "id": n, "method": "terminal.attach", "params": {"id": id}});
            c.send(&Frame::message(&request.to_string()));
        }
        let mut known = Vec::new();
        let mut painted = 0;
        while painted < 2 {
            let f = c.next_frame();
            if f.channel == 0 {
                let v: Value = serde_json::from_slice(&f.payload).unwrap();
                known.push(v["result"]["channel"].as_u64().unwrap() as u32);
            } else {
                assert!(
                    known.contains(&f.channel),
                    "a frame on channel {} before its reply",
                    f.channel
                );
                if f.kind == Kind::DATA {
                    painted += 1;
                }
            }
        }
    }
}

/// Review B1: input that the program does not read must not stop the connection.
#[test]
fn input_past_the_budget_does_not_stop_the_connection() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    // The program never reads its input.
    let id = create(&mut c, "stty raw -echo; sleep 30");
    let pane = attach(&mut c, &id, false);
    std::thread::sleep(Duration::from_millis(200));
    let mb = vec![b'x'; 1 << 20];
    for _ in 0..10 {
        c.send(&Frame::new(pane.channel, Kind::DATA, mb.clone()));
    }
    let start = Instant::now();
    assert!(c.ok("terminal.list", json!({}))["terminals"].is_array());
    let busy = c.call("terminal.input", json!({"id": id, "text": "more"}));
    assert_eq!(busy["error"]["data"]["kind"], "busy", "{busy}");
    c.ok("terminal.kill", json!({"id": id}));
    assert!(start.elapsed() < Duration::from_secs(5));
}

/// Review B2: kill reaches a process that keeps the PTY open after the program stopped.
#[test]
fn kill_after_the_exit_stops_the_rest_of_the_process_group() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    subscribe(&mut c);
    let id = create(&mut c, "trap '' HUP; sleep 60 & echo \"bg=$!\"; exit 0");
    let mut pane = attach(&mut c, &id, true);
    pane.read_until(&mut c, "the pid", |p| find(&p.bytes, b"bg=").is_some());
    let text = String::from_utf8_lossy(&pane.bytes).to_string();
    let pid: i32 = text
        .split("bg=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    c.next_event("terminal.exit");
    assert!(common::alive(pid));
    c.ok("terminal.kill", json!({"id": id}));
    wait_until("the background job stops", || {
        !common::alive(pid) || is_zombie(pid)
    });
    assert_eq!(c.ok("terminal.list", json!({}))["terminals"], json!([]));
}

fn is_zombie(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .map(|s| {
            s.rsplit(')')
                .next()
                .unwrap_or("")
                .trim_start()
                .starts_with('Z')
        })
        .unwrap_or(true)
}

/// Review B3: the program loses the focus only when the pane with the focus loses it.
#[test]
fn a_focus_out_of_a_pane_without_the_focus_does_not_reach_the_program() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let id = create(
        &mut c,
        "printf '\\033[?1004h'; stty raw -echo; \
         timeout --foreground 2 dd bs=1 count=12 2>/dev/null | od -An -c | tr -d ' \\n'; \
         stty sane; printf '\\nend\\n'; sleep 0.5",
    );
    let mut a = attach(&mut c, &id, false);
    let b = attach(&mut c, &id, false);
    a.read_until(&mut c, "the paint", |p| !p.bytes.is_empty());
    std::thread::sleep(Duration::from_millis(300));
    c.send(&Frame::focus(a.channel, true));
    c.send(&Frame::focus(b.channel, true));
    c.send(&Frame::focus(a.channel, false));
    a.read_until(&mut c, "the end", |p| find(&p.bytes, b"end").is_some());
    let text = a.text();
    assert!(text.contains("033[I"), "{text}");
    assert!(!text.contains("033[O"), "{text}");
}
