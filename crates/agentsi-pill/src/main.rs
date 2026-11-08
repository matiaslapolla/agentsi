//! `agentsi-pill`: the pill process.

use agentsi_pill::audio::{self, Direction};
use agentsi_pill::config::Config;
use agentsi_pill::state::{Effect, Model, Phase};
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "\
usage: agentsi-pill [--input <device>] [--output <device>]
       agentsi-pill --list-devices
       agentsi-pill --show <hidden|preparing|speaking|listening|transcribing|waiting|error> [--toggle|--conversation] [--message <line>] [--seconds <n>]
       agentsi-pill --version

  --show     draw one phase for a few seconds (at most 30) and stop. It does not talk to the core.
  --toggle   with --show listening, draw the mark of toggle dictation.
  --conversation   with --show, draw the mark of conversation mode.
  settings   <config>/agentsi/pill.toml (keys input, output), then AGENTSI_PILL_INPUT and AGENTSI_PILL_OUTPUT, then the flags.
";

/// The longest test window.
const MAX_SHOW: u64 = 30;

struct Args {
    list_devices: bool,
    show: Option<Phase>,
    toggle: bool,
    conversation: bool,
    message: Option<String>,
    seconds: u64,
    flags: Config,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut out = Args {
        list_devices: false,
        show: None,
        toggle: false,
        conversation: false,
        message: None,
        seconds: 5,
        flags: Config::default(),
    };
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--list-devices" => out.list_devices = true,
            "--show" => {
                let v = value("--show")?;
                out.show = Some(Phase::parse(&v).ok_or(format!("unknown phase \"{v}\""))?);
            }
            "--toggle" => out.toggle = true,
            "--conversation" => out.conversation = true,
            "--message" => out.message = Some(value("--message")?),
            "--seconds" => {
                let v = value("--seconds")?;
                out.seconds = v
                    .parse::<u64>()
                    .ok()
                    .filter(|s| (1..=MAX_SHOW).contains(s))
                    .ok_or(format!("--seconds takes 1 to {MAX_SHOW}"))?;
            }
            "--input" => out.flags.input = Some(value("--input")?),
            "--output" => out.flags.output = Some(value("--output")?),
            other => return Err(format!("unknown argument \"{other}\"")),
        }
    }
    Ok(out)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("agentsi-pill {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let args = match parse(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("agentsi-pill: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.list_devices {
        return match audio::list() {
            Ok(devices) => {
                print!("{}", audio::render(&devices));
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("agentsi-pill: cannot list the audio devices: {e}");
                ExitCode::FAILURE
            }
        };
    }
    run(args)
}

#[cfg(target_os = "linux")]
fn run(args: Args) -> ExitCode {
    use agentsi_pill::dictate;
    use agentsi_pill::link::{self, LinkEvent};
    use agentsi_pill::speak;
    use agentsi_pill::state::Input;
    use agentsi_pill::view;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    if let Err(e) = agentsi_pill::wayland_display(|k| std::env::var(k).ok()) {
        eprintln!("agentsi-pill: {e}. The pill needs a Wayland session.");
        return ExitCode::from(2);
    }
    let config = match Config::load() {
        Ok(file) => file.over(Config::from_env()).over(args.flags.clone()),
        Err(e) => {
            eprintln!("agentsi-pill: {e}");
            return ExitCode::from(2);
        }
    };
    let (tx, events) = futures::channel::mpsc::unbounded::<LinkEvent>();
    let mut start = Model::default();
    let mut lifetime = None;
    if let Some(phase) = args.show {
        start = Model::in_phase(
            phase,
            args.message
                .as_deref()
                .or((phase == Phase::Error).then_some(agentsi_pill::link::STOP_FAILED)),
        );
        if args.toggle {
            start = start.with_toggle();
        }
        if args.conversation {
            start = start.with_conversation("demo");
        }
        lifetime = Some(Duration::from_secs(args.seconds));
    } else {
        // A device that is missing or ambiguous shows as an error line, and the pill runs on.
        let mut failures = Vec::new();
        match audio::list() {
            Ok(devices) => {
                for (direction, list, wanted) in [
                    (Direction::Input, &devices.input, &config.input),
                    (Direction::Output, &devices.output, &config.output),
                ] {
                    match audio::choose(list, direction, wanted.as_deref()) {
                        Ok(device) => {
                            eprintln!("agentsi-pill: {direction:?} device: {}", device.name)
                        }
                        Err(e) => {
                            eprintln!("agentsi-pill: {e}");
                            if wanted.is_some() {
                                failures.push(e.to_string());
                            }
                        }
                    }
                }
            }
            Err(e) => eprintln!("agentsi-pill: cannot list the audio devices: {e}"),
        }
        if !failures.is_empty() {
            start.apply(Input::Fail(failures.join(". ")));
        }
        link::spawn(tx.clone());
    }
    let hook: view::EffectHook = if args.show.is_some() {
        Arc::new(|effect: &Effect| eprintln!("agentsi-pill: {effect:?}"))
    } else {
        let speaker = speak::Speaker::start(
            speak::open_voice(),
            speak::open_cpal(config.output.clone()),
            tx.clone(),
            agentsi_pill::stats_enabled(),
        );
        let dictator = dictate::Dictator::start(
            dictate::CpalMic::open(config.input.clone()),
            dictate::open_recognizer(),
            dictate::open_detector(),
            Box::new(dictate::CoreDelivery),
            tx.clone(),
            agentsi_pill::stats_enabled(),
        );
        Arc::new(move |effect: &Effect| {
            speaker.effect(effect);
            dictator.effect(effect);
        })
    };
    view::run(view::Options {
        start,
        link: tx,
        events,
        hook,
        lifetime,
    });
    if agentsi_pill::stats_enabled() {
        eprintln!(
            "agentsi-pill: frames {}",
            view::FRAMES.load(Ordering::Relaxed)
        );
    }
    ExitCode::SUCCESS
}

/// macOS (M2) uses a nonactivating panel through `WindowKind::PopUp`. It is not built yet.
#[cfg(not(target_os = "linux"))]
fn run(_args: Args) -> ExitCode {
    eprintln!("agentsi-pill: the pill runs on Linux only in M1.");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn flags_parse() {
        let a = parse(&args(&[
            "--show",
            "error",
            "--message",
            "x",
            "--seconds",
            "7",
            "--input",
            "USB",
        ]))
        .unwrap();
        assert_eq!(a.show, Some(Phase::Error));
        assert_eq!(a.message.as_deref(), Some("x"));
        assert_eq!(a.seconds, 7);
        assert_eq!(a.flags.input.as_deref(), Some("USB"));
    }

    #[test]
    fn a_test_window_cannot_last_over_30_seconds() {
        assert!(parse(&args(&["--show", "speaking", "--seconds", "31"])).is_err());
        assert!(parse(&args(&["--show", "speaking", "--seconds", "0"])).is_err());
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(parse(&args(&["--show", "loud"])).is_err());
        assert!(parse(&args(&["--input"])).is_err());
        assert!(parse(&args(&["--nope"])).is_err());
    }
}
