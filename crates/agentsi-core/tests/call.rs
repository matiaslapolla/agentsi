//! `call.*` on a real core with a throwaway data root: an old call from the Python package,
//! the listened mark and the render. The render with the real Kokoro voice runs only when
//! the model and `espeak-ng` exist, and it writes files. It plays no sound.

mod common;

use agentsi_proto::call_api::decode_pcm;
use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const REGISTRY: &str = r#"{"owner":{"name":"Ada","language":"English"},"products":[
  {"id":"alpha","name":"Alpha","paths":["/work/alpha"],"status":"active","roles":["pm","dev","qa"]}]}"#;

const SCRIPT: &str = r#"{"id":"m1","product":"alpha","question":"Does the call play?","status":"valid",
  "script":[{"role":"pm","text":"Welcome to the test meeting. Today we check the call."},
            {"role":"dev","text":"The build is green, and the parser bug is fixed."}]}"#;

fn old_call() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../agentsi-agency/tests/fixtures/old-call")
}

fn setup(dir: &TempDir, with_old_audio: bool) -> PathBuf {
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    conn.execute(
        "insert into minutes (id, product, question, status, created_at, data)
         values ('m1', 'alpha', 'Does the call play?', 'valid', '2026-10-01T10:00:00.000Z', ?1)",
        [SCRIPT.replace('\n', "")],
    )
    .unwrap();
    if with_old_audio {
        let audio = root.join("workspace/alpha/audio/m1");
        std::fs::create_dir_all(&audio).unwrap();
        for name in ["call.webm", "timings.json"] {
            std::fs::copy(old_call().join(name), audio.join(name)).unwrap();
        }
    }
    root
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

#[test]
fn an_old_call_plays_through_the_core_and_the_mark_is_stored() {
    let dir = TempDir::new();
    let home = setup(&dir, true);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let got = c.ok("call.get", json!({"product": "alpha", "minutes_id": "m1"}));
    assert_eq!(got["question"], "Does the call play?");
    assert_eq!(got["timed"], true);
    assert_eq!(got["audio"], json!({"format": "webm", "bytes": 37995}));
    assert_eq!(got["segments"].as_array().unwrap().len(), 3);
    assert_eq!(got["segments"][2]["role"], "qa");
    assert_eq!(
        (got["heard"].clone(), got.get("render").cloned()),
        (json!(false), None)
    );

    let chunk = c.ok(
        "call.audio",
        json!({"product": "alpha", "minutes_id": "m1", "from": 24_000, "count": 48_000}),
    );
    let samples = decode_pcm(chunk["pcm"].as_str().unwrap()).unwrap();
    assert_eq!(
        (chunk["from"].clone(), samples.len()),
        (json!(24_000), 48_000)
    );
    assert!(chunk["total"].as_u64().unwrap() > 224_000);
    assert!(
        samples.iter().any(|s| s.abs() > 1_000),
        "the old call is silent"
    );

    assert_eq!(
        c.ok(
            "call.heard",
            json!({"product": "alpha", "minutes_id": "m1", "heard": true})
        ),
        json!({"changed": true})
    );
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "alpha", "kind": "call"})
    );
    assert_eq!(
        c.ok(
            "call.heard",
            json!({"product": "alpha", "minutes_id": "m1", "heard": true})
        ),
        json!({"changed": false})
    );
    assert_eq!(
        c.ok("call.get", json!({"product": "alpha", "minutes_id": "m1"}))["heard"],
        true
    );
    assert_eq!(
        c.ok(
            "call.heard",
            json!({"product": "alpha", "minutes_id": "m1", "heard": false})
        ),
        json!({"changed": true})
    );
    assert_eq!(
        c.ok("call.get", json!({"product": "alpha", "minutes_id": "m1"}))["heard"],
        false
    );

    assert_eq!(
        c.ok("call.cancel", json!({"render": "none"})),
        json!({"canceled": false})
    );
}

#[test]
fn a_call_with_no_audio_shows_its_script_and_cannot_be_marked() {
    let dir = TempDir::new();
    let home = setup(&dir, false);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);

    let got = c.ok("call.get", json!({"product": "alpha", "minutes_id": "m1"}));
    assert_eq!(
        (got["timed"].clone(), got.get("audio").cloned()),
        (json!(false), None)
    );
    assert_eq!(
        got["segments"][1],
        json!({"idx": 1, "role": "dev", "text": "The build is green, and the parser bug is fixed.", "start": 0.0, "end": 0.0})
    );
    assert_eq!(
        c.ok(
            "call.heard",
            json!({"product": "alpha", "minutes_id": "m1", "heard": true})
        ),
        json!({"changed": false})
    );
    let r = c.call(
        "call.audio",
        json!({"product": "alpha", "minutes_id": "m1", "from": 0, "count": 10}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
    for (product, id) in [("alpha", "nope"), ("beta", "m1"), ("alpha", "../m1")] {
        let r = c.call("call.get", json!({"product": product, "minutes_id": id}));
        assert_eq!(kind(&r), "not_found", "{product} {id}: {r}");
    }
}

#[test]
fn a_render_without_the_voice_model_fails_with_a_hint_and_writes_nothing() {
    let dir = TempDir::new();
    let home = setup(&dir, true);
    let empty = dir.0.join("no-models");
    std::fs::create_dir_all(&empty).unwrap();
    let core = CoreProc::start_with(
        &dir,
        &[
            ("AGENTSI_HOME", home.as_path()),
            ("XDG_DATA_HOME", empty.as_path()),
        ],
    );
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["meeting"]}));
    let r = c.call(
        "call.render",
        json!({"product": "alpha", "minutes_id": "m1", "render": "r1"}),
    );
    assert_eq!(kind(&r), "unavailable", "{r}");
    let done = c.next_event("meeting.render");
    assert_eq!(
        (done["render"].clone(), done["stage"].clone()),
        (json!("r1"), json!("done"))
    );
    assert!(done["error"].as_str().is_some(), "{done}");
    let audio = home.join("workspace/alpha/audio/m1");
    let mut files: Vec<String> = std::fs::read_dir(audio)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(files, ["call.webm", "timings.json"], "the old call stays");
    let r = c.call(
        "call.render",
        json!({"product": "alpha", "minutes_id": "nope"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
}

fn voice_is_ready() -> bool {
    let model = agentsi_models::default_root()
        .map(|r| r.join("catalog/kokoro/kokoro-v1.0.onnx"))
        .is_some_and(|p| p.exists());
    let espeak = std::process::Command::new("espeak-ng")
        .arg("--version")
        .output()
        .is_ok();
    if !(model && espeak) {
        eprintln!("skipped: the Kokoro model or espeak-ng is missing");
    }
    model && espeak
}

#[test]
fn a_render_with_kokoro_writes_a_new_call_that_the_screen_data_reads_back() {
    if !voice_is_ready() {
        return;
    }
    let dir = TempDir::new();
    let home = setup(&dir, false);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["meeting", "data"]}));
    let done = c.ok(
        "call.render",
        json!({"product": "alpha", "minutes_id": "m1", "render": "r1"}),
    );
    assert_eq!(done["segments"], 2);
    assert!(done["duration"].as_f64().unwrap() > 4.0, "{done}");
    let mut stages = Vec::new();
    loop {
        let e = c.next_event("meeting.render");
        stages.push(format!(
            "{} {}/{}",
            e["stage"].as_str().unwrap(),
            e["done"],
            e["of"]
        ));
        if e["stage"] == "done" {
            break;
        }
    }
    assert_eq!(
        stages,
        [
            "start 0/2",
            "segment 1/2",
            "segment 2/2",
            "encode 2/2",
            "done 2/2"
        ]
    );

    let got = c.ok("call.get", json!({"product": "alpha", "minutes_id": "m1"}));
    assert_eq!(got["timed"], true);
    assert_eq!(got["audio"]["format"], "opus");
    let end = got["segments"][1]["end"].as_f64().unwrap();
    assert!((end - done["duration"].as_f64().unwrap()).abs() < 0.01);
    let chunk = c.ok(
        "call.audio",
        json!({"product": "alpha", "minutes_id": "m1", "from": 0, "count": 10_000_000}),
    );
    let samples = decode_pcm(chunk["pcm"].as_str().unwrap()).unwrap();
    assert_eq!(samples.len() as u64, chunk["total"].as_u64().unwrap());
    assert!((samples.len() as f64 / 24_000.0 - end).abs() < 0.01);
    // The second segment starts after a pause: the samples just before its start are silent.
    let start = (got["segments"][1]["start"].as_f64().unwrap() * 24_000.0) as usize;
    assert!(
        samples[start - 4_000..start - 500]
            .iter()
            .all(|s| s.abs() < 300)
    );
    assert!(
        samples[start + 4_000..start + 12_000]
            .iter()
            .any(|s| s.abs() > 1_000)
    );
}

/// The resident memory of a process, in MB.
fn rss_mb(pid: u32) -> f64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    line.split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<f64>()
        .unwrap()
        / 1024.0
}

#[test]
fn a_core_gives_back_the_memory_of_the_voice_when_a_render_ends() {
    if !voice_is_ready() {
        return;
    }
    let dir = TempDir::new();
    let home = setup(&dir, false);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let pid = core.child.as_ref().unwrap().id();
    let mut c = Framed::connect(&core.socket.path);
    let before = rss_mb(pid);
    c.ok(
        "call.render",
        json!({"product": "alpha", "minutes_id": "m1", "render": "r1"}),
    );
    // Kokoro takes about 500 MB while it renders. A core with a call behind it idles like one without.
    let after = rss_mb(pid);
    assert!(
        after < before + 100.0,
        "the core holds {after:.0} MB after a render, {before:.0} MB before"
    );
}

#[test]
fn a_cancel_stops_a_render_and_writes_nothing() {
    if !voice_is_ready() {
        return;
    }
    let dir = TempDir::new();
    let home = setup(&dir, true);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", home.as_path())]);
    let mut watcher = Framed::connect(&core.socket.path);
    watcher.ok("core.subscribe", json!({"topics": ["meeting"]}));
    let socket = core.socket.path.clone();
    let renderer = std::thread::spawn(move || {
        let mut c = Framed::connect(&socket);
        c.call(
            "call.render",
            json!({"product": "alpha", "minutes_id": "m1", "render": "r1"}),
        )
    });
    // The first segment is ready: the render has begun and the second one has not.
    loop {
        let e = watcher.next_event("meeting.render");
        if e["stage"] == "segment" {
            break;
        }
    }
    assert_eq!(
        watcher.ok("call.cancel", json!({"render": "r1"})),
        json!({"canceled": true})
    );
    let reply = renderer.join().unwrap();
    assert_eq!(kind(&reply), "conflict", "{reply}");
    let audio = home.join("workspace/alpha/audio/m1");
    let mut files: Vec<String> = std::fs::read_dir(audio)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        ["call.webm", "timings.json"],
        "the old call stays and nothing new appears"
    );
    let got = watcher.ok("call.get", json!({"product": "alpha", "minutes_id": "m1"}));
    assert!(got.get("render").is_none(), "{got}");
}
