//! The risk of U26: an old call that no longer plays. `fixtures/old-call/` holds a real
//! `call.webm` and `timings.json` that the Python package wrote on synthetic text. The
//! call code must read them as it reads a new call.

use agentsi_agency::call::{self, Format, codec};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

const PRODUCT: &str = "demo";
const ID: &str = "20261020-1000-a";

/// The RMS of each window of 100 ms (x1000) of `ffmpeg -c:a libopus -ar 24000` on this file.
const REFERENCE: [i32; 93] = [
    63, 179, 76, 104, 114, 48, 66, 54, 70, 37, 114, 81, 36, 17, 1, 0, 0, 27, 114, 163, 112, 109,
    42, 91, 6, 50, 66, 52, 33, 2, 0, 0, 0, 0, 79, 118, 134, 91, 55, 36, 120, 96, 58, 20, 30, 109,
    65, 109, 79, 90, 55, 117, 51, 39, 10, 107, 9, 26, 2, 0, 0, 0, 0, 69, 122, 129, 87, 119, 115,
    52, 97, 20, 11, 74, 97, 34, 14, 7, 0, 0, 0, 0, 129, 147, 57, 144, 108, 18, 104, 64, 20, 12, 1,
];

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/old-call")
        .join(name)
}

/// A data root with the old call under `workspace/demo/audio/<id>/`, and its minutes in the store.
fn root_with_old_call() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let audio = call::audio_dir(dir.path(), PRODUCT, ID);
    std::fs::create_dir_all(&audio).unwrap();
    for name in ["call.webm", "timings.json"] {
        std::fs::copy(fixture(name), audio.join(name)).unwrap();
    }
    let c = agentsi_store::db::open_writer(&dir.path().join("agentsi.db")).unwrap();
    let data = serde_json::json!({
        "id": ID, "product": PRODUCT, "question": "Does the old call play?", "status": "valid",
        "script": [{"role": "pm", "text": "Welcome to the test meeting. Today we check the call."}]
    });
    c.execute(
        "insert into minutes (id, product, question, status, created_at, data)
         values (?1, ?2, 'Does the old call play?', 'valid', '2026-10-20T10:00:00.000Z', ?3)",
        rusqlite::params![ID, PRODUCT, data.to_string()],
    )
    .unwrap();
    (dir, c)
}

fn windows(samples: &[f32]) -> Vec<i32> {
    samples
        .as_chunks::<2_400>()
        .0
        .iter()
        .map(|w| {
            let mean = w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32;
            (mean.sqrt() * 1000.0).round() as i32
        })
        .collect()
}

#[test]
fn the_call_screen_data_of_an_old_call_has_its_segments_and_its_audio() {
    let (dir, c) = root_with_old_call();
    let view = call::get(&c, dir.path(), PRODUCT, ID).unwrap();
    assert!(view.timed);
    assert_eq!(view.question, "Does the old call play?");
    let roles: Vec<&str> = view.segments.iter().map(|s| s.role.as_str()).collect();
    assert_eq!(roles, ["pm", "dev", "qa"]);
    assert_eq!(
        (view.segments[1].start, view.segments[1].end),
        (3.409, 5.926)
    );
    let audio = view.audio.unwrap();
    assert_eq!((audio.format.as_str(), audio.bytes), ("webm", 37_995));
    assert!(!view.heard);
}

#[test]
fn an_old_webm_decodes_to_what_ffmpeg_decodes() {
    let samples = codec::decode(&fixture("call.webm")).unwrap();
    // 9.356 s of speech, and at most the 20 ms frame of padding that ffmpeg trims.
    let seconds = samples.len() as f64 / f64::from(codec::RATE);
    assert!((9.356..9.40).contains(&seconds), "{seconds}");
    let got = windows(&samples);
    assert_eq!(got.len(), REFERENCE.len());
    for (i, (g, r)) in got.iter().zip(REFERENCE).enumerate() {
        assert!((g - r).abs() <= 2, "window {i}: {g} against {r}");
    }
}

#[test]
fn the_pauses_of_timings_json_fall_on_the_silence_of_the_audio() {
    let samples = codec::decode(&fixture("call.webm")).unwrap();
    let loud = |from: f64, to: f64| {
        let a = (from * f64::from(codec::RATE)) as usize;
        let b = (to * f64::from(codec::RATE)) as usize;
        let w = &samples[a..b];
        (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt()
    };
    let segments = call::read_timings(fixture("timings.json").parent().unwrap()).unwrap();
    for s in &segments {
        assert!(
            loud(s.start + 0.2, s.end - 0.3) > 0.03,
            "segment {} is silent",
            s.idx
        );
    }
    for pair in segments.windows(2) {
        // The pause between two segments is 0.35 s long.
        assert!(
            loud(pair[0].end + 0.05, pair[1].start - 0.05) < 0.005,
            "no pause after {}",
            pair[0].idx
        );
    }
}

#[test]
fn the_audio_of_an_old_call_reaches_the_screen_in_chunks() {
    let (dir, c) = root_with_old_call();
    let (first, total) = call::audio_chunk(&c, dir.path(), PRODUCT, ID, 0, 48_000).unwrap();
    assert_eq!(first.len(), 48_000);
    assert!(total > 224_000);
    let (rest, same) = call::audio_chunk(&c, dir.path(), PRODUCT, ID, 48_000, 1 << 40).unwrap();
    assert_eq!((same, rest.len() as u64), (total, total - 48_000));
    let (none, _) = call::audio_chunk(&c, dir.path(), PRODUCT, ID, total + 10, 100).unwrap();
    assert!(none.is_empty());
    assert!(first.iter().any(|s| s.abs() > 1_000));
}

#[test]
fn an_old_call_can_be_marked_as_listened_and_the_mark_removed() {
    let (dir, c) = root_with_old_call();
    assert!(call::set_heard(&c, dir.path(), PRODUCT, ID, true).unwrap());
    assert!(
        !call::set_heard(&c, dir.path(), PRODUCT, ID, true).unwrap(),
        "a second mark changes nothing"
    );
    assert!(call::get(&c, dir.path(), PRODUCT, ID).unwrap().heard);
    assert!(call::set_heard(&c, dir.path(), PRODUCT, ID, false).unwrap());
    assert!(!call::get(&c, dir.path(), PRODUCT, ID).unwrap().heard);
}

#[test]
fn a_new_render_of_an_old_call_wins_and_the_old_file_stays() {
    let (dir, c) = root_with_old_call();
    let out = call::audio_dir(dir.path(), PRODUCT, ID);
    std::fs::write(
        out.join("call.opus"),
        codec::encode_ogg_opus(&vec![0.1; 24_000]).unwrap(),
    )
    .unwrap();
    let audio = call::get(&c, dir.path(), PRODUCT, ID)
        .unwrap()
        .audio
        .unwrap();
    assert_eq!(audio.format, "opus");
    assert!(out.join("call.webm").exists());
    assert_eq!(call::find_audio(&out).unwrap().format, Format::Opus);
}
