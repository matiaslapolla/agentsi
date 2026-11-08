//! Measurements: PTY-read-to-paint latency, build and paint time per frame, CPU and memory.
use std::{fs::OpenOptions, io::Write, time::Instant};

pub struct Stats {
    path: Option<String>,
    start: Instant,
    last: Instant,
    last_ticks: u64,
    pub frames: u64,
    pub bytes: u64,
    pub latency_ms: Vec<f32>,
    pub build_ms: Vec<f32>,
    pub paint_ms: Vec<f32>,
    cpu: Vec<(f32, f32, f32)>, // t, cpu%, rss MB
}

fn ticks() -> u64 {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let rest = s.rsplit_once(')').map(|x| x.1).unwrap_or("");
    let f: Vec<&str> = rest.split_whitespace().collect();
    // After the comm field: state is f[0], utime f[11], stime f[12].
    f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
}

fn rss_mb() -> f32 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0) / 1024.0
}

fn pct(v: &mut [f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f32 * p).round() as usize]
}

impl Stats {
    pub fn new(path: Option<String>) -> Self {
        Stats { path, start: Instant::now(), last: Instant::now(), last_ticks: ticks(), frames: 0, bytes: 0, latency_ms: vec![], build_ms: vec![], paint_ms: vec![], cpu: vec![] }
    }

    fn line(&self, s: &str) {
        if let Some(p) = &self.path
            && let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p)
        {
            let _ = writeln!(f, "{s}");
        }
    }

    pub fn note(&mut self, s: &str) {
        self.line(&format!("[{:6.2}s] {s}", self.start.elapsed().as_secs_f32()));
    }

    pub fn event(&mut self, s: &str) {
        self.note(&format!("event {s}"));
    }

    /// Once a second: CPU use of this process (the child process is not included) and resident memory.
    pub fn sample(&mut self) {
        let now = Instant::now();
        let t = ticks();
        let dt = now.duration_since(self.last).as_secs_f32();
        let cpu = (t - self.last_ticks) as f32 / 100.0 / dt * 100.0;
        let (rss, at) = (rss_mb(), self.start.elapsed().as_secs_f32());
        self.cpu.push((at, cpu, rss));
        self.line(&format!("[{at:6.2}s] sample cpu={cpu:.1}% rss={rss:.0}MB frames={} bytes={}", self.frames, self.bytes));
        self.last = now;
        self.last_ticks = t;
    }

    pub fn finish(&mut self) {
        let (mut l, mut b, mut p) = (self.latency_ms.clone(), self.build_ms.clone(), self.paint_ms.clone());
        let n = l.len();
        self.line(&format!(
            "summary frames={} latency_samples={n} latency_ms p50={:.2} p95={:.2} max={:.2} | build_ms p50={:.2} p95={:.2} max={:.2} | paint_ms p50={:.2} p95={:.2} max={:.2}",
            self.frames, pct(&mut l, 0.5), pct(&mut l, 0.95), pct(&mut l, 1.0), pct(&mut b, 0.5), pct(&mut b, 0.95), pct(&mut b, 1.0), pct(&mut p, 0.5), pct(&mut p, 0.95), pct(&mut p, 1.0)
        ));
        if !self.cpu.is_empty() {
            let avg = self.cpu.iter().map(|c| c.1).sum::<f32>() / self.cpu.len() as f32;
            let rss = self.cpu.iter().map(|c| c.2).fold(0.0, f32::max);
            self.line(&format!("summary cpu_avg={avg:.1}% rss_max={rss:.0}MB over {} samples", self.cpu.len()));
        }
    }
}
