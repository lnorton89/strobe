// ============================================================================
// Module:       strobe_audio::capture::device
// Description:  Audio device enumeration and the cpal input stream: opening
//               a device, downmixing to mono, and feeding the ring buffer.
//
// Dependencies: cpal, rtrb, log; crate::capture::CaptureError
// ============================================================================

//! Devices and streams.
//!
//! **What can be captured.** Every input device is a candidate. On Windows
//! an *output* device can be captured too: cpal's WASAPI backend turns on
//! loopback when an input stream is built on a render device, which is how
//! "whatever the speakers are playing" is analysed without a virtual cable.
//! Other platforms list inputs only, because a monitor of the output
//! already shows up as an input there (PulseAudio and PipeWire expose
//! `Monitor of ...` sources).
//!
//! **Linux sound servers.** cpal opens ALSA's `default` PCM, which does not
//! exist under WSLg (only `pulse` does). When the default input will not
//! open, [`open`] tries an input named `pulse`, then `pipewire`, and logs
//! which one it used, before the caller falls back to synthetic audio.
//!
//! **Not every stream error is a failure.** cpal reports a buffer glitch
//! (`Xrun`), a default-device reroute it already followed (`DeviceChanged`)
//! and a refused real-time promotion (`RealtimeDenied`) through the same
//! error callback as a lost device, but the stream keeps running after all
//! three. Those are counted ([`Opened::glitches`]); only the rest mark the
//! stream failed ([`is_fatal`]). Treating a WASAPI glitch as a failure made
//! the worker reopen a working microphone 70 times a second on an AMD
//! desktop (DESKTOP-30KSOCM, Task A1, D266).
//!
//! **The audio callback is a transport stage.** It converts to `f32`,
//! averages the channels into mono, and pushes into a lock-free ring. It
//! does no analysis, takes no lock and allocates nothing; if the analysis
//! thread falls behind, samples are dropped rather than waited for.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig};
use rtrb::{Consumer, Producer, RingBuffer};

use super::CaptureError;

/// Seconds of audio the ring holds. The analysis thread drains it every
/// 10 ms, so a full second absorbs a scheduling stall of a hundred cycles.
const RING_SECONDS: f32 = 1.0;

/// What kind of source a device is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// A microphone, line-in, or monitor source.
    Input,
    /// An output device captured by loopback (Windows).
    Loopback,
}

/// One capturable device, for a settings list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Stable identifier; pass it to [`DeviceChoice::ById`].
    pub id: String,
    /// Name to show a person.
    pub name: String,
    /// Input or loopback.
    pub kind: DeviceKind,
    /// Whether the system default for its direction.
    pub is_default: bool,
}

/// Which device to capture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DeviceChoice {
    /// The system default input device.
    #[default]
    DefaultInput,
    /// The system default output device, captured by loopback (Windows).
    DefaultLoopback,
    /// A specific device from [`list_devices`].
    ById(String),
}

/// A device name for display, tolerating backends that cannot give one.
fn display_name(device: &cpal::Device) -> String {
    device
        .description()
        .map_or_else(|_| "unnamed device".to_owned(), |d| d.name().to_owned())
}

/// A device's stable identifier as a string.
fn device_id(device: &cpal::Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

/// Every device that can be captured now.
///
/// Returns an empty list on a machine with no audio devices, and an error
/// only when the host itself cannot enumerate.
pub fn list_devices() -> Result<Vec<DeviceInfo>, CaptureError> {
    let host = cpal::default_host();
    let default_input = host.default_input_device().and_then(|d| device_id(&d));
    let default_output = host.default_output_device().and_then(|d| device_id(&d));
    let mut found = Vec::new();

    let inputs = host
        .input_devices()
        .map_err(|e| CaptureError::Host(e.to_string()))?;
    for device in inputs {
        let Some(id) = device_id(&device) else {
            continue;
        };
        found.push(DeviceInfo {
            is_default: default_input.as_deref() == Some(id.as_str()),
            name: display_name(&device),
            kind: DeviceKind::Input,
            id,
        });
    }

    if cfg!(windows) {
        let outputs = host
            .output_devices()
            .map_err(|e| CaptureError::Host(e.to_string()))?;
        for device in outputs {
            let Some(id) = device_id(&device) else {
                continue;
            };
            found.push(DeviceInfo {
                is_default: default_output.as_deref() == Some(id.as_str()),
                name: format!("{} (loopback)", display_name(&device)),
                kind: DeviceKind::Loopback,
                id,
            });
        }
    }
    Ok(found)
}

/// An open, playing capture stream and the consumer end of its ring.
pub(crate) struct Opened {
    /// The stream; dropping it stops capture.
    pub(crate) _stream: cpal::Stream,
    /// Mono samples from the audio callback.
    pub(crate) consumer: Consumer<f32>,
    /// Set by the stream's error callback when the device fails.
    pub(crate) failed: Arc<AtomicBool>,
    /// Errors the stream survives (glitches, a followed reroute), counted
    /// by the error callback; the worker reads and resets it.
    pub(crate) glitches: Arc<AtomicU64>,
    /// The device's sample rate.
    pub(crate) sample_rate: u32,
    /// The device's display name.
    pub(crate) name: String,
    /// Whether this is an output device captured by loopback, which
    /// delivers nothing at all while nothing plays, so the stall watchdog
    /// leaves it alone (D420).
    pub(crate) loopback: bool,
}

/// Finds the device `choice` names.
fn find_device(host: &cpal::Host, choice: &DeviceChoice) -> Result<cpal::Device, CaptureError> {
    match choice {
        DeviceChoice::DefaultInput => host.default_input_device().ok_or(CaptureError::NoDevice),
        DeviceChoice::DefaultLoopback => host.default_output_device().ok_or(CaptureError::NoDevice),
        DeviceChoice::ById(text) => {
            let id: DeviceId = text
                .parse()
                .map_err(|_| CaptureError::DeviceNotFound(text.clone()))?;
            host.device_by_id(&id)
                .ok_or_else(|| CaptureError::DeviceNotFound(text.clone()))
        }
    }
}

/// Whether a stream error means the stream has stopped. A glitch, a
/// reroute cpal already followed, and a refused real-time promotion leave
/// it running; anything else, including kinds cpal adds later, is treated
/// as fatal so the device is reopened.
pub(crate) fn is_fatal(kind: ErrorKind) -> bool {
    !matches!(
        kind,
        ErrorKind::Xrun | ErrorKind::DeviceChanged | ErrorKind::RealtimeDenied
    )
}

/// The flags the stream's error callback writes.
#[derive(Clone)]
struct Health {
    failed: Arc<AtomicBool>,
    glitches: Arc<AtomicU64>,
}

/// Builds the input stream for one sample type.
fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut producer: Producer<f32>,
    health: Health,
) -> Result<cpal::Stream, CaptureError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let scale = 1.0 / channels as f32;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                for frame in data.chunks(channels) {
                    let mut sum = 0.0_f32;
                    for sample in frame {
                        sum += (*sample).to_sample::<f32>();
                    }
                    // A full ring means the analysis thread is behind: drop
                    // the sample. Blocking here would stall the driver.
                    let _ = producer.push(sum * scale);
                }
            },
            move |error| {
                if is_fatal(error.kind()) {
                    log::warn!("audio stream error: {error}");
                    health.failed.store(true, Ordering::Release);
                } else {
                    health.glitches.fetch_add(1, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|e| CaptureError::Stream(e.to_string()))
}

/// The sound-server PCMs tried, in order, when the default input will not
/// open on Linux. WSLg has no ALSA `default` PCM but does have `pulse`.
const FALLBACK_NAMES: [&str; 2] = ["pulse", "pipewire"];

/// Which of `names` to use when the default input cannot be opened: the
/// first match for [`FALLBACK_NAMES`], in that order, ignoring case.
fn fallback_index(names: &[String]) -> Option<usize> {
    FALLBACK_NAMES.iter().find_map(|wanted| {
        names
            .iter()
            .position(|name| name.trim().eq_ignore_ascii_case(wanted))
    })
}

/// A device's name for [`fallback_index`]: the backend's own id (ALSA's PCM
/// name, `pulse`) when there is one, else the display name.
fn fallback_name(device: &cpal::Device) -> String {
    device
        .id()
        .map_or_else(|_| display_name(device), |id| id.id().to_owned())
}

/// Opens the input device named `pulse`, else `pipewire`, among the host's
/// inputs. `first` is the error that sent us here, returned if neither opens.
fn open_sound_server(host: &cpal::Host, first: CaptureError) -> Result<Opened, CaptureError> {
    let Ok(inputs) = host.input_devices() else {
        return Err(first);
    };
    let devices: Vec<cpal::Device> = inputs.collect();
    let names: Vec<String> = devices.iter().map(fallback_name).collect();
    let Some(device) = fallback_index(&names).and_then(|i| devices.get(i)) else {
        return Err(first);
    };
    match open_device(device) {
        Ok(opened) => {
            log::info!(
                "default input unavailable ({first}); using the {} device",
                fallback_name(device)
            );
            Ok(opened)
        }
        Err(error) => {
            log::warn!("fallback input {} failed: {error}", fallback_name(device));
            Err(first)
        }
    }
}

/// Opens `choice` and starts it playing. On Linux a default input that will
/// not open falls back to a `pulse` or `pipewire` device (see
/// [`open_sound_server`]).
pub(crate) fn open(choice: &DeviceChoice) -> Result<Opened, CaptureError> {
    let host = cpal::default_host();
    let first = match find_device(&host, choice).and_then(|device| open_device(&device)) {
        Ok(opened) => return Ok(opened),
        Err(error) => error,
    };
    if cfg!(target_os = "linux") && *choice == DeviceChoice::DefaultInput {
        return open_sound_server(&host, first);
    }
    Err(first)
}

/// Opens one device and starts it playing.
fn open_device(device: &cpal::Device) -> Result<Opened, CaptureError> {
    let name = display_name(device);

    // A render device has no input configs; its output config describes the
    // loopback format.
    let loopback = !device.supports_input();
    let supported = if !loopback {
        device.default_input_config()
    } else {
        device.default_output_config()
    }
    .map_err(|e| CaptureError::Stream(e.to_string()))?;

    let config = supported.config();
    let sample_rate = config.sample_rate;
    let capacity = ((sample_rate as f32 * RING_SECONDS) as usize).max(1024);
    let (producer, consumer) = RingBuffer::<f32>::new(capacity);
    let health = Health {
        failed: Arc::new(AtomicBool::new(false)),
        glitches: Arc::new(AtomicU64::new(0)),
    };

    let format = supported.sample_format();
    let stream = if format == SampleFormat::F32 {
        build::<f32>(device, config, producer, health.clone())?
    } else if format == SampleFormat::F64 {
        build::<f64>(device, config, producer, health.clone())?
    } else if format == SampleFormat::I16 {
        build::<i16>(device, config, producer, health.clone())?
    } else if format == SampleFormat::I32 {
        build::<i32>(device, config, producer, health.clone())?
    } else if format == SampleFormat::U16 {
        build::<u16>(device, config, producer, health.clone())?
    } else {
        return Err(CaptureError::UnsupportedFormat(format!("{format:?}")));
    };
    stream
        .play()
        .map_err(|e| CaptureError::Stream(e.to_string()))?;
    Ok(Opened {
        _stream: stream,
        consumer,
        failed: health.failed,
        glitches: health.glitches,
        sample_rate,
        name,
        loopback,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_fallback_prefers_pulse_then_pipewire() -> Result<()> {
        assert!(
            fallback_index(&names(&["hw:0,0", "pipewire", "Pulse"])) == Some(2),
            "pulse wins over pipewire whatever the order, ignoring case"
        );
        assert!(
            fallback_index(&names(&["sysdefault", "pipewire"])) == Some(1),
            "pipewire is used when pulse is absent"
        );
        assert!(
            fallback_index(&names(&["hw:0,0", "default", "pulseaudio-monitor"])).is_none(),
            "only exact names match"
        );
        assert!(fallback_index(&[]).is_none(), "no devices, no fallback");
        Ok(())
    }

    #[test]
    fn a_glitch_a_followed_reroute_and_a_refused_priority_are_not_failures() -> Result<()> {
        for kind in [
            ErrorKind::Xrun,
            ErrorKind::DeviceChanged,
            ErrorKind::RealtimeDenied,
        ] {
            assert!(!is_fatal(kind), "{kind:?} leaves the stream running");
        }
        for kind in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::StreamInvalidated,
            ErrorKind::BackendError,
            ErrorKind::DeviceBusy,
            ErrorKind::Other,
        ] {
            assert!(is_fatal(kind), "{kind:?} stops the stream");
        }
        Ok(())
    }
}
