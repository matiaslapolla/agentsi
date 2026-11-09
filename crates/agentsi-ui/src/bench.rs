//! Bench mode: `--bench <idle|scroll|terminal|start|attention|alert> --seconds <n> --json`.
//!
//! A view puts `Bench::marker` in its root and calls `Bench::start` in its context.
//! The marker records the time of each paint and each input event. After a warm-up,
//! the bench measures for `seconds`, prints one report and quits the app. The timers
//! of the bench draw no frames.
//!
//! `--bench start --runs <n>` starts the program `n` times as new processes and
//! reports the time from each start to its first paint.
use std::cell::RefCell;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::{
    Context, DispatchPhase, IntoElement, KeyDownEvent, MouseDownEvent, MouseMoveEvent, Pixels,
    ScrollEvent, Styled, Subscription, Window, canvas, px,
};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchKind {
    /// Count the frames with no input and no animation.
    Idle,
    /// Scroll a long list and take the intervals between frames.
    Scroll,
    /// Take the time from a PTY read to the paint. The terminal pane records each sample.
    Terminal,
    /// Take the time from the start of the process to the first paint.
    Start,
    /// Take the time from a `blocked` state that the bench asks for to the paint of the
    /// attention marker. The marker records each sample.
    Attention,
    /// Ask for a `blocked` state in a pane that is not in view, wait for the glow to end, and
    /// count the frames of an idle window.
    Alert,
}

impl BenchKind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "idle" => BenchKind::Idle,
            "scroll" => BenchKind::Scroll,
            "terminal" => BenchKind::Terminal,
            "start" => BenchKind::Start,
            "attention" => BenchKind::Attention,
            "alert" => BenchKind::Alert,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            BenchKind::Idle => "idle",
            BenchKind::Scroll => "scroll",
            BenchKind::Terminal => "terminal",
            BenchKind::Start => "start",
            BenchKind::Attention => "attention",
            BenchKind::Alert => "alert",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BenchArgs {
    pub kind: BenchKind,
    pub seconds: f64,
    pub warmup: Duration,
    pub json: bool,
    /// Scroll speed in px per second.
    pub speed: f32,
    /// For `start`: the number of new processes to time.
    pub runs: u32,
    /// The window must stay without focus for the whole measurement.
    pub unfocused: bool,
}

pub const USAGE: &str = "usage: --bench <idle|scroll|terminal|start|attention|alert> [--seconds <0.1..3600>] \
[--warmup-ms <0..600000>] [--speed <px/s>] [--runs <1..100>] [--unfocused] [--json]";

const SECONDS_MAX: f64 = 3600.;
const WARMUP_MS_MAX: u64 = 600_000;
const RUNS_MAX: u32 = 100;
/// A start run that has not painted after this time fails.
const START_TIMEOUT: Duration = Duration::from_secs(10);
/// The pause between two start runs. Without it, the compositor still closes the window
/// of the last run while the next run waits for its first configure.
const START_GAP: Duration = Duration::from_millis(500);
/// The pass lines of `docs/specs/agentsi-desktop-ui.md`.
const SCROLL_P95_FACTOR: f64 = 1.2;
const TERMINAL_P95_MS: f64 = 16.;
const START_P95_MS: f64 = 400.;
const ATTENTION_P95_MS: f64 = 100.;
/// The alert bench waits this long after the `blocked` state before it counts frames.
/// The glow ends after 0.18 s + 3 pulses of 1.2 s.
const ALERT_WAIT: Duration = Duration::from_secs(10);
/// The attention line of the spec is a p95 over 20 runs: one sample per run.
pub const ATTENTION_SAMPLES: usize = 20;

fn number<T: std::str::FromStr>(name: &str, v: &str) -> Result<T, String> {
    v.parse()
        .map_err(|_| format!("{name}: `{v}` is not a number. {USAGE}"))
}

impl BenchArgs {
    /// Reads the bench flags. `Ok(None)` when there is no `--bench`.
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Option<Self>, String> {
        let mut kind = None;
        let mut warmup_given = false;
        let mut out = BenchArgs {
            kind: BenchKind::Idle,
            seconds: 10.,
            warmup: Duration::from_millis(1000),
            json: false,
            speed: 4000.,
            runs: 1,
            unfocused: false,
        };
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            let mut value = |name: &str| it.next().ok_or(format!("{name} needs a value. {USAGE}"));
            match a.as_str() {
                "--bench" => {
                    let v = value("--bench")?;
                    kind =
                        Some(BenchKind::parse(&v).ok_or(format!("unknown bench `{v}`. {USAGE}"))?);
                }
                "--seconds" => {
                    let s: f64 = number("--seconds", &value("--seconds")?)?;
                    if !(s.is_finite() && (0.1..=SECONDS_MAX).contains(&s)) {
                        return Err(format!(
                            "--seconds must be from 0.1 to {SECONDS_MAX}. {USAGE}"
                        ));
                    }
                    out.seconds = s;
                }
                "--warmup-ms" => {
                    let ms: u64 = number("--warmup-ms", &value("--warmup-ms")?)?;
                    if ms > WARMUP_MS_MAX {
                        return Err(format!(
                            "--warmup-ms must be {WARMUP_MS_MAX} or less. {USAGE}"
                        ));
                    }
                    out.warmup = Duration::from_millis(ms);
                    warmup_given = true;
                }
                "--speed" => {
                    let v: f32 = number("--speed", &value("--speed")?)?;
                    if !(v.is_finite() && v > 0.) {
                        return Err(format!("--speed must be more than 0. {USAGE}"));
                    }
                    out.speed = v;
                }
                "--runs" => {
                    let n: u32 = number("--runs", &value("--runs")?)?;
                    if !(1..=RUNS_MAX).contains(&n) {
                        return Err(format!("--runs must be from 1 to {RUNS_MAX}. {USAGE}"));
                    }
                    out.runs = n;
                }
                "--unfocused" => out.unfocused = true,
                "--json" => out.json = true,
                _ => {}
            }
        }
        if kind == Some(BenchKind::Alert) && !warmup_given {
            out.warmup = ALERT_WAIT;
        }
        Ok(kind.map(|k| BenchArgs { kind: k, ..out }))
    }
}

struct State {
    args: BenchArgs,
    /// The start of `main`, as the caller gives it.
    main_start: Instant,
    paints: Vec<Instant>,
    first_paint_unix_ms: Option<f64>,
    latencies: Vec<(Instant, f64)>,
    window_from: Option<Instant>,
    cpu_from: Option<f64>,
    active: Vec<bool>,
    /// Each change of window focus. Focus is input: a run with a change is not valid.
    activations: Vec<Instant>,
    /// Each pointer, wheel or key event that reached the window.
    inputs: Vec<Instant>,
    refresh: Option<Duration>,
    _activation: Option<Subscription>,
}

/// The bench of one window. Clone it into the view.
#[derive(Clone)]
pub struct Bench(Rc<RefCell<State>>);

/// One finished measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub kind: BenchKind,
    pub seconds: f64,
    pub frames_before_window: usize,
    pub frames: usize,
    /// When there are 100 frames or less: the time of each frame, in ms after the start of `main`.
    pub frame_times_ms: Vec<f64>,
    pub fps: f64,
    pub interval_ms: Stats,
    pub latency_ms: Stats,
    pub refresh_ms: Option<f64>,
    pub first_paint_ms: Option<f64>,
    pub first_paint_unix_ms: Option<f64>,
    pub process_age_at_main_ms: Option<f64>,
    pub cpu_ms: Option<f64>,
    pub rss_mb: Option<f64>,
    pub window_active: bool,
    pub unfocused: bool,
    /// Changes of window focus during the measurement.
    pub focus_changes: usize,
    /// Pointer, wheel and key events during the measurement.
    pub input_events: usize,
    /// `false` when input reached the window, or when its focus was not the expected one.
    /// Then `pass` is `None`.
    pub valid: bool,
    pub pass: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub n: usize,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Stats {
    pub fn of(mut v: Vec<f64>) -> Stats {
        if v.is_empty() {
            return Stats::default();
        }
        v.sort_by(|a, b| a.total_cmp(b));
        let n = v.len();
        let q = |p: f64| v[(((n - 1) as f64) * p).round() as usize];
        Stats {
            n,
            mean: v.iter().sum::<f64>() / n as f64,
            p50: q(0.5),
            p95: q(0.95),
            p99: q(0.99),
            max: v[n - 1],
        }
    }

    fn json(&self) -> Value {
        json!({"n": self.n, "mean": self.mean, "p50": self.p50, "p95": self.p95, "p99": self.p99, "max": self.max})
    }
}

impl Bench {
    /// The bench of this process, from `std::env::args`. Exits with code 2 on a bad flag.
    /// With `--bench start --runs <n>` it times `n` new processes, prints the report and exits.
    pub fn from_env(main_start: Instant) -> Option<Bench> {
        match BenchArgs::parse(std::env::args().skip(1)) {
            Ok(Some(args)) if args.kind == BenchKind::Start && args.runs > 1 => {
                let (out, pass) = drive_starts(&args);
                println!("{out}");
                std::process::exit(if pass { 0 } else { 1 });
            }
            Ok(Some(args)) => Some(Bench::new(args, main_start)),
            Ok(None) => None,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        }
    }

    pub fn new(args: BenchArgs, main_start: Instant) -> Bench {
        Bench(Rc::new(RefCell::new(State {
            args,
            main_start,
            paints: Vec::new(),
            first_paint_unix_ms: None,
            latencies: Vec::new(),
            window_from: None,
            cpu_from: None,
            active: Vec::new(),
            activations: Vec::new(),
            inputs: Vec::new(),
            refresh: None,
            _activation: None,
        })))
    }

    pub fn kind(&self) -> BenchKind {
        self.0.borrow().args.kind
    }

    pub fn args(&self) -> BenchArgs {
        self.0.borrow().args.clone()
    }

    /// An empty element that records the time of each paint, and each input event.
    /// Put it in the root of the view. GPUI keeps the listeners of the last frame, so
    /// the marker also sees input while the window draws nothing.
    pub fn marker(&self) -> impl IntoElement + use<> {
        let b = self.clone();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                {
                    let mut s = b.0.borrow_mut();
                    s.paints.push(Instant::now());
                    if s.first_paint_unix_ms.is_none() {
                        s.first_paint_unix_ms = Some(unix_ms());
                    }
                }
                let input = |b: Bench| {
                    move |phase: DispatchPhase| {
                        if phase == DispatchPhase::Bubble {
                            b.0.borrow_mut().inputs.push(Instant::now());
                        }
                    }
                };
                let f = input(b.clone());
                window.on_mouse_event(move |_: &MouseMoveEvent, p, _, _| f(p));
                let f = input(b.clone());
                window.on_mouse_event(move |_: &MouseDownEvent, p, _, _| f(p));
                let f = input(b.clone());
                window.on_mouse_event(move |_: &ScrollEvent, p, _, _| f(p));
                let f = input(b.clone());
                window.on_key_event(move |_: &KeyDownEvent, p, _, _| f(p));
            },
        )
        .size_0()
    }

    /// Records one latency sample, for example from a PTY read to its paint.
    /// Only the samples of the measurement count.
    pub fn record_latency(&self, ms: f64) {
        self.0.borrow_mut().latencies.push((Instant::now(), ms));
    }

    /// The scroll offset for this frame, in the scroll bench. It goes down and up `range` px.
    /// The caller sets it and asks for the next frame.
    pub fn scroll_offset(&self, range: Pixels) -> Option<Pixels> {
        let s = self.0.borrow();
        if s.args.kind != BenchKind::Scroll {
            return None;
        }
        let range = f32::from(range).max(1.);
        let t = s.main_start.elapsed().as_secs_f32();
        let pos = (t * s.args.speed) % (2. * range);
        Some(px(if pos > range { 2. * range - pos } else { pos }))
    }

    /// Starts the timers of the bench in the view of `cx`. When the bench ends, it prints the report and quits.
    pub fn start<V: 'static>(&self, window: &mut Window, cx: &mut Context<V>) {
        let rec = self.clone();
        let sub = cx.observe_window_activation(window, move |_, _, _| {
            rec.0.borrow_mut().activations.push(Instant::now())
        });
        self.0.borrow_mut()._activation = Some(sub);
        let b = self.clone();
        let args = self.args();
        window
            .spawn(cx, async move |cx| {
                if args.kind == BenchKind::Start {
                    let deadline = Instant::now() + START_TIMEOUT;
                    while b.0.borrow().paints.is_empty() && Instant::now() < deadline {
                        cx.background_executor()
                            .timer(Duration::from_millis(5))
                            .await;
                    }
                } else {
                    cx.background_executor().timer(args.warmup).await;
                    _ = cx.update(|window, cx| {
                        let mut s = b.0.borrow_mut();
                        s.window_from = Some(Instant::now());
                        s.cpu_from = proc_stat().map(|p| p.cpu_ms);
                        s.active.push(window.is_window_active());
                        s.refresh = window.refresh_interval(cx);
                    });
                    cx.background_executor()
                        .timer(Duration::from_secs_f64(args.seconds))
                        .await;
                }
                _ = cx.update(|window, cx| {
                    b.0.borrow_mut().active.push(window.is_window_active());
                    let r = b.report();
                    println!("{}", if args.json { r.to_json() } else { r.to_text() });
                    cx.quit();
                });
            })
            .detach();
    }

    /// The report at this moment.
    pub fn report(&self) -> Report {
        let s = self.0.borrow();
        let now = Instant::now();
        let from = s.window_from.unwrap_or(now);
        let in_window: Vec<Instant> = s.paints.iter().copied().filter(|t| *t >= from).collect();
        let frames_before_window = s.paints.len() - in_window.len();
        let seconds = now.duration_since(from).as_secs_f64();
        let intervals = in_window
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.)
            .collect::<Vec<_>>();
        let interval_ms = Stats::of(intervals);
        let latency_ms = Stats::of(
            s.latencies
                .iter()
                .filter(|(t, _)| *t >= from)
                .map(|(_, ms)| *ms)
                .collect(),
        );
        let refresh_ms = s.refresh.map(|d| d.as_secs_f64() * 1000.);
        let first_paint_ms = s
            .paints
            .first()
            .map(|t| t.duration_since(s.main_start).as_secs_f64() * 1000.);
        let process_age_at_main_ms = process_age_ms()
            .map(|age| (age - s.main_start.elapsed().as_secs_f64() * 1000.).max(0.));
        let focus_changes = s.activations.iter().filter(|t| **t >= from).count();
        let input_events = s.inputs.iter().filter(|t| **t >= from).count();
        let focused = !s.active.is_empty() && s.active.iter().all(|a| *a);
        let unfocused = !s.active.is_empty() && s.active.iter().all(|a| !*a);
        let focus_ok = if s.args.unfocused { unfocused } else { focused };
        let valid = s.args.kind == BenchKind::Start
            || (focus_ok && focus_changes == 0 && input_events == 0);
        let frames = in_window.len();
        let pass = match s.args.kind {
            _ if !valid => None,
            BenchKind::Idle | BenchKind::Alert => Some(frames == 0),
            BenchKind::Scroll => {
                refresh_ms.map(|r| interval_ms.n > 0 && interval_ms.p95 <= SCROLL_P95_FACTOR * r)
            }
            BenchKind::Terminal => (latency_ms.n > 0).then_some(latency_ms.p95 <= TERMINAL_P95_MS),
            BenchKind::Attention => {
                (latency_ms.n >= ATTENTION_SAMPLES).then_some(latency_ms.p95 <= ATTENTION_P95_MS)
            }
            BenchKind::Start => {
                first_paint_ms.map(|f| f + process_age_at_main_ms.unwrap_or(0.) <= START_P95_MS)
            }
        };
        Report {
            kind: s.args.kind,
            seconds,
            frames_before_window,
            frames,
            frame_times_ms: if frames <= 100 {
                in_window
                    .iter()
                    .map(|t| t.duration_since(s.main_start).as_secs_f64() * 1000.)
                    .collect()
            } else {
                Vec::new()
            },
            fps: if seconds > 0. {
                frames as f64 / seconds
            } else {
                0.
            },
            interval_ms,
            latency_ms,
            refresh_ms,
            first_paint_ms,
            first_paint_unix_ms: s.first_paint_unix_ms,
            process_age_at_main_ms,
            cpu_ms: match (s.cpu_from, proc_stat()) {
                (Some(a), Some(b)) => Some(b.cpu_ms - a),
                _ => None,
            },
            rss_mb: process_rss_mb(),
            window_active: focused,
            unfocused: s.args.unfocused,
            focus_changes,
            input_events,
            valid,
            pass,
        }
    }
}

impl Report {
    pub fn to_json(&self) -> String {
        json!({
            "bench": self.kind.name(),
            "seconds": self.seconds,
            "frames_before_window": self.frames_before_window,
            "frames": self.frames,
            "frame_times_ms": self.frame_times_ms,
            "fps": self.fps,
            "interval_ms": self.interval_ms.json(),
            "latency_ms": self.latency_ms.json(),
            "refresh_ms": self.refresh_ms,
            "first_paint_ms": self.first_paint_ms,
            "first_paint_unix_ms": self.first_paint_unix_ms,
            "process_age_at_main_ms": self.process_age_at_main_ms,
            "cpu_ms": self.cpu_ms,
            "rss_mb": self.rss_mb,
            "window_active": self.window_active,
            "unfocused": self.unfocused,
            "focus_changes": self.focus_changes,
            "input_events": self.input_events,
            "valid": self.valid,
            "pass": self.pass,
        })
        .to_string()
    }

    pub fn to_text(&self) -> String {
        format!(
            "bench {}: {} frames in {:.1} s ({:.2} fps), {} before the window; interval p50 {:.2} ms, p95 {:.2} ms; refresh {} ms; first paint {} ms; focus changes {}; input events {}; valid {}; pass {:?}",
            self.kind.name(),
            self.frames,
            self.seconds,
            self.fps,
            self.frames_before_window,
            self.interval_ms.p50,
            self.interval_ms.p95,
            self.refresh_ms.map_or("?".into(), |r| format!("{r:.2}")),
            self.first_paint_ms
                .map_or("?".into(), |r| format!("{r:.1}")),
            self.focus_changes,
            self.input_events,
            self.valid,
            self.pass
        )
    }
}

/// Starts this program `args.runs` times with `--bench start --json`. Each run is a new
/// process. Returns the report and the verdict.
fn drive_starts(args: &BenchArgs) -> (String, bool) {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            return (
                json!({"bench": "start", "error": e.to_string()}).to_string(),
                false,
            );
        }
    };
    let mut samples = Vec::new();
    let mut failed_runs = Vec::new();
    for run in 0..args.runs {
        if run > 0 {
            std::thread::sleep(START_GAP);
        }
        let t0 = unix_ms();
        let out = Command::new(&exe)
            .args(["--bench", "start", "--json"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let paint = out.ok().and_then(|o| {
            let line = String::from_utf8_lossy(&o.stdout)
                .lines()
                .last()?
                .to_string();
            serde_json::from_str::<Value>(&line).ok()?["first_paint_unix_ms"].as_f64()
        });
        match paint {
            Some(p) => samples.push(p - t0),
            None => failed_runs.push(run),
        }
    }
    let stats = Stats::of(samples.clone());
    let pass = failed_runs.is_empty() && stats.n > 0 && stats.p95 <= START_P95_MS;
    let out = json!({
        "bench": "start",
        "runs": args.runs,
        "first_paint_ms": stats.json(),
        "samples_ms": samples,
        "failed_runs": failed_runs,
        "pass": pass,
    });
    (out.to_string(), pass)
}

fn unix_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0., |d| d.as_secs_f64() * 1000.)
}

/// The fields of `/proc/self/stat` that the bench reads (Linux).
struct ProcStat {
    /// utime + stime, in ms.
    cpu_ms: f64,
    /// The start of the process after boot, in ms.
    start_ms: f64,
}

fn proc_stat() -> Option<ProcStat> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Field 2, the command name, can hold spaces. The fields after its `)` start at field 3.
    let f: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let tick_ms = 1000. / sysconf(libc::_SC_CLK_TCK)?;
    let field = |n: usize| f.get(n - 3)?.parse::<f64>().ok();
    Some(ProcStat {
        cpu_ms: (field(14)? + field(15)?) * tick_ms,
        start_ms: field(22)? * tick_ms,
    })
}

/// The age of this process, in ms (Linux, one clock tick of resolution).
/// `/proc/uptime` and the start time in `/proc/self/stat` come from two kernel readings
/// that are each rounded to a tick (10 ms). A reading below 0 means they disagree, so the
/// age is 0 then.
fn process_age_ms() -> Option<f64> {
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((uptime * 1000. - proc_stat()?.start_ms).max(0.))
}

fn process_rss_mb() -> Option<f64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: f64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * sysconf(libc::_SC_PAGESIZE)? / (1024. * 1024.))
}

fn sysconf(name: libc::c_int) -> Option<f64> {
    // SAFETY: `sysconf` reads a system constant and has no preconditions.
    let v = unsafe { libc::sysconf(name) };
    (v > 0).then_some(v as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &str) -> Result<Option<BenchArgs>, String> {
        BenchArgs::parse(s.split_whitespace().map(String::from))
    }

    fn measuring(kind: &str) -> Bench {
        let b = Bench::new(
            args(&format!("--bench {kind}")).unwrap().unwrap(),
            Instant::now(),
        );
        b.0.borrow_mut().active = vec![true, true];
        b
    }

    #[test]
    fn no_bench_flag_gives_none() {
        assert_eq!(args("--foo"), Ok(None));
    }

    #[test]
    fn the_flags_parse() {
        let a = args("--bench scroll --seconds 3 --json --warmup-ms 200 --speed 100")
            .unwrap()
            .unwrap();
        assert_eq!(a.kind, BenchKind::Scroll);
        assert_eq!(a.seconds, 3.);
        assert!(a.json);
        assert_eq!(a.warmup, Duration::from_millis(200));
        assert_eq!(a.speed, 100.);
        let a = args("--bench start --runs 10 --unfocused")
            .unwrap()
            .unwrap();
        assert_eq!((a.runs, a.unfocused), (10, true));
    }

    #[test]
    fn the_alert_bench_waits_10_s_unless_the_warmup_is_given() {
        let a = args("--bench alert --seconds 10 --json").unwrap().unwrap();
        assert_eq!(
            (a.kind, a.warmup),
            (BenchKind::Alert, Duration::from_secs(10))
        );
        let a = args("--bench alert --warmup-ms 500").unwrap().unwrap();
        assert_eq!(a.warmup, Duration::from_millis(500));
        let a = args("--bench idle").unwrap().unwrap();
        assert_eq!(a.warmup, Duration::from_millis(1000));
    }

    #[test]
    fn bad_values_are_errors_with_the_usage() {
        for bad in [
            "--bench fast",
            "--bench",
            "--bench idle --seconds -1",
            "--bench idle --seconds nan",
            "--bench idle --seconds inf",
            "--bench idle --seconds 1e30",
            "--bench idle --seconds 0",
            "--bench idle --seconds ten",
            "--bench idle --warmup-ms -5",
            "--bench idle --warmup-ms 99999999999",
            "--bench scroll --speed 0",
            "--bench scroll --speed nan",
            "--bench start --runs 0",
            "--bench start --runs 101",
        ] {
            let e = args(bad).expect_err(bad);
            assert!(e.contains("usage:"), "{bad}: {e}");
        }
    }

    #[test]
    fn stats_take_percentiles() {
        let s = Stats::of((1..=100).map(f64::from).collect());
        assert_eq!(s.n, 100);
        assert_eq!(s.p50, 51.);
        assert_eq!(s.p95, 95.);
        assert_eq!(s.max, 100.);
        assert_eq!(Stats::of(vec![]), Stats::default());
    }

    #[test]
    fn an_idle_report_with_no_frames_passes() {
        let b = measuring("idle");
        {
            let mut s = b.0.borrow_mut();
            s.paints.push(Instant::now());
            s.window_from = Some(Instant::now());
        }
        let r = b.report();
        assert_eq!(
            (r.frames, r.frames_before_window, r.pass),
            (0, 1, Some(true))
        );
        let v: Value = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(v["frames"], 0);
        assert_eq!(v["pass"], true);
    }

    #[test]
    fn an_idle_report_with_frames_fails() {
        let b = measuring("idle");
        {
            let mut s = b.0.borrow_mut();
            s.window_from = Some(Instant::now());
            s.paints.push(Instant::now());
        }
        assert_eq!(b.report().pass, Some(false));
    }

    #[test]
    fn input_or_a_focus_change_makes_the_run_not_valid() {
        let b = measuring("idle");
        {
            let mut s = b.0.borrow_mut();
            s.window_from = Some(Instant::now());
            s.activations.push(Instant::now());
        }
        let r = b.report();
        assert_eq!((r.focus_changes, r.valid, r.pass), (1, false, None));
        let b = measuring("idle");
        {
            let mut s = b.0.borrow_mut();
            s.window_from = Some(Instant::now());
            s.inputs.push(Instant::now());
        }
        assert_eq!(b.report().pass, None);
        b.0.borrow_mut().inputs.clear();
        b.0.borrow_mut().active = vec![true, false];
        assert_eq!(b.report().pass, None);
    }

    #[test]
    fn an_unfocused_run_needs_no_focus_at_both_ends() {
        let b = Bench::new(
            args("--bench idle --unfocused").unwrap().unwrap(),
            Instant::now(),
        );
        {
            let mut s = b.0.borrow_mut();
            s.window_from = Some(Instant::now());
            s.active = vec![false, false];
        }
        assert_eq!(b.report().pass, Some(true));
        b.0.borrow_mut().active = vec![true, false];
        assert_eq!(b.report().pass, None);
    }

    #[test]
    fn latency_samples_of_the_warm_up_do_not_count() {
        let b = measuring("terminal");
        b.record_latency(500.);
        b.record_latency(500.);
        b.0.borrow_mut().window_from = Some(Instant::now());
        b.record_latency(5.);
        b.record_latency(7.);
        let r = b.report();
        assert_eq!(r.latency_ms.n, 2);
        assert_eq!(r.latency_ms.max, 7.);
        assert_eq!(r.pass, Some(true));
    }

    #[test]
    fn an_attention_run_needs_20_samples_and_a_p95_of_100_ms() {
        let b = measuring("attention");
        b.0.borrow_mut().window_from = Some(Instant::now());
        for _ in 0..ATTENTION_SAMPLES - 1 {
            b.record_latency(20.);
        }
        assert_eq!(b.report().pass, None);
        b.record_latency(20.);
        assert_eq!(b.report().pass, Some(true));
        b.record_latency(150.);
        b.record_latency(150.);
        assert_eq!(b.report().pass, Some(false));
    }

    #[test]
    fn proc_readers_give_values() {
        let p = proc_stat().expect("/proc/self/stat");
        assert!(p.cpu_ms >= 0. && p.start_ms > 0.);
        let age = process_age_ms().unwrap();
        assert!(age >= 0., "age {age} ms");
        let rss = process_rss_mb().unwrap();
        assert!(rss > 1. && rss < 100_000.);
    }
}
