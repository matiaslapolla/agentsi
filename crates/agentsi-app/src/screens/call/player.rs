//! Plays the samples of a call. The core sends the audio as 24 kHz samples, whatever the
//! file format, in chunks of about a minute. The audio callback of the device reads them
//! from `Shared`, so the screen can seek, change the speed and see the position without
//! touching the device. A test gives `Player` an `Output` that makes no sound.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

/// The rate of the samples that the core sends.
pub const RATE: u32 = agentsi_proto::call_api::SAMPLE_RATE;

/// The speeds of the speed button.
pub const SPEEDS: [f32; 4] = [1.0, 1.25, 1.5, 2.0];

#[derive(Default)]
struct Tape {
    samples: Vec<i16>,
    /// The next sample to play. It is fractional while the speed is not 1.
    pos: f64,
    playing: bool,
    speed: f32,
    /// Every sample of the call is in `samples`.
    complete: bool,
    /// The tape reached its end. The screen reads and clears it.
    ended: bool,
}

/// What the screen reads while a call plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// Seconds from the start of the call.
    pub position: f64,
    /// Seconds that the screen has loaded.
    pub loaded: f64,
    pub playing: bool,
    /// Playback waits for samples that are not loaded yet.
    pub buffering: bool,
    pub ended: bool,
}

/// The tape, shared with the audio callback.
pub struct Shared(Mutex<Tape>);

impl Shared {
    pub fn new() -> Shared {
        Shared(Mutex::new(Tape {
            speed: 1.0,
            ..Tape::default()
        }))
    }

    fn tape(&self) -> MutexGuard<'_, Tape> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn append(&self, chunk: &[i16]) {
        self.tape().samples.extend_from_slice(chunk);
    }

    pub fn finish_loading(&self) {
        self.tape().complete = true;
    }

    /// Drops the samples and the position: the call changed on disk.
    pub fn clear(&self) {
        let mut t = self.tape();
        let speed = t.speed;
        *t = Tape {
            speed,
            ..Tape::default()
        };
    }

    pub fn len(&self) -> usize {
        self.tape().samples.len()
    }

    pub fn set_playing(&self, playing: bool) {
        let mut t = self.tape();
        t.playing = playing;
        if playing {
            t.ended = false;
        }
    }

    pub fn set_speed(&self, speed: f32) {
        self.tape().speed = speed;
    }

    /// Moves the position. While samples are still loading it may lie past them: the tape
    /// then waits for them, so a seek before the first play is not lost.
    pub fn seek(&self, seconds: f64) {
        let mut t = self.tape();
        let target = seconds.max(0.0) * f64::from(RATE);
        t.pos = if t.complete {
            target.min(t.samples.len() as f64)
        } else {
            target
        };
        t.ended = false;
    }

    pub fn snapshot(&self) -> Snapshot {
        let t = self.tape();
        Snapshot {
            position: t.pos / f64::from(RATE),
            loaded: t.samples.len() as f64 / f64::from(RATE),
            playing: t.playing,
            buffering: t.playing && !t.complete && t.pos + 1.0 >= t.samples.len() as f64,
            ended: t.ended,
        }
    }

    /// The peak of the last `seconds` of samples before the position, from 0 to 1.
    pub fn peak(&self, seconds: f64) -> f32 {
        let t = self.tape();
        let end = (t.pos as usize).min(t.samples.len());
        let start = end.saturating_sub((seconds * f64::from(RATE)) as usize);
        let top = t.samples[start..end]
            .iter()
            .map(|s| s.unsigned_abs())
            .max()
            .unwrap_or(0);
        f32::from(top) / 32768.
    }

    /// Takes the end flag: true once for each end of the tape.
    pub fn take_ended(&self) -> bool {
        std::mem::take(&mut self.tape().ended)
    }

    /// Writes the next samples of the tape, resampled to `out_rate`, to every channel
    /// of `out`. This is the audio callback: it only locks the tape.
    pub fn fill(&self, out: &mut [f32], channels: usize, out_rate: u32) {
        let mut t = self.tape();
        let step = f64::from(RATE) / f64::from(out_rate.max(1)) * f64::from(t.speed);
        for frame in out.chunks_mut(channels.max(1)) {
            let value = if t.playing {
                let i = t.pos as usize;
                if i + 1 < t.samples.len() {
                    let frac = (t.pos - i as f64) as f32;
                    let (a, b) = (f32::from(t.samples[i]), f32::from(t.samples[i + 1]));
                    t.pos += step;
                    (a + (b - a) * frac) / 32768.0
                } else if t.complete {
                    t.playing = false;
                    t.ended = true;
                    0.0
                } else {
                    0.0
                }
            } else {
                0.0
            };
            frame.iter_mut().for_each(|s| *s = value);
        }
    }
}

fn calls() -> MutexGuard<'static, Vec<Weak<Shared>>> {
    static CALLS: Mutex<Vec<Weak<Shared>>> = Mutex::new(Vec::new());
    CALLS.lock().unwrap_or_else(|p| p.into_inner())
}

/// True while a call screen plays.
pub fn call_playing() -> bool {
    calls()
        .iter()
        .filter_map(Weak::upgrade)
        .any(|s| s.snapshot().playing)
}

/// An open output device.
pub trait Output {
    fn play(&mut self) -> Result<(), String>;
    fn pause(&mut self);
}

/// Opens the device when the owner plays the first time. It gets the tape to read.
pub type Open = Box<dyn Fn(Arc<Shared>) -> Result<Box<dyn Output>, String>>;

pub struct Player {
    pub shared: Arc<Shared>,
    open: Open,
    output: Option<Box<dyn Output>>,
}

impl Player {
    pub fn new(open: Open) -> Player {
        Player {
            shared: Arc::new(Shared::new()),
            open,
            output: None,
        }
    }

    /// The player of a call screen: the default output device. [`call_playing`] sees it.
    pub fn device() -> Player {
        let player = Player::tones();
        let mut calls = calls();
        calls.retain(|c| c.strong_count() > 0);
        calls.push(Arc::downgrade(&player.shared));
        player
    }

    /// A player on the default output device that [`call_playing`] does not see: the tones
    /// of the alerts use it.
    pub fn tones() -> Player {
        Player::new(Box::new(open_default))
    }

    /// Closes the device. The next `play` opens it again.
    pub fn close(&mut self) {
        self.pause();
        self.output = None;
    }

    /// Starts or resumes. The device opens on the first call, so a screen that is only
    /// read never touches it.
    pub fn play(&mut self) -> Result<(), String> {
        if self.output.is_none() {
            self.output = Some((self.open)(self.shared.clone())?);
        }
        if let Some(out) = &mut self.output {
            out.play()?;
        }
        self.shared.set_playing(true);
        Ok(())
    }

    pub fn pause(&mut self) {
        self.shared.set_playing(false);
        if let Some(out) = &mut self.output {
            out.pause();
        }
    }
}

struct Cpal(cpal::Stream);

impl Output for Cpal {
    fn play(&mut self) -> Result<(), String> {
        self.0.play().map_err(|e| e.to_string())
    }

    fn pause(&mut self) {
        // A host that cannot pause keeps the stream, which plays silence.
        let _ = self.0.pause();
    }
}

/// The formats that the stream writes, `f32` first.
const FORMATS: [SampleFormat; 8] = [
    SampleFormat::F32,
    SampleFormat::I16,
    SampleFormat::I32,
    SampleFormat::U16,
    SampleFormat::F64,
    SampleFormat::U8,
    SampleFormat::I8,
    SampleFormat::U32,
];

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let (channels, rate) = (config.channels as usize, config.sample_rate);
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            *config,
            move |out: &mut [T], _| {
                scratch.resize(out.len(), 0.0);
                shared.fill(&mut scratch, channels, rate);
                out.iter_mut()
                    .zip(&scratch)
                    .for_each(|(o, s)| *o = T::from_sample(*s));
            },
            |e| eprintln!("agentsi-app: the output stream failed: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

fn open_default(shared: Arc<Shared>) -> Result<Box<dyn Output>, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("No audio output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let (config, format) = (supported.config(), supported.sample_format());
    if !FORMATS.contains(&format) {
        return Err(format!("the output device wants samples of type {format}"));
    }
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, shared),
        SampleFormat::I16 => build::<i16>(&device, &config, shared),
        SampleFormat::I32 => build::<i32>(&device, &config, shared),
        SampleFormat::U16 => build::<u16>(&device, &config, shared),
        SampleFormat::F64 => build::<f64>(&device, &config, shared),
        SampleFormat::U8 => build::<u8>(&device, &config, shared),
        SampleFormat::I8 => build::<i8>(&device, &config, shared),
        _ => build::<u32>(&device, &config, shared),
    }?;
    let _ = stream.pause();
    Ok(Box::new(Cpal(stream)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<i16> {
        (0..n).map(|i| (i % 100) as i16 * 100).collect()
    }

    #[test]
    fn the_peak_is_the_loudest_sample_of_the_last_window_before_the_position() {
        let s = Shared::new();
        let mut samples = vec![0i16; 2_400];
        samples[100] = 16_384;
        samples[2_000] = -8_192;
        s.append(&samples);
        s.seek(2_400. / f64::from(RATE));
        assert_eq!(s.peak(0.05), 0.25, "50 ms is 1 200 samples: the loud one at 100 is out");
        assert_eq!(s.peak(0.2), 0.5);
        s.seek(0.0);
        assert_eq!(s.peak(0.05), 0.0);
    }

    #[test]
    fn a_stopped_tape_plays_silence_and_a_playing_one_reads_its_samples() {
        let s = Shared::new();
        s.append(&ramp(2_400));
        let mut out = [1.0f32; 8];
        s.fill(&mut out, 2, RATE);
        assert!(out.iter().all(|v| *v == 0.0), "paused");
        s.set_playing(true);
        s.fill(&mut out, 2, RATE);
        // Frame 1 is sample 1 on both channels.
        assert_eq!(out[2], out[3]);
        assert!((out[2] - 100.0 / 32768.0).abs() < 1e-6);
        assert!((s.snapshot().position - 4.0 / f64::from(RATE)).abs() < 1e-9);
    }

    #[test]
    fn a_device_at_twice_the_rate_reads_each_sample_for_two_frames() {
        let s = Shared::new();
        s.append(&ramp(100));
        s.set_playing(true);
        let mut out = [0.0f32; 4];
        s.fill(&mut out, 1, RATE * 2);
        assert!((out[1] - 50.0 / 32768.0).abs() < 1e-6, "{out:?}");
        assert!((out[2] - 100.0 / 32768.0).abs() < 1e-6);
    }

    #[test]
    fn the_speed_moves_the_position_faster() {
        let s = Shared::new();
        s.append(&ramp(24_000));
        s.set_speed(2.0);
        s.set_playing(true);
        s.fill(&mut [0.0; 100], 1, RATE);
        assert!((s.snapshot().position - 200.0 / f64::from(RATE)).abs() < 1e-9);
    }

    #[test]
    fn a_complete_tape_ends_once_and_an_incomplete_one_waits() {
        let s = Shared::new();
        s.append(&ramp(10));
        s.set_playing(true);
        let mut out = [0.0f32; 40];
        s.fill(&mut out, 1, RATE);
        let snap = s.snapshot();
        assert!(
            snap.playing && snap.buffering && !snap.ended,
            "waits for more samples"
        );
        s.append(&ramp(10));
        s.finish_loading();
        s.fill(&mut out, 1, RATE);
        assert!(!s.snapshot().playing);
        assert!(s.take_ended());
        assert!(!s.take_ended(), "the end is reported once");
    }

    #[test]
    fn a_seek_moves_the_position_and_stays_inside_the_loaded_samples() {
        let s = Shared::new();
        s.append(&ramp(48_000));
        s.finish_loading();
        s.seek(0.5);
        assert!((s.snapshot().position - 0.5).abs() < 1e-9);
        s.seek(99.0);
        assert!((s.snapshot().position - 2.0).abs() < 1e-9);
        s.seek(-3.0);
        assert_eq!(s.snapshot().position, 0.0);
    }

    #[test]
    fn a_seek_before_the_samples_load_is_kept_and_playback_waits_for_them() {
        let s = Shared::new();
        s.seek(1.0);
        s.set_playing(true);
        let mut out = [0.0f32; 4];
        s.fill(&mut out, 1, RATE);
        assert!(s.snapshot().buffering);
        assert!(
            (s.snapshot().position - 1.0).abs() < 1e-9,
            "no sample was lost"
        );
        s.append(&ramp(48_000));
        s.fill(&mut out, 1, RATE);
        assert!((s.snapshot().position - (1.0 + 4.0 / f64::from(RATE))).abs() < 1e-9);
    }

    struct Silent(Arc<Mutex<Vec<&'static str>>>);

    impl Output for Silent {
        fn play(&mut self) -> Result<(), String> {
            self.0.lock().unwrap().push("play");
            Ok(())
        }
        fn pause(&mut self) {
            self.0.lock().unwrap().push("pause");
        }
    }

    #[test]
    fn the_device_opens_at_the_first_play_and_a_failure_is_an_error() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let opened = Arc::new(Mutex::new(0));
        let (l, o) = (log.clone(), opened.clone());
        let mut p = Player::new(Box::new(move |_| {
            *o.lock().unwrap() += 1;
            Ok(Box::new(Silent(l.clone())))
        }));
        assert_eq!(*opened.lock().unwrap(), 0);
        p.play().unwrap();
        p.pause();
        p.play().unwrap();
        assert_eq!(*opened.lock().unwrap(), 1);
        assert_eq!(*log.lock().unwrap(), ["play", "pause", "play"]);
        assert!(p.shared.snapshot().playing);

        let mut broken = Player::new(Box::new(|_| Err("No audio output device".into())));
        assert_eq!(broken.play().unwrap_err(), "No audio output device");
        assert!(!broken.shared.snapshot().playing);
    }
}
