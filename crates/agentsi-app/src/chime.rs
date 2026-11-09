//! The tones of the alerts and the rules that decide when one plays.
//!
//! The app makes 4 tones in code: sine notes with a 5 ms rise and an exponential fall to
//! -40 dB. [`Chime::consider`] applies the rules and gives the tone to a [`Sound`]. The
//! [`Speaker`] plays it on the output device. A test gives the chime a [`Recorder`], so no
//! test makes a sound. Spec: `docs/specs/agentsi-desktop-immersive-alerts.md`, "Tones".

use std::time::{Duration, Instant};

use agentsi_proto::rpc::Notification;
use agentsi_ui::tokens::WorkState;
use gpui_kit::Global;

use crate::alerts::Alert;
use crate::alerts_file::{AlertsConfig, Minute};
use crate::screens::call::player::{self, Player, RATE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Attention,
    Ready,
    Error,
    Done,
}

/// One note of a tone: the frequency in Hz (0 is silence) and the length in ms.
type Note = (f32, u32);

impl Tone {
    fn notes(self) -> &'static [Note] {
        match self {
            Tone::Attention => &[(880.0, 120), (0.0, 40), (1175.0, 120)],
            Tone::Ready => &[(660.0, 140), (990.0, 160)],
            Tone::Error => &[(392.0, 160), (294.0, 200)],
            Tone::Done => &[(1320.0, 90)],
        }
    }

    #[cfg(test)]
    pub fn length(self) -> Duration {
        Duration::from_millis(self.notes().iter().map(|n| u64::from(n.1)).sum())
    }

    /// The tone of an alert, or none: a bell makes no tone, and Done plays only when asked.
    pub fn of(alert: &Alert, config: &AlertsConfig) -> Option<Tone> {
        match alert.state {
            WorkState::Blocked if alert.bell => None,
            WorkState::Blocked => Some(Tone::Attention),
            WorkState::Review => Some(Tone::Ready),
            WorkState::Failed => Some(Tone::Error),
            WorkState::Done => config.done.then_some(Tone::Done),
            _ => None,
        }
    }
}

/// The peak of a note: 50 % of full scale, times the volume.
const PEAK: f32 = 0.5;
const RISE_MS: f32 = 5.0;
/// The fall ends at -40 dB.
const FALL_DB: f32 = -40.0;

/// The samples of `tone` at `rate` Hz and `gain` (0.0 to 1.0).
pub fn render(tone: Tone, rate: u32, gain: f32) -> Vec<f32> {
    let per_ms = rate as f32 / 1000.0;
    let mut out = Vec::new();
    for &(hz, ms) in tone.notes() {
        let count = (ms as f32 * per_ms) as usize;
        for i in 0..count {
            let t = i as f32 / rate as f32;
            let sample = if hz == 0.0 {
                0.0
            } else {
                let rise = (t * 1000.0 / RISE_MS).min(1.0);
                let fall = 10f32.powf(FALL_DB / 20.0 * (i as f32 / count as f32));
                (std::f32::consts::TAU * hz * t).sin() * rise * fall * PEAK * gain
            };
            out.push(sample);
        }
    }
    out
}

/// What plays a tone.
pub trait Sound {
    fn play(&mut self, tone: Tone, gain: f32) -> Result<(), String>;
    /// Closes the output device after a quiet time.
    fn close(&mut self);
}

/// The output device. It opens at the first tone.
pub struct Speaker(Player);

impl Speaker {
    pub fn new() -> Speaker {
        Speaker(Player::tones())
    }
}

impl Sound for Speaker {
    fn play(&mut self, tone: Tone, gain: f32) -> Result<(), String> {
        let samples: Vec<i16> = render(tone, RATE, gain)
            .into_iter()
            .map(|s| (s * f32::from(i16::MAX)) as i16)
            .collect();
        self.0.shared.clear();
        self.0.shared.append(&samples);
        self.0.shared.finish_loading();
        self.0.play()
    }

    fn close(&mut self) {
        self.0.close();
    }
}

/// Why a tone did not play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// The alert has no tone.
    NoTone,
    Mute,
    Quiet,
    /// The target is visible in the active window.
    Visible,
    Call,
    /// The pill speaks.
    Speaking,
    /// The microphone is open.
    Microphone,
    /// A tone played less than 2 s before: the two alerts share it.
    Recent,
}

/// The time between two tones.
pub const SHARE: Duration = Duration::from_secs(2);
/// The device closes after this long with no tone.
pub const IDLE: Duration = Duration::from_secs(10);
/// The speech of the pill is guessed at this length for each word.
const WORD: Duration = Duration::from_millis(400);
const SPEECH_MAX: Duration = Duration::from_secs(60);

/// The rules of "Tones", and the voice state that they read.
pub struct Chime {
    sound: Box<dyn Sound>,
    last: Option<Instant>,
    speech_until: Option<Instant>,
    mic: bool,
    conversing: bool,
}

/// What the shell knows when an alert arrives.
#[derive(Debug, Clone, Copy)]
pub struct Scene {
    pub now: Instant,
    pub minute: Minute,
    /// The target is visible in the active window.
    pub visible: bool,
    pub call_playing: bool,
}

impl Global for ChimeGlobal {}

/// The chime of the app, in a global.
pub struct ChimeGlobal(pub std::rc::Rc<std::cell::RefCell<Chime>>);

impl Chime {
    pub fn new(sound: Box<dyn Sound>) -> Chime {
        Chime {
            sound,
            last: None,
            speech_until: None,
            mic: false,
            conversing: false,
        }
    }

    /// Reads an event of the `voice` topic: the speech of the pill and the microphone.
    /// `voice.mic` comes from the core lane. Without it, only the conversation mode and the
    /// time guess of the speech close the rules.
    pub fn hear(&mut self, now: Instant, n: &Notification) {
        match n.method.as_str() {
            "voice.speak" => {
                let words = n
                    .params
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map_or(0, |t| t.split_whitespace().count());
                let guess = (WORD * words as u32).min(SPEECH_MAX);
                self.speech_until = Some(now + guess);
            }
            "voice.stop" => self.speech_until = None,
            "voice.mic" => {
                self.mic = n.params.get("open").and_then(|o| o.as_bool()) == Some(true);
            }
            "converse.state" => match n.params.get("state").and_then(|s| s.as_str()) {
                Some("started") => self.conversing = true,
                Some("stopped") => self.conversing = false,
                _ => {}
            },
            _ => {}
        }
    }

    fn speaking(&self, now: Instant) -> bool {
        self.speech_until.is_some_and(|until| now < until)
    }

    /// Plays the tone of `alert` when no rule forbids it.
    pub fn consider(
        &mut self,
        alert: &Alert,
        config: &AlertsConfig,
        scene: Scene,
    ) -> Result<Tone, Skip> {
        let tone = Tone::of(alert, config).ok_or(Skip::NoTone)?;
        if config.mute {
            return Err(Skip::Mute);
        }
        if config.is_quiet(scene.minute) {
            return Err(Skip::Quiet);
        }
        if scene.visible {
            return Err(Skip::Visible);
        }
        if scene.call_playing {
            return Err(Skip::Call);
        }
        if self.speaking(scene.now) {
            return Err(Skip::Speaking);
        }
        if self.mic || self.conversing {
            return Err(Skip::Microphone);
        }
        if self
            .last
            .is_some_and(|at| scene.now.duration_since(at) < SHARE)
        {
            return Err(Skip::Recent);
        }
        self.play(tone, config.gain(), scene.now);
        Ok(tone)
    }

    /// Plays `Attention` once at the new volume, after the owner changed it. The mute stays.
    pub fn preview(&mut self, config: &AlertsConfig, now: Instant) {
        if !config.mute {
            self.play(Tone::Attention, config.gain(), now);
        }
    }

    fn play(&mut self, tone: Tone, gain: f32, now: Instant) {
        if let Err(e) = self.sound.play(tone, gain) {
            eprintln!("agentsi-app: no tone: {e}");
        }
        self.last = Some(now);
    }

    /// True when the device has been open and quiet for [`IDLE`]. The shell calls this on a
    /// timer after each tone.
    pub fn close_if_idle(&mut self, now: Instant) -> bool {
        match self.last {
            Some(at) if now.duration_since(at) >= IDLE => {
                self.sound.close();
                self.last = None;
                true
            }
            _ => false,
        }
    }
}

/// `call_playing()` of the player, for the scene.
pub fn call_playing() -> bool {
    player::call_playing()
}

/// A [`Sound`] that makes no sound and records the tones, for tests.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct Recorder(pub std::rc::Rc<std::cell::RefCell<Vec<(Tone, f32)>>>);

#[cfg(test)]
impl Sound for Recorder {
    fn play(&mut self, tone: Tone, gain: f32) -> Result<(), String> {
        self.0.borrow_mut().push((tone, gain));
        Ok(())
    }

    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::AlertTarget;
    use serde_json::json;
    use std::time::SystemTime;

    fn alert(state: WorkState, bell: bool) -> Alert {
        Alert {
            id: 1,
            target: AlertTarget::Pane {
                host: None,
                id: "t1".into(),
            },
            state,
            text: String::new(),
            label: String::new(),
            product: None,
            bell,
            at: SystemTime::now(),
        }
    }

    fn note(method: &str, params: serde_json::Value) -> Notification {
        serde_json::from_value(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .unwrap()
    }

    fn scene(now: Instant) -> Scene {
        Scene {
            now,
            minute: 12 * 60,
            visible: false,
            call_playing: false,
        }
    }

    fn chime() -> (Chime, Recorder) {
        let rec = Recorder::default();
        (Chime::new(Box::new(rec.clone())), rec)
    }

    #[test]
    fn each_tone_is_400_ms_or_less_and_stays_inside_its_peak() {
        for (tone, ms) in [
            (Tone::Attention, 280),
            (Tone::Ready, 300),
            (Tone::Error, 360),
            (Tone::Done, 90),
        ] {
            assert_eq!(tone.length(), Duration::from_millis(ms));
            let samples = render(tone, 48_000, 1.0);
            assert_eq!(samples.len(), 48 * ms as usize);
            let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak <= PEAK && peak > 0.4 * PEAK, "{tone:?} peak {peak}");
            let half = render(tone, 48_000, 0.5);
            let half_peak = half.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!((half_peak - peak / 2.0).abs() < 1e-3);
        }
    }

    #[test]
    fn a_note_rises_in_5_ms_and_falls_to_minus_40_db() {
        let s = render(Tone::Done, 48_000, 1.0);
        assert!(s[0].abs() < 1e-3, "starts at zero");
        let tail = s[s.len() - 48..].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(tail < PEAK * 0.02, "the end is near -40 dB: {tail}");
    }

    #[test]
    fn the_tone_of_an_alert_follows_its_state() {
        let c = AlertsConfig::default();
        assert_eq!(
            Tone::of(&alert(WorkState::Blocked, false), &c),
            Some(Tone::Attention)
        );
        assert_eq!(Tone::of(&alert(WorkState::Blocked, true), &c), None);
        assert_eq!(
            Tone::of(&alert(WorkState::Review, false), &c),
            Some(Tone::Ready)
        );
        assert_eq!(
            Tone::of(&alert(WorkState::Failed, false), &c),
            Some(Tone::Error)
        );
        assert_eq!(Tone::of(&alert(WorkState::Done, false), &c), None);
        let on = AlertsConfig { done: true, ..c };
        assert_eq!(
            Tone::of(&alert(WorkState::Done, false), &on),
            Some(Tone::Done)
        );
    }

    #[test]
    fn a_hidden_target_plays_its_tone_at_the_volume() {
        let (mut chime, rec) = chime();
        let c = AlertsConfig {
            volume: 50,
            ..AlertsConfig::default()
        };
        let got = chime.consider(&alert(WorkState::Blocked, false), &c, scene(Instant::now()));
        assert_eq!(got, Ok(Tone::Attention));
        assert_eq!(*rec.0.borrow(), [(Tone::Attention, 0.5)]);
    }

    #[test]
    fn no_tone_plays_for_a_visible_target_the_mute_the_quiet_hours_or_a_call() {
        let (mut chime, rec) = chime();
        let a = alert(WorkState::Failed, false);
        let now = Instant::now();
        let c = AlertsConfig::default();
        let s = scene(now);
        assert_eq!(
            chime.consider(&a, &c, Scene { visible: true, ..s }),
            Err(Skip::Visible)
        );
        assert_eq!(
            chime.consider(
                &a,
                &c,
                Scene {
                    call_playing: true,
                    ..s
                }
            ),
            Err(Skip::Call)
        );
        let mute = AlertsConfig { mute: true, ..c };
        assert_eq!(chime.consider(&a, &mute, s), Err(Skip::Mute));
        let quiet = AlertsConfig {
            quiet: Some((11 * 60, 13 * 60)),
            ..c
        };
        assert_eq!(chime.consider(&a, &quiet, s), Err(Skip::Quiet));
        assert!(rec.0.borrow().is_empty());
    }

    #[test]
    fn the_pill_speech_and_the_microphone_close_the_tone() {
        let (mut chime, rec) = chime();
        let a = alert(WorkState::Review, false);
        let c = AlertsConfig::default();
        let t0 = Instant::now();
        chime.hear(
            t0,
            &note(
                "voice.speak",
                json!({"terminal": "t1", "role": "pm", "text": "one two three"}),
            ),
        );
        assert_eq!(
            chime.consider(&a, &c, scene(t0 + Duration::from_millis(1100))),
            Err(Skip::Speaking)
        );
        // 3 words at 0.4 s: the guess ends at 1.2 s.
        assert_eq!(
            chime.consider(&a, &c, scene(t0 + Duration::from_millis(1300))),
            Ok(Tone::Ready)
        );
        chime.hear(
            t0,
            &note(
                "voice.speak",
                json!({"terminal": "t1", "role": "pm", "text": "again"}),
            ),
        );
        chime.hear(t0, &note("voice.stop", json!({"terminal": "t1"})));
        let later = t0 + Duration::from_secs(5);
        chime.hear(later, &note("voice.mic", json!({"open": true})));
        assert_eq!(chime.consider(&a, &c, scene(later)), Err(Skip::Microphone));
        chime.hear(later, &note("voice.mic", json!({"open": false})));
        assert_eq!(chime.consider(&a, &c, scene(later)), Ok(Tone::Ready));
        assert_eq!(rec.0.borrow().len(), 2);
    }

    #[test]
    fn the_speech_guess_stops_at_60_seconds() {
        let (mut chime, _) = chime();
        let text = "word ".repeat(500);
        let t0 = Instant::now();
        chime.hear(
            t0,
            &note(
                "voice.speak",
                json!({"terminal": "t", "role": "pm", "text": text}),
            ),
        );
        assert!(chime.speaking(t0 + Duration::from_secs(59)));
        assert!(!chime.speaking(t0 + Duration::from_secs(61)));
    }

    #[test]
    fn conversation_mode_closes_the_tone_and_a_missing_mic_event_does_not() {
        let (mut chime, rec) = chime();
        let a = alert(WorkState::Blocked, false);
        let c = AlertsConfig::default();
        let now = Instant::now();
        // The core sends no `voice.mic` yet: nothing marks the microphone open.
        assert_eq!(chime.consider(&a, &c, scene(now)), Ok(Tone::Attention));
        chime.hear(
            now,
            &note(
                "converse.state",
                json!({"state": "started", "terminal": "t1"}),
            ),
        );
        let later = now + Duration::from_secs(5);
        assert_eq!(chime.consider(&a, &c, scene(later)), Err(Skip::Microphone));
        chime.hear(
            later,
            &note(
                "converse.state",
                json!({"state": "stopped", "terminal": "t1"}),
            ),
        );
        assert_eq!(chime.consider(&a, &c, scene(later)), Ok(Tone::Attention));
        assert_eq!(rec.0.borrow().len(), 2);
    }

    #[test]
    fn two_alerts_in_two_seconds_share_one_tone() {
        let (mut chime, rec) = chime();
        let a = alert(WorkState::Blocked, false);
        let c = AlertsConfig::default();
        let t0 = Instant::now();
        assert_eq!(chime.consider(&a, &c, scene(t0)), Ok(Tone::Attention));
        assert_eq!(
            chime.consider(&a, &c, scene(t0 + Duration::from_millis(1900))),
            Err(Skip::Recent)
        );
        assert_eq!(
            chime.consider(&a, &c, scene(t0 + Duration::from_millis(2000))),
            Ok(Tone::Attention)
        );
        assert_eq!(rec.0.borrow().len(), 2);
    }

    #[test]
    fn a_bell_makes_no_tone_and_the_device_closes_after_ten_quiet_seconds() {
        let (mut chime, rec) = chime();
        let t0 = Instant::now();
        let c = AlertsConfig::default();
        assert_eq!(
            chime.consider(&alert(WorkState::Blocked, true), &c, scene(t0)),
            Err(Skip::NoTone)
        );
        chime
            .consider(&alert(WorkState::Blocked, false), &c, scene(t0))
            .unwrap();
        assert!(!chime.close_if_idle(t0 + Duration::from_secs(9)));
        assert!(chime.close_if_idle(t0 + Duration::from_secs(10)));
        assert_eq!(rec.0.borrow().len(), 1);
    }

    #[test]
    fn a_volume_change_previews_attention_unless_the_mute_is_on() {
        let (mut chime, rec) = chime();
        let c = AlertsConfig {
            volume: 80,
            ..AlertsConfig::default()
        };
        chime.preview(&c, Instant::now());
        assert_eq!(*rec.0.borrow(), [(Tone::Attention, 0.8)]);
        chime.preview(&AlertsConfig { mute: true, ..c }, Instant::now());
        assert_eq!(rec.0.borrow().len(), 1);
    }
}
