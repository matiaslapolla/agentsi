//! `<config>/agentsi/alerts.toml`: the mute, the volume, the tone of Done, the system
//! notifications and the quiet hours.
//!
//! ```toml
//! mute = false
//! volume = 60                 # 0 to 100
//! done = false                # the tone of Done
//! system = true               # system notifications
//! quiet = ["22:00", "08:00"]  # absent: no quiet hours
//! ```
//!
//! - A missing file means the defaults. An invalid key gives its default and one line in
//!   [`note`]. An unknown key is kept in the file and noted.
//! - [`change`] applies a choice at once and saves it, with the comments and unknown keys
//!   of the file. A file that does not parse is not overwritten.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::{App, Global};
use toml_edit::{Array, DocumentMut, Item, value};

pub const FILE_NAME: &str = "alerts.toml";

/// A time of day, in minutes after midnight.
pub type Minute = u16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertsConfig {
    pub mute: bool,
    /// 0 to 100.
    pub volume: u8,
    pub done: bool,
    pub system: bool,
    /// From and to. The range may cross midnight.
    pub quiet: Option<(Minute, Minute)>,
}

impl Default for AlertsConfig {
    fn default() -> Self {
        AlertsConfig {
            mute: false,
            volume: 60,
            done: false,
            system: true,
            quiet: None,
        }
    }
}

impl AlertsConfig {
    /// The volume as a factor of the peak: 0.0 to 1.0.
    pub fn gain(&self) -> f32 {
        f32::from(self.volume.min(100)) / 100.0
    }

    /// True when `now` lies in the quiet hours.
    pub fn is_quiet(&self, now: Minute) -> bool {
        match self.quiet {
            None => false,
            Some((from, to)) if from == to => false,
            Some((from, to)) if from < to => (from..to).contains(&now),
            Some((from, to)) => now >= from || now < to,
        }
    }
}

/// "22:00" gives 1320. Hours 0 to 23, minutes 0 to 59.
pub fn parse_time(text: &str) -> Option<Minute> {
    let (h, m) = text.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u16, u16) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

pub fn format_time(minute: Minute) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

/// The minute of the day on the local clock.
pub fn local_minute(at: SystemTime) -> Minute {
    let secs = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: `tm` is plain data. `localtime_r` only reads `secs` and writes `tm`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    (tm.tm_hour.clamp(0, 23) * 60 + tm.tm_min.clamp(0, 59)) as Minute
}

/// `<config>/agentsi/alerts.toml`.
pub fn path() -> Option<PathBuf> {
    crate::layout::path().map(|p| p.with_file_name(FILE_NAME))
}

/// The file in `path`, and one line for the owner when something in it cannot be used.
pub fn read(path: &Path) -> (AlertsConfig, Option<String>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (AlertsConfig::default(), None),
        Err(e) => (
            AlertsConfig::default(),
            Some(format!(
                "{} cannot be read: {e}. The default alerts stay.",
                path.display()
            )),
        ),
    }
}

fn parse(text: &str, path: &Path) -> (AlertsConfig, Option<String>) {
    let doc: DocumentMut = match text.parse() {
        Ok(d) => d,
        Err(e) => {
            let e: toml_edit::TomlError = e;
            return (
                AlertsConfig::default(),
                Some(format!(
                    "{} does not parse: {}. The default alerts stay.",
                    path.display(),
                    e.message()
                )),
            );
        }
    };
    let mut problems = Vec::new();
    let mut config = AlertsConfig::default();
    let flag = |key: &str, item: &Item, slot: &mut bool, problems: &mut Vec<String>| match item
        .as_bool()
    {
        Some(b) => *slot = b,
        None => problems.push(format!("{key} must be true or false.")),
    };
    for (key, item) in doc.iter() {
        match key {
            "mute" => flag(key, item, &mut config.mute, &mut problems),
            "done" => flag(key, item, &mut config.done, &mut problems),
            "system" => flag(key, item, &mut config.system, &mut problems),
            "volume" => match item.as_integer() {
                Some(v @ 0..=100) => config.volume = v as u8,
                _ => problems.push("volume must be a whole number from 0 to 100.".into()),
            },
            "quiet" => match quiet_of(item) {
                Some(range) => config.quiet = Some(range),
                None => {
                    problems.push("quiet must be two times, like [\"22:00\", \"08:00\"].".into())
                }
            },
            other => problems.push(format!("{other}: no setting has this name.")),
        }
    }
    let note = (!problems.is_empty()).then(|| {
        format!(
            "{}: {} The default stays for each of these.",
            path.display(),
            problems.join(" ")
        )
    });
    (config, note)
}

fn quiet_of(item: &Item) -> Option<(Minute, Minute)> {
    let array = item.as_array()?;
    let mut times = array.iter().map(|v| v.as_str().and_then(parse_time));
    let (from, to) = (times.next()??, times.next()??);
    times.next().is_none().then_some((from, to))
}

/// The file and the line about it, kept as a global so the settings screen shows them.
#[derive(Clone, Debug, Default)]
pub struct Setting {
    path: Option<PathBuf>,
    note: Option<String>,
    config: AlertsConfig,
}

impl Global for Setting {}

/// Reads the file at `path` and makes it the alerts of the app. The app calls this once.
pub fn install(cx: &mut App, path: Option<PathBuf>) {
    let (config, note) = match &path {
        Some(p) => read(p),
        None => (AlertsConfig::default(), None),
    };
    cx.set_global(Setting { path, note, config });
}

/// The line about the file, or none when it is whole.
pub fn note(cx: &App) -> Option<String> {
    cx.try_global::<Setting>().and_then(|s| s.note.clone())
}

/// The settings in effect.
pub fn current(cx: &App) -> AlertsConfig {
    cx.try_global::<Setting>()
        .map(|s| s.config)
        .unwrap_or_default()
}

/// Applies `f` to the settings in effect and saves the result. The change applies at once,
/// even when the save fails. The error is one line for the owner, and [`note`] shows it too.
pub fn change(cx: &mut App, f: impl FnOnce(&mut AlertsConfig)) -> Result<(), String> {
    let mut setting = cx.try_global::<Setting>().cloned().unwrap_or_default();
    f(&mut setting.config);
    let saved = match &setting.path {
        Some(path) => write(path, setting.config),
        None => Err("There is no config folder, so the alerts cannot be saved.".into()),
    };
    setting.note = saved.clone().err();
    cx.set_global(setting);
    saved
}

fn write(path: &Path, config: AlertsConfig) -> Result<(), String> {
    crate::keys::edit(path, |doc| {
        doc["mute"] = value(config.mute);
        doc["volume"] = value(i64::from(config.volume));
        doc["done"] = value(config.done);
        doc["system"] = value(config.system);
        match config.quiet {
            Some((from, to)) => {
                let mut times = Array::new();
                times.push(format_time(from));
                times.push(format_time(to));
                doc["quiet"] = value(times);
            }
            None => {
                doc.remove("quiet");
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentsi-alerts-file-{name}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn of(text: &str) -> (AlertsConfig, Option<String>) {
        parse(text, Path::new("alerts.toml"))
    }

    #[test]
    fn a_missing_file_gives_the_defaults_and_no_note() {
        let (c, note) = read(&tempdir("missing").join(FILE_NAME));
        assert_eq!((c, note), (AlertsConfig::default(), None));
        assert_eq!(c.volume, 60);
        assert!(c.system && !c.mute && !c.done && c.quiet.is_none());
    }

    #[test]
    fn each_key_of_the_file_sets_its_setting() {
        let (c, note) = of(
            "mute = true\nvolume = 20\ndone = true\nsystem = false\nquiet = [\"22:00\", \"08:00\"]\n",
        );
        assert_eq!(note, None);
        assert_eq!(
            c,
            AlertsConfig {
                mute: true,
                volume: 20,
                done: true,
                system: false,
                quiet: Some((1320, 480)),
            }
        );
    }

    #[test]
    fn an_invalid_key_gives_its_default_and_one_note_and_the_good_keys_stay() {
        let (c, note) =
            of("mute = true\nvolume = 150\nquiet = [\"25:00\", \"08:00\"]\nsystem = 3\nfont = 1\n");
        assert!(c.mute);
        assert_eq!(c.volume, 60);
        assert_eq!(c.quiet, None);
        assert!(c.system);
        let note = note.expect("one note");
        assert!(note.contains("volume must be a whole number from 0 to 100"));
        assert!(note.contains("quiet must be two times"));
        assert!(note.contains("system must be true or false"));
        assert!(note.contains("font: no setting has this name"));
        assert_eq!(note.lines().count(), 1, "one note, one line: {note}");
    }

    #[test]
    fn a_file_that_does_not_parse_gives_the_defaults_and_one_note() {
        let (c, note) = of("mute = = true");
        assert_eq!(c, AlertsConfig::default());
        assert!(note.unwrap().contains("does not parse"));
    }

    #[test]
    fn a_write_keeps_the_comments_and_the_unknown_keys() {
        let path = tempdir("write").join(FILE_NAME);
        std::fs::write(&path, "# my alerts\nmine = 1\nvolume = 10\n").unwrap();
        let config = AlertsConfig {
            mute: true,
            volume: 35,
            quiet: Some((60, 120)),
            ..AlertsConfig::default()
        };
        write(&path, config).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# my alerts") && text.contains("mine = 1"),
            "{text}"
        );
        assert_eq!(read(&path).0, config);
        write(&path, AlertsConfig::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("quiet"), "{text}");
        assert!(text.contains("mine = 1"));
    }

    #[test]
    fn a_file_that_does_not_parse_is_not_overwritten() {
        let path = tempdir("broken").join(FILE_NAME);
        std::fs::write(&path, "mute = = x").unwrap();
        assert!(write(&path, AlertsConfig::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mute = = x");
    }

    #[test]
    fn the_quiet_hours_cross_midnight() {
        let c = AlertsConfig {
            quiet: Some((1320, 480)),
            ..AlertsConfig::default()
        };
        assert!(c.is_quiet(1380) && c.is_quiet(0) && c.is_quiet(479));
        assert!(!c.is_quiet(480) && !c.is_quiet(720) && !c.is_quiet(1319));
        let day = AlertsConfig {
            quiet: Some((600, 720)),
            ..AlertsConfig::default()
        };
        assert!(day.is_quiet(600) && !day.is_quiet(720));
        assert!(!AlertsConfig::default().is_quiet(0));
    }

    #[test]
    fn a_time_needs_two_digits_for_the_hour_and_the_minute() {
        assert_eq!(parse_time("08:05"), Some(485));
        assert_eq!(parse_time("8:05"), None);
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("12:60"), None);
        assert_eq!(format_time(485), "08:05");
        assert!(local_minute(SystemTime::now()) < 1440);
    }
}
