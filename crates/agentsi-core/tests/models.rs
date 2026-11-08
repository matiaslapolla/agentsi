//! `models.*` against a local HTTP server and a small catalog. No network, no real model.

mod common;

use agentsi_core::methods::models::Models;
use agentsi_core::server::{bind, euid};
use agentsi_core::state::Core;
use agentsi_models::{Catalog, Downloader, Store};
use common::TempDir;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const REPO: &str = "test/voices";
const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A catalog of models with one file each: `(id, file name, pinned bytes)`.
fn catalog(models: &[(&str, &str, &[u8])]) -> Catalog {
    let mut text = String::new();
    for (id, name, bytes) in models {
        text.push_str(&format!(
            "[[model]]\nid = \"{id}\"\nname = \"Model {id}\"\nkind = \"speech-out\"\n\
             license = \"Apache-2.0\"\ndescription = \"test\"\nrepo = \"{REPO}\"\n\
             revision = \"{REVISION}\"\n[[model.file]]\nname = \"{name}\"\nsize = {}\n\
             sha256 = \"{}\"\n\n",
            bytes.len(),
            sha(bytes)
        ));
    }
    Catalog::parse(&text).unwrap()
}

/// Serves `files` by file name with the Hugging Face layout. Each answer is a 200 with the
/// whole body, written in pieces with `pause` between them.
fn serve(files: HashMap<String, Vec<u8>>, pause: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let files = Arc::new(files);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let files = files.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).unwrap() == 0 || header.trim().is_empty() {
                        break;
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/");
                let name = path.rsplit('/').next().unwrap_or_default();
                let mut out = stream;
                let Some(body) = files.get(name) else {
                    let _ = out.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    return;
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = out.write_all(head.as_bytes());
                for piece in body.chunks(1024) {
                    if out.write_all(piece).is_err() {
                        return;
                    }
                    std::thread::sleep(pause);
                }
            });
        }
    });
    base
}

struct Served {
    core: Arc<Core>,
    socket: PathBuf,
    root: PathBuf,
    task: tokio::task::JoinHandle<std::io::Result<agentsi_core::server::HeldLock>>,
    _dir: TempDir,
}

impl Served {
    async fn start(catalog: Catalog, base: &str) -> Served {
        let dir = TempDir::new();
        let socket = dir.socket();
        let root = dir.0.join("models");
        let bound = bind(&socket).unwrap();
        let downloader = Downloader::with_base_url(base)
            .attempts(1)
            .retry_delay(Duration::ZERO);
        let core = Core::with_models(Models::new(catalog, Some(Store::new(&root)), downloader));
        let task = tokio::spawn(bound.serve(core.clone(), euid(), std::future::pending()));
        Served {
            core,
            socket: socket.path,
            root,
            task,
            _dir: dir,
        }
    }

    fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join("catalog").join(id)
    }

    async fn stop(self) {
        self.core.ask_stop();
        drop(self.task.await.unwrap().unwrap());
    }
}

/// A line-mode client that keeps the events that arrive between the replies.
struct Client {
    r: tokio::io::BufReader<OwnedReadHalf>,
    w: OwnedWriteHalf,
    events: VecDeque<Value>,
    id: u64,
}

impl Client {
    async fn connect(socket: &Path) -> Client {
        let (r, w) = UnixStream::connect(socket).await.unwrap().into_split();
        let mut c = Client {
            r: tokio::io::BufReader::new(r),
            w,
            events: VecDeque::new(),
            id: 0,
        };
        let reply = c
            .call("core.subscribe", json!({"topics": ["models"]}))
            .await;
        assert_eq!(reply["result"]["topics"], json!(["models"]));
        c
    }

    async fn read(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(10), self.r.read_line(&mut line))
            .await
            .expect("a message within 10 s")
            .unwrap();
        assert!(n > 0, "the core closed the connection");
        serde_json::from_str(&line).unwrap()
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let request = json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params});
        self.w
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        loop {
            let message = self.read().await;
            if message["id"] == json!(self.id) {
                return message;
            }
            self.events.push_back(message);
        }
    }

    /// The next event named `method`. Earlier events of other names are dropped.
    async fn event(&mut self, method: &str) -> Value {
        loop {
            let message = match self.events.pop_front() {
                Some(m) => m,
                None => self.read().await,
            };
            if message["method"] == method {
                return message["params"].clone();
            }
            assert_ne!(
                message["method"], "models.error",
                "unexpected failure: {message}"
            );
        }
    }

    async fn model(&mut self, id: &str) -> Value {
        let list = self.call("models.list", json!({})).await;
        list["result"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .cloned()
            .unwrap()
    }
}

/// Voice 4: a file with a wrong hash is deleted, the core sends the error, and the list
/// keeps the error for the settings screen.
#[tokio::test]
async fn a_file_with_a_wrong_hash_is_deleted_and_the_error_reaches_the_client_and_the_list() {
    let pinned = b"the pinned model bytes".repeat(100);
    let mut wrong = pinned.clone();
    wrong[7] ^= 0xff;
    let base = serve(
        HashMap::from([("tiny.onnx".to_string(), wrong)]),
        Duration::ZERO,
    );
    let served = Served::start(catalog(&[("tiny", "tiny.onnx", &pinned)]), &base).await;
    let mut c = Client::connect(&served.socket).await;

    let reply = c.call("models.download", json!({"id": "tiny"})).await;
    assert_eq!(reply["result"], json!({"id": "tiny", "started": true}));
    let error = c.event("models.error").await;
    assert_eq!(error["id"], "tiny");
    assert_eq!(error["error"], "hash_mismatch");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("was deleted"), "{message}");

    let dir = served.model_dir("tiny");
    assert!(!dir.join("tiny.onnx").exists());
    assert!(!dir.join("tiny.onnx.partial").exists());
    let model = c.model("tiny").await;
    assert_eq!(model["state"], "error");
    assert_eq!(model["error"], "hash_mismatch");
    assert_eq!(model["message"], message);
    served.stop().await;
}

#[tokio::test]
async fn a_download_sends_progress_then_done_and_the_model_is_downloaded() {
    let pinned = vec![42u8; 64 * 1024];
    let base = serve(
        HashMap::from([("tiny.onnx".to_string(), pinned.clone())]),
        Duration::ZERO,
    );
    let served = Served::start(catalog(&[("tiny", "tiny.onnx", &pinned)]), &base).await;
    let mut c = Client::connect(&served.socket).await;
    assert_eq!(c.model("tiny").await["state"], "missing");

    c.call("models.download", json!({"id": "tiny"})).await;
    let progress = c.event("models.progress").await;
    assert_eq!(progress["total"], pinned.len());
    assert_eq!(c.event("models.done").await, json!({"id": "tiny"}));
    let model = c.model("tiny").await;
    assert_eq!(model["state"], "downloaded");
    assert_eq!(model["size"], pinned.len());
    assert_eq!(model["license"], "Apache-2.0");
    assert_eq!(
        std::fs::read(served.model_dir("tiny").join("tiny.onnx")).unwrap(),
        pinned
    );
    // A downloaded model starts nothing.
    let again = c.call("models.download", json!({"id": "tiny"})).await;
    assert_eq!(again["result"]["started"], false);
    served.stop().await;
}

#[tokio::test]
async fn a_foreign_file_with_a_catalog_name_gives_a_conflict_and_stays() {
    let pinned = b"pinned".repeat(50);
    let foreign = b"theirs".repeat(50);
    let base = serve(
        HashMap::from([("tiny.onnx".to_string(), pinned.clone())]),
        Duration::ZERO,
    );
    let served = Served::start(catalog(&[("tiny", "tiny.onnx", &pinned)]), &base).await;
    let dir = served.model_dir("tiny");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tiny.onnx"), &foreign).unwrap();
    let mut c = Client::connect(&served.socket).await;

    c.call("models.download", json!({"id": "tiny"})).await;
    let error = c.event("models.error").await;
    assert_eq!(error["error"], "conflict");
    assert_eq!(error["file"], "tiny.onnx");
    assert_eq!(std::fs::read(dir.join("tiny.onnx")).unwrap(), foreign);
    let model = c.model("tiny").await;
    assert_eq!(model["state"], "conflict");
    assert_eq!(model["file"], "tiny.onnx");

    // A delete leaves the foreign file, and the list no longer holds the failure.
    let deleted = c.call("models.delete", json!({"id": "tiny"})).await;
    assert_eq!(deleted["result"]["state"], "partial");
    assert_eq!(std::fs::read(dir.join("tiny.onnx")).unwrap(), foreign);
    served.stop().await;
}

#[tokio::test]
async fn a_catalog_file_of_another_size_is_a_conflict_in_the_list_without_a_download() {
    let pinned = b"pinned".repeat(50);
    let base = serve(HashMap::new(), Duration::ZERO);
    let served = Served::start(catalog(&[("tiny", "tiny.onnx", &pinned)]), &base).await;
    let dir = served.model_dir("tiny");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tiny.onnx"), b"short").unwrap();
    let mut c = Client::connect(&served.socket).await;
    let model = c.model("tiny").await;
    assert_eq!(model["state"], "conflict");
    assert_eq!(model["file"], "tiny.onnx");
    served.stop().await;
}

#[tokio::test]
async fn a_delete_removes_only_the_files_that_the_download_wrote() {
    let pinned = vec![7u8; 4096];
    let base = serve(
        HashMap::from([("tiny.onnx".to_string(), pinned.clone())]),
        Duration::ZERO,
    );
    let served = Served::start(catalog(&[("tiny", "tiny.onnx", &pinned)]), &base).await;
    // The folders of the Python voice code, and a file of the owner in the model folder.
    let python = served.root.join("kokoro");
    std::fs::create_dir_all(&python).unwrap();
    std::fs::write(python.join("kokoro-v1.0.onnx"), b"python model").unwrap();
    let mut c = Client::connect(&served.socket).await;
    c.call("models.download", json!({"id": "tiny"})).await;
    c.event("models.done").await;
    let dir = served.model_dir("tiny");
    std::fs::write(dir.join("notes.txt"), b"mine").unwrap();

    let deleted = c.call("models.delete", json!({"id": "tiny"})).await;
    assert_eq!(deleted["result"]["state"], "missing", "{deleted}");
    assert!(!dir.join("tiny.onnx").exists());
    assert!(!dir.join("receipt.json").exists());
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"mine");
    assert_eq!(
        std::fs::read(python.join("kokoro-v1.0.onnx")).unwrap(),
        b"python model"
    );
    served.stop().await;
}

#[tokio::test]
async fn a_running_download_is_not_started_twice_and_cannot_be_deleted() {
    let pinned = vec![1u8; 64 * 1024];
    // 64 pieces of 1 KiB, 20 ms apart: about 1.3 s.
    let base = serve(
        HashMap::from([("tiny.onnx".to_string(), pinned.clone())]),
        Duration::from_millis(20),
    );
    let served = Served::start(catalog(&[("tiny", "tiny.onnx", &pinned)]), &base).await;
    let mut c = Client::connect(&served.socket).await;
    let first = c.call("models.download", json!({"id": "tiny"})).await;
    assert_eq!(first["result"]["started"], true);
    let second = c.call("models.download", json!({"id": "tiny"})).await;
    assert_eq!(second["result"]["started"], false);
    assert_eq!(c.model("tiny").await["state"], "downloading");
    let deleted = c.call("models.delete", json!({"id": "tiny"})).await;
    assert_eq!(deleted["error"]["data"]["kind"], "busy");

    // The core sends at most 10 progress events each second.
    let started = std::time::Instant::now();
    let mut progress = 0;
    loop {
        let m = match c.events.pop_front() {
            Some(m) => m,
            None => c.read().await,
        };
        match m["method"].as_str() {
            Some("models.progress") => progress += 1,
            Some("models.done") => break,
            other => panic!("unexpected {other:?}: {m}"),
        }
    }
    let allowed = (started.elapsed().as_millis() / 100) as usize + 3;
    assert!(progress <= allowed, "{progress} progress events");
    served.stop().await;
}

#[tokio::test]
async fn an_unknown_model_is_not_found() {
    let base = serve(HashMap::new(), Duration::ZERO);
    let served = Served::start(catalog(&[("tiny", "tiny.onnx", b"x")]), &base).await;
    let mut c = Client::connect(&served.socket).await;
    for method in ["models.download", "models.delete"] {
        let reply = c.call(method, json!({"id": "nope"})).await;
        assert_eq!(reply["error"]["data"]["kind"], "not_found", "{method}");
    }
    served.stop().await;
}
