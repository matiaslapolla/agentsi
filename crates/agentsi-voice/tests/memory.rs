//! What each voice model costs in resident memory, and what a drop gives back.
//!
//! It loads the real models, so it is ignored. Run it alone, so no other test shares the
//! process:
//! `cargo test -p agentsi-voice --test memory -- --ignored --nocapture --test-threads=1`.

use agentsi_voice::{stt, tts, vad};

/// `VmRSS` of `/proc/self/status`, in MB.
fn rss_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").expect("Linux has /proc");
    let kb: f64 = status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .expect("a VmRSS line");
    kb / 1024.0
}

fn step(what: &str, before: &mut f64) {
    let now = rss_mb();
    eprintln!("{what:<40} {now:>8.1} MB ({:+.1})", now - *before);
    *before = now;
}

#[test]
#[ignore = "loads the real models: run it by hand"]
fn a_dropped_model_gives_its_memory_back() {
    let (Some(stt_dir), Some(vad_dir)) = (stt::model_dir(), vad::model_dir()) else {
        eprintln!("skip: HOME is not set");
        return;
    };
    let mut rss = rss_mb();
    step("start", &mut rss);
    for round in 1..=2 {
        let transcriber = stt::Transcriber::open(&stt_dir).expect("the speech model loads");
        step(&format!("{round}: speech model open"), &mut rss);
        let detector = vad::Silero::open(&vad_dir).expect("the voice detector loads");
        step(&format!("{round}: voice detector open"), &mut rss);
        drop(detector);
        drop(transcriber);
        step(&format!("{round}: speech model dropped"), &mut rss);
        agentsi_voice::release_memory();
        step(&format!("{round}: memory released"), &mut rss);
        let voice = tts::Voice::open(tts::Options::standard()).expect("the voice loads");
        step(&format!("{round}: voice open"), &mut rss);
        drop(voice);
        step(&format!("{round}: voice dropped"), &mut rss);
        agentsi_voice::release_memory();
        step(&format!("{round}: memory released"), &mut rss);
    }
}
