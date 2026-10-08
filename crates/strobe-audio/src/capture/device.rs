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
//! **The audio callback is a transport stage.** It converts to `f32`,
//! averages the channels into mono, and pushes into a lock-free ring. It
//! does no analysis, takes no lock and allocates nothing; if the analysis
//! thread falls behind, samples are dropped rather than waited for.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{DeviceId, FromSample, SampleFormat, SizedSample, StreamConfig};
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
    /// The device's sample rate.
    pub(crate) sample_rate: u32,
    /// The device's display name.
    pub(crate) name: String,
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

/// Builds the input stream for one sample type.
fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut producer: Producer<f32>,
    failed: Arc<AtomicBool>,
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
                log::warn!("audio stream error: {error}");
                failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| CaptureError::Stream(e.to_string()))
}

/// Opens `choice` and starts it playing.
pub(crate) fn open(choice: &DeviceChoice) -> Result<Opened, CaptureError> {
    let host = cpal::default_host();
    let device = find_device(&host, choice)?;
    let name = display_name(&device);

    // A render device has no input configs; its output config describes the
    // loopback format.
    let supported = if device.supports_input() {
        device.default_input_config()
    } else {
        device.default_output_config()
    }
    .map_err(|e| CaptureError::Stream(e.to_string()))?;

    let config = supported.config();
    let sample_rate = config.sample_rate;
    let capacity = ((sample_rate as f32 * RING_SECONDS) as usize).max(1024);
    let (producer, consumer) = RingBuffer::<f32>::new(capacity);
    let failed = Arc::new(AtomicBool::new(false));

    let format = supported.sample_format();
    let stream = if format == SampleFormat::F32 {
        build::<f32>(&device, config, producer, failed.clone())?
    } else if format == SampleFormat::F64 {
        build::<f64>(&device, config, producer, failed.clone())?
    } else if format == SampleFormat::I16 {
        build::<i16>(&device, config, producer, failed.clone())?
    } else if format == SampleFormat::I32 {
        build::<i32>(&device, config, producer, failed.clone())?
    } else if format == SampleFormat::U16 {
        build::<u16>(&device, config, producer, failed.clone())?
    } else {
        return Err(CaptureError::UnsupportedFormat(format!("{format:?}")));
    };
    stream
        .play()
        .map_err(|e| CaptureError::Stream(e.to_string()))?;
    Ok(Opened {
        _stream: stream,
        consumer,
        failed,
        sample_rate,
        name,
    })
}
