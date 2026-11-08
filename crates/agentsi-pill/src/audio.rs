//! The audio devices and the choice of one input and one output.

use cpal::traits::{DeviceTrait, HostTrait};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The stable id of the device on this host, for example `alsa:default`.
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub input: Vec<DeviceInfo>,
    pub output: Vec<DeviceInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
}

impl Direction {
    fn word(self) -> &'static str {
        match self {
            Direction::Input => "input",
            Direction::Output => "output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceError {
    /// The host has no device in this direction.
    NoDevice(Direction),
    /// No device matches the request.
    NotFound {
        direction: Direction,
        wanted: String,
    },
    /// More than one device matches the request.
    Ambiguous {
        direction: Direction,
        wanted: String,
        names: Vec<String>,
    },
}

impl fmt::Display for ChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChoiceError::NoDevice(d) => write!(f, "No audio {} device", d.word()),
            ChoiceError::NotFound { direction, wanted } => {
                write!(
                    f,
                    "No audio {} device matches \"{wanted}\"",
                    direction.word()
                )
            }
            ChoiceError::Ambiguous {
                direction,
                wanted,
                names,
            } => {
                write!(
                    f,
                    "\"{wanted}\" matches {} audio {} devices: {}",
                    names.len(),
                    direction.word(),
                    names.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for ChoiceError {}

/// Picks a device. With no request, it takes the default device, else the first.
/// A request matches, in this order: the id, the exact name (ignoring case), and a
/// part of the name (ignoring case). A name or a part that two devices have is ambiguous.
pub fn choose<'a>(
    list: &'a [DeviceInfo],
    direction: Direction,
    wanted: Option<&str>,
) -> Result<&'a DeviceInfo, ChoiceError> {
    let Some(wanted) = wanted.map(str::trim).filter(|w| !w.is_empty()) else {
        return list
            .iter()
            .find(|d| d.default)
            .or(list.first())
            .ok_or(ChoiceError::NoDevice(direction));
    };
    if let Some(device) = list.iter().find(|d| d.id == wanted) {
        return Ok(device);
    }
    let lower = wanted.to_lowercase();
    let exact: Vec<&DeviceInfo> = list
        .iter()
        .filter(|d| d.name.to_lowercase() == lower)
        .collect();
    let partial: Vec<&DeviceInfo> = if exact.is_empty() {
        list.iter()
            .filter(|d| d.name.to_lowercase().contains(&lower))
            .collect()
    } else {
        exact
    };
    match partial.as_slice() {
        [] => Err(ChoiceError::NotFound {
            direction,
            wanted: wanted.to_owned(),
        }),
        [one] => Ok(one),
        many => Err(ChoiceError::Ambiguous {
            direction,
            wanted: wanted.to_owned(),
            names: many.iter().map(|d| d.name.clone()).collect(),
        }),
    }
}

fn default_id(device: Option<cpal::Device>) -> Option<String> {
    device.and_then(|d| d.id().ok()).map(|id| id.to_string())
}

fn describe(device: &cpal::Device, default: &Option<String>) -> Option<DeviceInfo> {
    let id = device.id().ok()?.to_string();
    let name = device.description().ok()?.name().to_owned();
    Some(DeviceInfo {
        default: default.as_deref() == Some(id.as_str()),
        id,
        name,
    })
}

/// Lists the devices of the default host.
pub fn list() -> Result<Devices, String> {
    let host = cpal::default_host();
    let default_input = default_id(host.default_input_device());
    let default_output = default_id(host.default_output_device());
    let input = host
        .input_devices()
        .map_err(|e| e.to_string())?
        .filter_map(|d| describe(&d, &default_input))
        .collect();
    let output = host
        .output_devices()
        .map_err(|e| e.to_string())?
        .filter_map(|d| describe(&d, &default_output))
        .collect();
    Ok(Devices { input, output })
}

/// The output device that `wanted` names, by the rules of [`choose`].
pub fn output_device(wanted: Option<&str>) -> Result<cpal::Device, String> {
    device(Direction::Output, wanted)
}

/// The input device that `wanted` names, by the rules of [`choose`].
pub fn input_device(wanted: Option<&str>) -> Result<cpal::Device, String> {
    device(Direction::Input, wanted)
}

fn device(direction: Direction, wanted: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let (default, found) = match direction {
        Direction::Input => (
            default_id(host.default_input_device()),
            host.input_devices(),
        ),
        Direction::Output => (
            default_id(host.default_output_device()),
            host.output_devices(),
        ),
    };
    let (mut devices, infos): (Vec<cpal::Device>, Vec<DeviceInfo>) = found
        .map_err(|e| e.to_string())?
        .filter_map(|d| describe(&d, &default).map(|info| (d, info)))
        .unzip();
    let chosen = choose(&infos, direction, wanted).map_err(|e| e.to_string())?;
    let index = infos
        .iter()
        .position(|d| d.id == chosen.id)
        .expect("choose returns an item of the list");
    Ok(devices.swap_remove(index))
}

/// The text of `agentsi-pill --list-devices`.
pub fn render(devices: &Devices) -> String {
    let mut out = String::new();
    for (title, list) in [("input", &devices.input), ("output", &devices.output)] {
        out.push_str(&format!("{title}:\n"));
        if list.is_empty() {
            out.push_str("  (none)\n");
        }
        for d in list {
            out.push_str(&format!(
                "  {}{}  ({})\n",
                d.name,
                if d.default { " [default]" } else { "" },
                d.id
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, name: &str, default: bool) -> DeviceInfo {
        DeviceInfo {
            id: id.into(),
            name: name.into(),
            default,
        }
    }

    fn mics() -> Vec<DeviceInfo> {
        vec![
            dev("alsa:a", "Built-in Audio", false),
            dev("alsa:b", "USB Microphone", true),
            dev("alsa:c", "USB Headset", false),
        ]
    }

    #[test]
    fn no_request_gives_the_default_device() {
        assert_eq!(
            choose(&mics(), Direction::Input, None).unwrap().id,
            "alsa:b"
        );
    }

    #[test]
    fn no_request_and_no_default_gives_the_first_device() {
        let list = vec![dev("x", "X", false), dev("y", "Y", false)];
        assert_eq!(
            choose(&list, Direction::Output, Some("  ")).unwrap().id,
            "x"
        );
    }

    #[test]
    fn an_empty_list_has_no_device() {
        assert_eq!(
            choose(&[], Direction::Input, None),
            Err(ChoiceError::NoDevice(Direction::Input))
        );
    }

    #[test]
    fn the_id_matches_first() {
        assert_eq!(
            choose(&mics(), Direction::Input, Some("alsa:c"))
                .unwrap()
                .name,
            "USB Headset"
        );
    }

    #[test]
    fn the_exact_name_ignores_case() {
        assert_eq!(
            choose(&mics(), Direction::Input, Some("built-in audio"))
                .unwrap()
                .id,
            "alsa:a"
        );
    }

    #[test]
    fn a_part_of_the_name_matches_when_only_one_device_has_it() {
        assert_eq!(
            choose(&mics(), Direction::Input, Some("headset"))
                .unwrap()
                .id,
            "alsa:c"
        );
    }

    #[test]
    fn a_part_that_many_devices_have_is_ambiguous() {
        let err = choose(&mics(), Direction::Input, Some("usb")).unwrap_err();
        assert!(matches!(err, ChoiceError::Ambiguous { ref names, .. } if names.len() == 2));
        assert!(err.to_string().contains("USB Microphone, USB Headset"));
    }

    #[test]
    fn two_devices_with_the_same_name_are_ambiguous() {
        let list = vec![dev("a", "USB Audio", false), dev("b", "usb audio", false)];
        assert!(matches!(
            choose(&list, Direction::Input, Some("USB Audio")),
            Err(ChoiceError::Ambiguous { .. })
        ));
    }

    #[test]
    fn an_unknown_request_is_not_found() {
        let err = choose(&mics(), Direction::Output, Some("hdmi")).unwrap_err();
        assert_eq!(err.to_string(), "No audio output device matches \"hdmi\"");
    }

    #[test]
    fn render_marks_the_default() {
        let text = render(&Devices {
            input: mics(),
            output: vec![],
        });
        assert!(text.contains("USB Microphone [default]  (alsa:b)"));
        assert!(text.contains("output:\n  (none)"));
    }
}
