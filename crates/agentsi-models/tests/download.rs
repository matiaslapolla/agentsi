//! Downloads against a local HTTP server. No network.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentsi_models::{Catalog, Downloader, Error, Event, State, Store};
use sha2::{Digest, Sha256};

#[derive(Default)]
struct Behavior {
    /// The first body is cut after this many bytes, with the full Content-Length announced.
    cut_first_at: Option<usize>,
    /// Answer 200 with the whole file, like a server with no ranges.
    ignore_range: bool,
    /// Pause after each 16 KiB written.
    slow: Option<Duration>,
    /// Answer 200 with an empty body.
    empty_200: bool,
    /// Answer this status with no body.
    status: Option<u16>,
    /// Serve these bytes in place of the real ones (same length).
    tamper: bool,
}

struct Server {
    base: String,
    ranges: Arc<Mutex<Vec<String>>>,
}

fn serve(files: HashMap<String, Vec<u8>>, behavior: Behavior) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let state = Arc::new((files, behavior, AtomicBool::new(false), ranges.clone()));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            std::thread::spawn(move || handle(stream, &state));
        }
    });
    Server { base, ranges }
}

type State_ = (
    HashMap<String, Vec<u8>>,
    Behavior,
    AtomicBool,
    Arc<Mutex<Vec<String>>>,
);

fn handle(stream: TcpStream, state: &State_) {
    let (files, behavior, cut_done, ranges) = state;
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut range = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header.trim().is_empty() {
            break;
        }
        if let Some(v) = header.to_ascii_lowercase().strip_prefix("range: bytes=") {
            range = Some(v.trim().to_owned());
        }
    }
    ranges
        .lock()
        .unwrap()
        .push(range.clone().unwrap_or_default());
    let mut out = stream;
    let Some(data) = files.get(&path) else {
        out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        return;
    };
    let (status, start, end) = match range.as_deref().and_then(|r| r.split_once('-')) {
        Some((s, e)) if !behavior.ignore_range => {
            let s: usize = s.parse().unwrap();
            let e: usize = e.parse::<usize>().unwrap().min(data.len() - 1);
            ("206 Partial Content", s, e)
        }
        _ => ("200 OK", 0, data.len() - 1),
    };
    if let Some(code) = behavior.status {
        out.write_all(
            format!("HTTP/1.1 {code} Err\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
        return;
    }
    if behavior.empty_200 {
        out.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        return;
    }
    let mut body = data[start..=end].to_vec();
    if behavior.tamper {
        body[0] ^= 0xff;
    }
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if status.starts_with("206") {
        head += &format!("Content-Range: bytes {start}-{end}/{}\r\n", data.len());
    }
    head += "\r\n";
    let _ = out.write_all(head.as_bytes());
    let cut = behavior
        .cut_first_at
        .filter(|_| !cut_done.swap(true, Ordering::SeqCst));
    let limit = cut.map_or(body.len(), |c| c.min(body.len()));
    for chunk in body[..limit].chunks(16 * 1024) {
        if out.write_all(chunk).is_err() {
            return;
        }
        if let Some(d) = behavior.slow {
            std::thread::sleep(d);
        }
    }
    let _ = out.flush();
}

fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

struct Fixture {
    catalog: Catalog,
    files: HashMap<String, Vec<u8>>,
    store: Store,
    _dir: tempdir::Dir,
}

mod tempdir {
    pub struct Dir(pub std::path::PathBuf);
    impl Dir {
        pub fn new(tag: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("agentsi-models-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

const REV: &str = "0123456789abcdef0123456789abcdef01234567";

/// One model `m` with `a.bin` (300 000 bytes) and `b.txt` (1 000 bytes, path `sub/b.txt` in the repo).
fn fixture(tag: &str) -> Fixture {
    let a = bytes(1, 300_000);
    let b = bytes(2, 1_000);
    let catalog = Catalog::parse(&format!(
        r#"
[[model]]
id = "m"
name = "Test"
kind = "speech-in"
license = "MIT"
description = "test"
repo = "o/r"
revision = "{REV}"
[[model.file]]
name = "a.bin"
size = {}
sha256 = "{}"
[[model.file]]
name = "b.txt"
path = "sub/b.txt"
size = {}
sha256 = "{}"
"#,
        a.len(),
        sha(&a),
        b.len(),
        sha(&b)
    ))
    .unwrap();
    let files = HashMap::from([
        (format!("/o/r/resolve/{REV}/a.bin"), a),
        (format!("/o/r/resolve/{REV}/sub/b.txt"), b),
    ]);
    let dir = tempdir::Dir::new(tag);
    let store = Store::new(dir.0.join("models"));
    Fixture {
        catalog,
        files,
        store,
        _dir: dir,
    }
}

fn fast(server: &Server) -> Downloader {
    Downloader::with_base_url(&server.base).retry_delay(Duration::ZERO)
}

fn names(dir: PathBuf) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[test]
fn full_download_writes_files_and_receipt() {
    let f = fixture("full");
    let server = serve(f.files.clone(), Behavior::default());
    let model = f.catalog.get("m").unwrap();
    let mut events = Vec::new();
    let receipt = fast(&server)
        .piece_size(100_000)
        .download(&f.store, model, &AtomicBool::new(false), |e| events.push(e))
        .unwrap();
    assert_eq!(receipt.id, "m");
    assert_eq!(f.store.state(model), State::Ready);
    assert_eq!(
        std::fs::read(f.store.file_path(model, "a.bin")).unwrap(),
        f.files[&format!("/o/r/resolve/{REV}/a.bin")]
    );
    assert_eq!(
        names(f.store.model_dir(model)),
        [".lock", "a.bin", "b.txt", "receipt.json"]
    );
    assert_eq!(events.last(), Some(&Event::Done));
    assert!(events.contains(&Event::Verifying {
        file: "a.bin".into()
    }));
    let last = events.iter().rev().find_map(|e| match e {
        Event::Progress { bytes, total, .. } => Some((*bytes, *total)),
        _ => None,
    });
    assert_eq!(last, Some((301_000, 301_000)));
    // 3 ranged pieces for a.bin (100 000 each), 1 for b.txt.
    assert_eq!(server.ranges.lock().unwrap().len(), 4);
    // A Ready model makes no request.
    fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap();
    assert_eq!(server.ranges.lock().unwrap().len(), 4);
}

#[test]
fn resumes_after_a_cut_connection() {
    let f = fixture("cut");
    let server = serve(
        f.files.clone(),
        Behavior {
            cut_first_at: Some(150_000),
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap();
    let mut events = Vec::new();
    fast(&server)
        .piece_size(1 << 20)
        .download(&f.store, model, &AtomicBool::new(false), |e| events.push(e))
        .unwrap();
    assert_eq!(f.store.state(model), State::Ready);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Retry { attempt: 1, .. }))
    );
    let ranges = server.ranges.lock().unwrap().clone();
    assert_eq!(ranges[0], "0-299999");
    // The second request asks for the rest, never for byte 0 again.
    assert!(ranges[1].starts_with("150000-"), "{ranges:?}");
}

#[test]
fn a_stopped_download_continues_from_where_it_stopped() {
    let f = fixture("stop");
    let server = serve(f.files.clone(), Behavior::default());
    let model = f.catalog.get("m").unwrap();
    let cancel = AtomicBool::new(false);
    // The progress event after the first 100 000-byte piece stops the run: 33 % of the model.
    let err = fast(&server)
        .piece_size(100_000)
        .download(&f.store, model, &cancel, |e| {
            if let Event::Progress {
                file, file_bytes, ..
            } = e
                && file == "a.bin"
                && file_bytes >= 100_000
            {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap_err();
    assert!(matches!(err, Error::Canceled), "{err}");
    let partial = f.store.model_dir(model).join("a.bin.partial");
    let have = std::fs::metadata(&partial).unwrap().len();
    assert!((100_000..300_000).contains(&have), "{have}");
    assert_eq!(f.store.state(model), State::Partial { bytes: have });
    server.ranges.lock().unwrap().clear();
    fast(&server)
        .piece_size(100_000)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap();
    let first = server.ranges.lock().unwrap()[0].clone();
    assert!(
        first.starts_with(&format!("{have}-")),
        "{first} after {have} bytes"
    );
    assert_eq!(f.store.state(model), State::Ready);
}

#[test]
fn a_wrong_hash_leaves_no_file() {
    let f = fixture("hash");
    let server = serve(
        f.files.clone(),
        Behavior {
            tamper: true,
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap();
    let err = fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert!(
        matches!(err, Error::HashMismatch { ref file, .. } if file == "a.bin"),
        "{err}"
    );
    assert_eq!(err.kind(), "hash_mismatch");
    assert_eq!(f.store.state(model), State::NotDownloaded);
    assert_eq!(names(f.store.model_dir(model)), [".lock"]);
}

#[test]
fn two_downloads_of_one_model_do_not_run_together() {
    let f = fixture("lock");
    let server = serve(
        f.files.clone(),
        Behavior {
            slow: Some(Duration::from_millis(20)),
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap().clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let first = {
        let (store, base, model) = (f.store.clone(), server.base.clone(), model.clone());
        std::thread::spawn(move || {
            let mut sent = false;
            Downloader::with_base_url(base).download(&store, &model, &AtomicBool::new(false), |e| {
                if !sent && matches!(e, Event::Progress { .. }) {
                    sent = true;
                    started_tx.send(()).unwrap();
                }
            })
        })
    };
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let second = fast(&server).download(&f.store, &model, &AtomicBool::new(false), |_| {});
    assert!(
        matches!(second, Err(Error::Busy(ref id)) if id == "m"),
        "{second:?}"
    );
    assert!(matches!(f.store.delete(&model), Err(Error::Busy(_))));
    first.join().unwrap().unwrap();
    assert_eq!(f.store.state(&model), State::Ready);
    // The lock is free again.
    f.store.delete(&model).unwrap();
    assert_eq!(f.store.state(&model), State::NotDownloaded);
    assert_eq!(names(f.store.model_dir(&model)), [".lock"]);
}

#[test]
fn a_server_without_ranges_sends_the_whole_file() {
    let f = fixture("noranges");
    let server = serve(
        f.files.clone(),
        Behavior {
            ignore_range: true,
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap();
    fast(&server)
        .piece_size(100_000)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap();
    assert_eq!(f.store.state(model), State::Ready);
}

#[test]
fn a_missing_file_fails_without_retries() {
    let f = fixture("404");
    let server = serve(HashMap::new(), Behavior::default());
    let model = f.catalog.get("m").unwrap();
    let err = fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert_eq!(err.kind(), "network");
    assert!(err.to_string().contains("404"), "{err}");
    assert_eq!(server.ranges.lock().unwrap().len(), 1);
}

#[test]
fn a_good_file_already_in_place_is_adopted() {
    let f = fixture("adopt");
    let server = serve(f.files.clone(), Behavior::default());
    let model = f.catalog.get("m").unwrap();
    std::fs::create_dir_all(f.store.model_dir(model)).unwrap();
    std::fs::write(
        f.store.file_path(model, "a.bin"),
        &f.files[&format!("/o/r/resolve/{REV}/a.bin")],
    )
    .unwrap();
    fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap();
    // Only b.txt was fetched.
    assert_eq!(server.ranges.lock().unwrap().len(), 1);
    assert_eq!(f.store.state(model), State::Ready);
}

#[test]
fn a_changed_catalog_entry_is_not_ready() {
    let f = fixture("rev");
    let server = serve(f.files.clone(), Behavior::default());
    let model = f.catalog.get("m").unwrap();
    fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap();
    let mut newer = model.clone();
    newer.revision = "f".repeat(40);
    assert_ne!(f.store.state(&newer), State::Ready);
}

#[test]
fn an_empty_answer_ends_in_an_error_not_a_loop() {
    let f = fixture("empty");
    let server = serve(
        f.files.clone(),
        Behavior {
            empty_200: true,
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap();
    let err = fast(&server)
        .attempts(3)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert_eq!(err.kind(), "network");
    assert_eq!(server.ranges.lock().unwrap().len(), 3);
}

#[test]
fn a_server_error_is_retried_then_reported() {
    let f = fixture("503");
    let server = serve(
        f.files.clone(),
        Behavior {
            status: Some(503),
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap();
    let mut retries = 0;
    let err = fast(&server)
        .attempts(3)
        .download(&f.store, model, &AtomicBool::new(false), |e| {
            retries += usize::from(matches!(e, Event::Retry { .. }))
        })
        .unwrap_err();
    assert_eq!(err.kind(), "network");
    assert_eq!(retries, 2);
    assert_eq!(server.ranges.lock().unwrap().len(), 3);
}

#[test]
fn a_huge_piece_size_does_not_overflow() {
    let f = fixture("huge");
    let server = serve(
        f.files.clone(),
        Behavior {
            cut_first_at: Some(1000),
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap();
    fast(&server)
        .piece_size(u64::MAX)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap();
    assert_eq!(f.store.state(model), State::Ready);
}

#[test]
fn a_foreign_file_is_never_replaced_or_deleted() {
    let f = fixture("foreign");
    let server = serve(f.files.clone(), Behavior::default());
    let model = f.catalog.get("m").unwrap();
    let dir = f.store.model_dir(model);
    std::fs::create_dir_all(&dir).unwrap();
    let mine = b"the owner's own file, not the pinned one".to_vec();
    std::fs::write(dir.join("a.bin"), &mine).unwrap();
    std::fs::write(dir.join("notes.txt"), b"unrelated").unwrap();
    let err = fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert!(
        matches!(err, Error::Conflict { ref file } if file == "a.bin"),
        "{err}"
    );
    assert_eq!(err.kind(), "conflict");
    assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), mine);
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"unrelated");
    assert!(server.ranges.lock().unwrap().is_empty());
    // A delete leaves them too.
    f.store.delete(model).unwrap();
    assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), mine);
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"unrelated");
}

#[test]
fn the_app_folder_is_separate_from_the_python_voice_folders() {
    let f = fixture("layout");
    let model = f.catalog.get("m").unwrap();
    assert_eq!(
        f.store.model_dir(model),
        f.store.root().join("catalog").join("m")
    );
}

#[test]
fn a_delete_after_a_failed_later_file_removes_the_checked_earlier_file() {
    let f = fixture("later");
    let mut files = f.files.clone();
    let b = files
        .get_mut(&format!("/o/r/resolve/{REV}/sub/b.txt"))
        .unwrap();
    b[0] ^= 0xff;
    let server = serve(files, Behavior::default());
    let model = f.catalog.get("m").unwrap();
    let err = fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert!(matches!(err, Error::HashMismatch { ref file, .. } if file == "b.txt"));
    let dir = f.store.model_dir(model);
    assert_eq!(names(dir.clone()), [".lock", "a.bin"]);
    f.store.delete(model).unwrap();
    assert_eq!(names(dir), [".lock"]);
}

#[test]
fn a_file_that_appears_during_the_download_is_not_replaced() {
    let f = fixture("appears");
    let server = serve(
        f.files.clone(),
        Behavior {
            slow: Some(Duration::from_millis(30)),
            ..Default::default()
        },
    );
    let model = f.catalog.get("m").unwrap().clone();
    let store = f.store.clone();
    let dir = store.model_dir(&model);
    let downloader = fast(&server);
    let run = std::thread::spawn(move || {
        downloader.download(&store, &model, &AtomicBool::new(false), |_| {})
    });
    // a.bin takes 19 pieces of 16 KiB, 30 ms apart.
    let partial = dir.join("a.bin.partial");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !partial.exists() {
        assert!(std::time::Instant::now() < deadline, "no partial file");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mine = b"the owner's file".to_vec();
    std::fs::write(dir.join("a.bin"), &mine).unwrap();
    let err = run.join().unwrap().unwrap_err();
    assert!(
        matches!(err, Error::Conflict { ref file } if file == "a.bin"),
        "{err}"
    );
    assert_eq!(std::fs::read(dir.join("a.bin")).unwrap(), mine);
}

#[test]
fn the_files_of_an_older_catalog_entry_stay_deletable() {
    let f = fixture("older");
    let server = serve(f.files.clone(), Behavior::default());
    let model = f.catalog.get("m").unwrap();
    fast(&server)
        .download(&f.store, model, &AtomicBool::new(false), |_| {})
        .unwrap();
    // A newer catalog pins another a.bin. The old one is a conflict, and the old receipt
    // still lists it, so a delete removes it.
    let mut newer = model.clone();
    newer.revision = "f".repeat(40);
    newer.files[0].size += 1;
    newer.files[0].sha256 = "0".repeat(64);
    let err = fast(&server)
        .download(&f.store, &newer, &AtomicBool::new(false), |_| {})
        .unwrap_err();
    assert!(matches!(err, Error::Conflict { ref file } if file == "a.bin"));
    f.store.delete(&newer).unwrap();
    assert_eq!(names(f.store.model_dir(&newer)), [".lock"]);
}
