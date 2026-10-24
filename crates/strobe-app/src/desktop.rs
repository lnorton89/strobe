// ============================================================================
// Module:       strobe_app::desktop
// Description:  Native remote desktop viewer, UI input translation and bounded
//               background JPEG capture and decoding workers.
//
// Dependencies: egui, image, base64; strobe-remote; std::sync, std::thread
// ============================================================================

//! The host's actual UI is mirrored, rather than reimplementing each action.
//! Host rendering and input stay on the app thread; compression, socket I/O
//! and image decoding run on workers. Captures are limited to five per second.
//! Only Strobe's control window is exposed, never the operating system desktop.

use base64::Engine as _;
use egui::{Event, Pos2, Vec2};
use std::io::{self, Cursor};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use strobe_remote::desktop::{Desktop, Input, MAX_EDGE, MAX_JPEG, Modifiers};
use strobe_remote::{ClientMessage, ErrorCode, ServerMessage};

use crate::trusted;

/// Maximum capture rate, including a control window covered by fullscreen output.
pub const STREAM_PERIOD: Duration = Duration::from_millis(200);

/// Whether `action` is one the operator must take at the host, so it is
/// refused from a UI pass that carried remote desktop input (D275).
///
/// Full desktop control is the whole Strobe window, editor and file saves
/// included, as the owner asked. What it must not do is widen who else can
/// reach this computer, or make that wider reach outlive the session:
/// approving or inviting another device, switching a network listener on
/// or rebinding OSC (which is saved, unauthenticated, and could be put on
/// every interface), and turning this host into a controller of a third
/// machine. Narrowing (Stop, Deny, Revoke, switching desktop access off)
/// stays allowed. A local click in the same pass is refused too; the
/// notice says to do it again at the host.
pub fn host_only(action: &crate::ui::Action) -> bool {
    use crate::ui::Action;
    matches!(
        action,
        Action::SetLanRemote(true)
            | Action::SetDesktopAccess(true)
            | Action::DecideRemotePairing { approve: true, .. }
            | Action::ConnectDesktop
            | Action::SetOsc(_)
            | Action::SetImportListener(true)
    )
}

/// The host's capture of its control window (D275), off the frame's
/// critical path in both halves.
///
/// - **Readback without a stall.** A capture is a `glReadPixels` into a
///   pixel-pack buffer with a fence behind it, which returns at once; a
///   later control frame collects it only when the fence has signalled (a
///   zero-timeout poll). The context is shared with the output, so a
///   readback into client memory would wait for the output's queued frame
///   too. A capture not ready is collected next time; one that failed is
///   dropped.
/// - **No allocation per capture.** The pack buffer lives while a lease
///   does and is rebuilt only when the window's size changes. Pixel
///   buffers go to the encoder and come back to be reused, so at most a
///   few exist.
/// - **Encoding on a worker.** Queue capacity is one: a capture is skipped
///   when the encoder is behind, never waited for or accumulated.
pub struct Capture {
    sender: Option<SyncSender<CaptureFrame>>,
    returned: Option<Receiver<Vec<u8>>>,
    spare: Option<Vec<u8>>,
    worker: Option<JoinHandle<()>>,
    due: Instant,
    gpu: Option<Readback>,
}

/// The pack buffer and the capture in flight, on the context that made them.
struct Readback {
    gl: strobe_gfx::gl::Gl,
    buffer: strobe_gfx::gl::Buffer,
    /// The window size the buffer was made for.
    size: [u32; 2],
    /// The size and fence of the capture in flight.
    pending: Option<([u32; 2], strobe_gfx::gl::Fence)>,
}

struct CaptureFrame {
    desktop: Desktop,
    size: [u32; 2],
    pixels: Vec<u8>,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            sender: None,
            returned: None,
            spare: None,
            worker: None,
            due: Instant::now(),
            gpu: None,
        }
    }
}

impl Capture {
    /// Once per control frame, after the paint and before the swap: collect
    /// a finished capture for the encoder and, when one is due for an
    /// active lease, start the next. Never waits on the GPU.
    pub fn frame(
        &mut self,
        gl: &strobe_gfx::gl::Gl,
        now: Instant,
        desktop: &Desktop,
        size: [u32; 2],
    ) {
        if !desktop.active() {
            // No lease: free the pack buffer and anything in flight.
            self.gpu = None;
            return;
        }
        if self
            .gpu
            .as_ref()
            .is_some_and(|gpu| gpu.gl.is_lost() || gl.is_lost())
        {
            // A rebuilt context: these objects belong to the old one.
            self.gpu = None;
        }
        self.collect(desktop);
        if self.gpu.as_ref().is_some_and(|gpu| gpu.pending.is_some()) || !self.take_due(now) {
            return;
        }
        if size.contains(&0) {
            // Minimised: nothing to show. The controller sees its image go
            // stale and disables its input until frames come back.
            return;
        }
        if size.iter().any(|edge| *edge > MAX_EDGE) {
            desktop.capture_failed(
                "Resize the host control window to at most 4096 pixels per edge, then pair again",
            );
            return;
        }
        self.start(gl, size);
    }

    /// Whether a capture is due at `now`; when it is, the next one is
    /// scheduled.
    ///
    /// The next is due a little under [`STREAM_PERIOD`] on. A control
    /// window the event loop runs at the stream's own rate (covered, with a
    /// lease: D290) has its frames that far apart as the loop schedules
    /// them, but `now` is read after make_current, which can wait up to a
    /// vertical blank for the output; without the slack, a frame read a
    /// few ms earlier than the last was not due, and the image came every
    /// 400 ms. In view, at 30 Hz, it is still a capture every sixth frame.
    fn take_due(&mut self, now: Instant) -> bool {
        /// Covers make_current's wait, up to a 60 Hz refresh and more.
        const SLACK: Duration = Duration::from_millis(25);
        if now < self.due {
            return false;
        }
        self.due = now + STREAM_PERIOD.saturating_sub(SLACK);
        true
    }

    fn start(&mut self, gl: &strobe_gfx::gl::Gl, size: [u32; 2]) {
        if self.gpu.as_ref().is_none_or(|gpu| gpu.size != size) {
            self.gpu = None;
            match gl.pack_buffer(size[0] as usize * size[1] as usize * 4) {
                Ok(buffer) => {
                    self.gpu = Some(Readback {
                        gl: gl.clone(),
                        buffer,
                        size,
                        pending: None,
                    });
                }
                Err(error) => {
                    log::warn!("desktop capture: no pack buffer: {error}");
                    return;
                }
            }
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let started = gl
            .read_default_pixels_into(&gpu.buffer, size[0] as i32, size[1] as i32)
            .and_then(|()| gl.fence());
        match started {
            Ok(fence) => gpu.pending = Some((size, fence)),
            Err(error) => log::warn!("desktop capture: readback not started: {error}"),
        }
    }

    fn collect(&mut self, desktop: &Desktop) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let Some((size, fence)) = gpu.pending.as_ref() else {
            return;
        };
        let size = *size;
        match gpu.gl.fence_status(fence) {
            strobe_gfx::gl::FenceStatus::Pending => return,
            strobe_gfx::gl::FenceStatus::Failed => {
                gpu.pending = None;
                return;
            }
            strobe_gfx::gl::FenceStatus::Signaled => {}
        }
        gpu.pending = None;
        let mut pixels = self
            .spare
            .take()
            .or_else(|| self.returned.as_ref().and_then(|back| back.try_recv().ok()))
            .unwrap_or_default();
        pixels.resize(size[0] as usize * size[1] as usize * 4, 0);
        if gpu.gl.read_buffer(&gpu.buffer, &mut pixels) {
            self.submit(desktop.clone(), size, pixels);
        } else {
            self.spare = Some(pixels);
        }
    }

    /// Hand bottom-up RGBA to the encoder, skipping if the queue is full.
    fn submit(&mut self, desktop: Desktop, size: [u32; 2], pixels: Vec<u8>) {
        if self.sender.is_none() {
            let (sender, frames) = sync_channel::<CaptureFrame>(1);
            let (give_back, returned) = sync_channel::<Vec<u8>>(2);
            match std::thread::Builder::new()
                .name("desktop capture".into())
                .spawn(move || encoder(&frames, &give_back))
            {
                Ok(worker) => {
                    self.sender = Some(sender);
                    self.returned = Some(returned);
                    self.worker = Some(worker);
                }
                Err(error) => {
                    log::warn!("desktop capture worker: {error}");
                    self.spare = Some(pixels);
                    return;
                }
            }
        }
        let Some(sender) = &self.sender else {
            self.spare = Some(pixels);
            return;
        };
        match sender.try_send(CaptureFrame {
            desktop,
            size,
            pixels,
        }) {
            Ok(()) => {}
            // The encoder is behind: this capture is skipped, its buffer kept.
            Err(TrySendError::Full(frame) | TrySendError::Disconnected(frame)) => {
                self.spare = Some(frame.pixels);
            }
        }
    }
}

/// The capture worker: encode, publish, and give the buffer back.
fn encoder(frames: &Receiver<CaptureFrame>, give_back: &SyncSender<Vec<u8>>) {
    let mut ladder = Ladder::default();
    while let Ok(mut frame) = frames.recv() {
        if frame.desktop.active() {
            match encode(frame.size, &mut frame.pixels, &mut ladder) {
                Ok(encoded) => frame.desktop.publish(
                    encoded.size,
                    strobe_remote::client::encode_image(&encoded.jpeg),
                ),
                Err(error) => {
                    log::warn!("desktop capture: {error}");
                    frame.desktop.capture_failed(
                        "Host capture failed; resize the host window and pair again",
                    );
                }
            }
        }
        // A full return queue drops this buffer; the app makes another.
        let _ = give_back.try_send(frame.pixels);
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(worker) = self.worker.take() {
            // Shutdown cannot recover a failed worker; it already reported captures.
            let _ = worker.join();
        }
    }
}

/// The encoder's steps from the best picture to the smallest, as (how many
/// times smaller each edge is, JPEG quality). A capture tries them in turn
/// until one fits [`MAX_JPEG`] (D406).
const LADDER: [(u32, u8); 8] = [
    (1, 90),
    (1, 75),
    (1, 60),
    (1, 45),
    (2, 75),
    (2, 60),
    (2, 45),
    (4, 60),
];

/// Captures in a row that fit in a third of [`MAX_JPEG`] before the
/// encoder tries the next better step again: five seconds at the stream
/// rate, so a picture that only just fits a step is not encoded twice
/// every other capture.
const CLIMB_AFTER: u32 = 25;

/// Where on [`LADDER`] the encoder is, kept from capture to capture so a
/// steady picture is encoded once.
#[derive(Debug, Default)]
struct Ladder {
    step: usize,
    easy: u32,
}

/// One encoded capture: what was sent and at what size.
struct Encoded {
    size: [u32; 2],
    jpeg: Vec<u8>,
}

fn limit_error(why: &str) -> image::ImageError {
    image::ImageError::IoError(io::Error::new(io::ErrorKind::InvalidData, why))
}

/// `rgb` (`size`, top-down) made `factor` times smaller on each edge, each
/// pixel the mean of the block it covers.
fn shrink(rgb: &[u8], size: [u32; 2], factor: u32) -> (Vec<u8>, [u32; 2]) {
    let factor = factor.max(1) as usize;
    let [width, height] = size.map(|edge| edge as usize);
    let small = [(width / factor).max(1), (height / factor).max(1)];
    let mut out = Vec::with_capacity(small[0] * small[1] * 3);
    for y in 0..small[1] {
        for x in 0..small[0] {
            let mut sum = [0_u32; 3];
            let mut count = 0_u32;
            for row in (y * factor..(y + 1) * factor).take_while(|row| *row < height) {
                let start = (row * width + x * factor) * 3;
                let end = (row * width + ((x + 1) * factor).min(width)) * 3;
                for pixel in rgb.get(start..end).unwrap_or_default().as_chunks::<3>().0 {
                    for (total, channel) in sum.iter_mut().zip(pixel) {
                        *total += u32::from(*channel);
                    }
                    count += 1;
                }
            }
            let count = count.max(1);
            out.extend(sum.map(|total| (total / count) as u8));
        }
    }
    (out, small.map(|edge| edge as u32))
}

/// Encodes a bottom-up RGBA capture, from `ladder`'s step down until it
/// fits [`MAX_JPEG`]; it fails only when the smallest step is too big.
fn encode(
    size: [u32; 2],
    pixels: &mut [u8],
    ladder: &mut Ladder,
) -> Result<Encoded, image::ImageError> {
    let [width, height] = size;
    let stride = width as usize * 4;
    if width == 0
        || height == 0
        || width > MAX_EDGE
        || height > MAX_EDGE
        || pixels.len() != stride * height as usize
    {
        return Err(image::ImageError::IoError(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid capture dimensions",
        )));
    }
    for y in 0..height as usize / 2 {
        let other = height as usize - 1 - y;
        let (top, bottom) = pixels.split_at_mut(other * stride);
        top[y * stride..(y + 1) * stride].swap_with_slice(&mut bottom[..stride]);
    }
    let rgb: Vec<u8> = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| pixel[..3].iter().copied())
        .collect();
    let mut shrunk: Option<(u32, Vec<u8>, [u32; 2])> = None;
    let mut step = ladder.step.min(LADDER.len() - 1);
    loop {
        let Some(&(factor, quality)) = LADDER.get(step) else {
            return Err(limit_error("capture exceeds JPEG limit"));
        };
        let (source, source_size) = if factor == 1 {
            (rgb.as_slice(), size)
        } else {
            if shrunk.as_ref().is_none_or(|(made, _, _)| *made != factor) {
                let (small, small_size) = shrink(&rgb, size, factor);
                shrunk = Some((factor, small, small_size));
            }
            match shrunk.as_ref() {
                Some((_, small, small_size)) => (small.as_slice(), *small_size),
                None => (rgb.as_slice(), size),
            }
        };
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality).encode(
            source,
            source_size[0],
            source_size[1],
            image::ExtendedColorType::Rgb8,
        )?;
        if jpeg.len() <= MAX_JPEG {
            // Back up a step only after a run of captures with room to spare.
            ladder.easy = if step > 0 && jpeg.len() <= MAX_JPEG / 3 {
                ladder.easy + 1
            } else {
                0
            };
            ladder.step = step;
            if ladder.easy >= CLIMB_AFTER {
                ladder.step = step - 1;
                ladder.easy = 0;
            }
            return Ok(Encoded {
                size: source_size,
                jpeg,
            });
        }
        ladder.easy = 0;
        step += 1;
        if step >= LADDER.len() {
            ladder.step = LADDER.len() - 1;
            return Err(limit_error("capture exceeds JPEG limit"));
        }
        log::debug!(
            "desktop capture: {} KiB is over the limit; trying {}x smaller at quality {} (D406)",
            jpeg.len() / 1024,
            LADDER.get(step).map_or(1, |(factor, _)| *factor),
            LADDER.get(step).map_or(0, |(_, quality)| *quality)
        );
    }
}

#[derive(Default)]
struct View {
    clipboard: Option<String>,
    note: String,
    revision: u64,
    image: Option<Arc<egui::ColorImage>>,
    size: [u32; 2],
    ready: bool,
    stage: Stage,
    /// The connection reuses a saved pairing, so no approval is awaited.
    reconnecting: bool,
    /// The connection proves a trust file (D288/D294): no code, no
    /// approval, and it retries by itself when it drops.
    trusted: bool,
}

/// Where a connection stands, for the viewer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Stage {
    /// Connecting, or waiting for the host's operator to approve.
    #[default]
    Connecting,
    /// The host accepted this desktop. Frames arrive, or are paused.
    Live,
    /// Over, and why. The last image, if any, stays on screen.
    Ended(End),
    /// A trusted connection (D294) that dropped, or could not be made yet,
    /// and is being tried again: this is attempt `n`, waiting out its
    /// backoff or in flight. The last image, if any, stays on screen. It
    /// ends only when the person disconnects or the host refuses.
    Reconnecting(u32),
}

/// Why a connection ended, which decides what the viewer offers next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    /// Nobody approved this desktop on the host in time.
    TimedOut,
    /// The host answered no: a wrong or stale code, or a revoked pairing.
    Refused,
    /// The host or the network went away, or the host dropped this desktop,
    /// after it had accepted it.
    Lost,
    /// The host could not be reached, or dropped the attempt before it
    /// accepted this desktop.
    Unreachable,
}

struct Credential {
    address: std::net::SocketAddr,
    token: String,
}

/// How long a connection waits for the host's operator to approve this
/// desktop before it gives up and says so (item 2 of the T480 to RX 580
/// test: it used to wait for ever).
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

/// The host address a person typed, as a socket address. A bare IP address
/// takes the remote's default port, so "192.168.1.20" is enough.
pub fn parse_host(text: &str) -> Result<SocketAddr, &'static str> {
    let text = text.trim();
    if let Ok(address) = text.parse::<SocketAddr>() {
        return Ok(address);
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, strobe_remote::server::DEFAULT_PORT));
    }
    Err("Enter the host's address, for example 192.168.1.20 (add :port if it is not 47812).")
}

/// Whether a connection has waited long enough for approval.
fn approval_expired(waited: Duration, limit: Duration) -> bool {
    waited >= limit
}

/// The viewer's line while it waits: the time spent, and when it stops.
fn waiting_text(waited: Duration, limit: Duration) -> String {
    format!(
        "Waiting {} s of {} s",
        waited.as_secs().min(limit.as_secs()),
        limit.as_secs()
    )
}

/// What to do on the host while this desktop waits.
fn wait_hint(reconnecting: bool, trusted: bool) -> &'static str {
    if trusted {
        "Proving the trust file to the host needs no approval. If this takes more than a few \
         seconds, check that Strobe is running on the host with --remote, \
         --allow-desktop-control and --remote-trust, on the same network."
    } else if reconnecting {
        "Reconnecting with the saved pairing needs no approval. If this takes more than a few \
         seconds, check that Strobe is running on the host and on the same network."
    } else {
        "On the host, a request from this desktop appears in Settings > Remote, and the \
         title bar says so: press Approve."
    }
}

/// What the viewer says when nobody approved in time.
fn timed_out_text(limit: Duration, reconnecting: bool) -> String {
    if reconnecting {
        format!(
            "The host did not answer within {} s. Check that Strobe is running there on the \
             same network, then reconnect.",
            limit.as_secs()
        )
    } else {
        format!(
            "The host did not approve this desktop within {} s. Ask again, and press Approve \
             on the host when the request appears.",
            limit.as_secs()
        )
    }
}

/// What the viewer says when the host answers no before the session starts.
///
/// The host's own words ("not paired, or revoked") are right for a phone
/// and no help to the person at this desktop, so each code says what to do
/// next: ask again, and approve on the host (D468).
fn refusal_text(code: ErrorCode, host_message: &str) -> String {
    const WHERE: &str = "On the host, Settings > Remote needs the remote on and full desktop \
                         control allowed; ask again from here and press Approve there.";
    match code {
        ErrorCode::Unauthorized => format!(
            "The host no longer knows this desktop (not paired, or the pairing was revoked). \
             {WHERE}"
        ),
        ErrorCode::PairingRejected => format!("The host said no ({host_message}). {WHERE}"),
        ErrorCode::Unavailable
        | ErrorCode::VersionMismatch
        | ErrorCode::InvalidState
        | ErrorCode::InvalidRequest
        | ErrorCode::Busy => host_message.to_string(),
    }
}

/// A failure's reason as a sentence: capitalised, with one full stop. The
/// card's heading says what kind of failure it was.
fn lost_text(reason: &str) -> String {
    let reason = reason.trim().trim_end_matches('.');
    let mut chars = reason.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => "The connection ended.".to_string(),
    }
}

/// Records how a connection ended. The first ending stands: a later error
/// from the same worker (the socket it closed itself) does not replace it.
fn end(view: &Mutex<View>, how: End, note: String) {
    let mut view = view.lock().unwrap_or_else(PoisonError::into_inner);
    if !matches!(view.stage, Stage::Ended(_)) {
        view.stage = Stage::Ended(how);
        view.note = note;
    }
    view.ready = false;
}

/// Controller state owned by the app. Address/code edits are presentation
/// state; connecting is performed through an Action after the UI pass.
pub struct Controller {
    /// The host's address, e.g. 192.168.1.20 or 192.168.1.20:47812. Empty
    /// until the person types one; the form shows a hint, not a value.
    pub address: String,
    /// One-time code shown by the host; cleared immediately on connect.
    pub code: String,
    /// Venue mode (D294): connect by proving a trust file instead of typing
    /// a code. The form's checkbox; remembered across launches.
    pub use_trust: bool,
    /// Where the trust file is, as typed. Only this path is remembered,
    /// never the file's contents.
    pub trust_path: String,
    /// This laptop's id on the host's list: stable across launches, and
    /// valid for the protocol (`trust::valid_controller_id`).
    pub controller_id: String,
    /// The current (or last) connection proves a trust file, so it retries
    /// by itself and a refusal offers Reconnect rather than a new code.
    trusted: bool,
    view: Arc<Mutex<View>>,
    sender: Option<SyncSender<Input>>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
    texture: Option<egui::TextureHandle>,
    texture_context: Option<egui::Context>,
    revision: u64,
    focused: bool,
    credential: Arc<Mutex<Option<Credential>>>,
    input_note: String,
    input_frame: Option<(egui::Context, u64)>,
    /// When the current connection began, for the elapsed time it shows.
    started: Option<Instant>,
    /// How long it waits for approval before it gives up.
    approval_limit: Duration,
    /// Something asked for the connect form to be brought into view and
    /// focused (the command palette, or a failed connection's retry).
    reveal: bool,
    /// The waiting card's heading and time line, and the whole second they
    /// were formatted for: they change once a second, not every frame.
    waiting: Option<(u64, String, String)>,
    /// The stage and readiness the viewer last drew, so the app redraws it
    /// when the worker changes either ([`Self::needs_redraw`]).
    shown: Option<(Stage, bool)>,
}

impl Default for Controller {
    fn default() -> Self {
        Self {
            address: String::new(),
            code: String::new(),
            use_trust: false,
            trust_path: String::new(),
            controller_id: String::new(),
            trusted: false,
            view: Arc::new(Mutex::new(View::default())),
            sender: None,
            worker: None,
            stop: Arc::new(AtomicBool::new(false)),
            socket: Arc::new(Mutex::new(None)),
            texture: None,
            texture_context: None,
            revision: 0,
            focused: false,
            credential: Arc::new(Mutex::new(None)),
            input_note: String::new(),
            input_frame: None,
            started: None,
            approval_limit: APPROVAL_TIMEOUT,
            reveal: false,
            waiting: None,
            shown: None,
        }
    }
}

impl Controller {
    /// Whether the remote view has replaced the local shell.
    pub fn active(&self) -> bool {
        self.worker.is_some()
    }

    /// The controller a launch starts with, and what to do first (D294).
    ///
    /// The form starts from what was remembered: the trust file's path, and
    /// whether it was in use, and this laptop's `controller_id`.
    /// `--control HOST:PORT` then decides the first action. With
    /// `--remote-trust FILE` it opens straight into the viewer (the file is
    /// used even if the form had it off). Without, it opens the form with
    /// the address filled in and the trust option off, for pairing with a
    /// code: a remembered trust file never connects by itself.
    pub fn at_launch(
        remembered: &crate::prefs::RemotePref,
        control: Option<SocketAddr>,
        trust_file: Option<&std::path::Path>,
    ) -> (Self, Option<crate::ui::Action>) {
        let mut controller = Self::default();
        controller
            .controller_id
            .clone_from(&remembered.controller_id);
        controller.trust_path.clone_from(&remembered.trust_file);
        controller.use_trust = remembered.use_trust && !remembered.trust_file.is_empty();
        let Some(address) = control else {
            return (controller, None);
        };
        controller.address = address.to_string();
        match trust_file {
            Some(file) => {
                controller.trust_path = file.display().to_string();
                controller.use_trust = true;
                (controller, Some(crate::ui::Action::ConnectDesktop))
            }
            None => {
                controller.use_trust = false;
                (controller, Some(crate::ui::Action::ControlAnotherDesktop))
            }
        }
    }

    /// What the form should remember after a Connect: the trust file's
    /// path (never its contents) and whether it is in use.
    pub fn remembered(&self, into: &mut crate::prefs::RemotePref) {
        into.trust_file = trusted::clean_path(&self.trust_path).to_string();
        into.use_trust = self.use_trust;
    }

    /// Start a fresh pairing session; no bearer secret is saved or logged.
    pub fn connect(&mut self) {
        self.disconnect();
        let address = match parse_host(&self.address) {
            Ok(address) => address,
            Err(note) => {
                self.set_note(note);
                return;
            }
        };
        // Show the address as it will be used, port included.
        self.address = address.to_string();
        let trust = if self.use_trust {
            // The whole path, shown and remembered as the file really read.
            self.trust_path = trusted::whole_path(&self.trust_path);
            match trusted::load_plan(&self.trust_path, &self.controller_id) {
                Ok(plan) => Some(plan),
                Err(note) => {
                    self.set_note(&note);
                    return;
                }
            }
        } else {
            None
        };
        // A trust file replaces the code; one typed as well is dropped.
        let code = std::mem::take(&mut self.code);
        let code = if trust.is_some() { String::new() } else { code };
        // With no code and no pairing held, the worker asks the host to
        // approve this desktop (D468).
        let remembered = self.remembers(address);
        self.trusted = trust.is_some();
        if trust.is_some() {
            log::info!(
                "desktop controller: connecting to {address} as {} with a trust file",
                self.controller_id
            );
        }
        self.view = Arc::new(Mutex::new(View {
            note: "Connecting".into(),
            reconnecting: trust.is_none() && code.trim().is_empty() && remembered,
            trusted: trust.is_some(),
            ..View::default()
        }));
        self.started = Some(Instant::now());
        self.stop = Arc::new(AtomicBool::new(false));
        // A fresh slot, so a worker still ending from the last connection
        // cannot overwrite this one's interrupter.
        self.socket = Arc::new(Mutex::new(None));
        let name = if strobe_remote::trust::valid_controller_id(&self.controller_id) {
            self.controller_id.clone()
        } else {
            strobe_remote::trust::controller_id_for(&trusted::computer_name(), [0; 4])
        };
        let link = Link {
            address,
            code,
            name,
            approval_limit: self.approval_limit,
            view: Arc::clone(&self.view),
            stop: Arc::clone(&self.stop),
            socket: Arc::clone(&self.socket),
            credential: Arc::clone(&self.credential),
            trust,
        };
        let (sender, inputs) = sync_channel(512);
        match std::thread::Builder::new()
            .name("desktop controller".into())
            .spawn(move || work(&link, &inputs))
        {
            Ok(worker) => {
                self.sender = Some(sender);
                self.worker = Some(worker);
            }
            Err(error) => {
                self.started = None;
                self.set_note(&error.to_string());
            }
        }
    }

    fn set_note(&self, note: &str) {
        self.view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note = note.to_string();
    }

    /// Whether a pairing for `address` is held in memory, so a blank code
    /// reconnects without the host's approval.
    fn remembers(&self, address: SocketAddr) -> bool {
        self.credential
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|credential| credential.address == address)
    }

    /// Asks for the connect form to be brought into view and focused the
    /// next time Settings draws it.
    pub fn reveal_form(&mut self) {
        self.reveal = true;
    }

    /// Whether [`Self::reveal_form`] asked and the form has not been shown
    /// since: Settings turns to the page the form is on.
    pub fn reveal_pending(&self) -> bool {
        self.reveal
    }

    /// Takes the request [`Self::reveal_form`] made, once.
    pub fn take_reveal(&mut self) -> bool {
        std::mem::take(&mut self.reveal)
    }

    /// Close the socket before joining; stale input is never replayed.
    pub fn disconnect(&mut self) {
        let was_active = self.worker.is_some();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(socket) = self
            .socket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            // A socket already closed by the peer is equally interrupted.
            let _ = socket.shutdown(Shutdown::Both);
        }
        self.sender = None;
        if let Some(worker) = self.worker.take()
            && worker.is_finished()
        {
            // The worker's visible status carries errors; shutdown only joins it.
            let _ = worker.join();
        }
        // An unfinished worker is left to end by itself, never joined here:
        // this is the frame loop's thread, and a worker still connecting
        // has no socket to interrupt yet (up to 3 s to connect and 5 s for
        // the handshake). It sees `stop` at its next turn and touches only
        // its own connection's state.
        {
            // The last image is only worth keeping while the viewer shows
            // it. What the form says afterwards must be true of now.
            let mut view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
            view.image = None;
            if was_active {
                match view.stage {
                    Stage::Connecting => view.note = "Connection cancelled.".into(),
                    Stage::Live => view.note = "Disconnected from the host.".into(),
                    Stage::Reconnecting(_) => {
                        view.note = "Disconnected from the host. Not reconnecting.".into();
                    }
                    Stage::Ended(_) => {}
                }
            }
        }
        self.started = None;
        self.waiting = None;
        self.shown = None;
        self.texture = None;
        self.texture_context = None;
        self.revision = 0;
        self.focused = false;
        self.input_note.clear();
        self.input_frame = None;
    }

    /// Whether the viewer has something new to draw: an image from the
    /// host, or a stage or readiness the worker changed since the last
    /// [`Self::show`]. The app redraws an idle viewer slowly and brings the
    /// next redraw forward for this (D302). One lock, no allocation.
    pub fn needs_redraw(&self) -> bool {
        let view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
        (view.image.is_some() && view.revision != self.revision)
            || self.shown != Some((view.stage, view.ready))
    }

    /// Status for Settings or the always-visible viewer toolbar.
    pub fn note(&self) -> String {
        self.view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note
            .clone()
    }

    fn send(&mut self, event: Input) {
        if let Some(sender) = &self.sender {
            match sender.try_send(event) {
                Ok(()) => {}
                Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                    // A trusted connection does not stop: closing its socket
                    // makes the worker see the drop and try again (D294).
                    if !self.trusted {
                        self.stop.store(true, Ordering::Relaxed);
                    }
                    if let Some(socket) = self
                        .socket
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .as_ref()
                    {
                        // A peer-closed socket already satisfies the interruption.
                        let _ = socket.shutdown(Shutdown::Both);
                    }
                    if self.trusted {
                        self.view
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .ready = false;
                        return;
                    }
                    // Ended, not paused: no frame is coming back on this
                    // connection, and a held pairing offers Reconnect.
                    end(
                        &self.view,
                        End::Lost,
                        "The desktop connection was interrupted: input could not reach the host."
                            .into(),
                    );
                }
            }
        }
    }

    /// Entire remote control window, with a local disconnect button that
    /// cannot be captured by the host or forwarded as a remote click.
    ///
    /// What fills the window follows the connection's stage: a waiting
    /// card with the time spent while the host has yet to approve, the
    /// host's window while it is live, and, once it has ended, the last
    /// image the host sent, dimmed and read-only, under a card saying why
    /// and what to do next.
    pub fn show(&mut self, ui: &mut egui::Ui, actions: &mut Vec<crate::ui::Action>) {
        if ui.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::F12,
            )
        }) {
            actions.push(crate::ui::Action::DisconnectDesktop);
        }
        let (size, ready, stage, note, reconnecting, trusted) = {
            let mut view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(text) = view.clipboard.take() {
                ui.ctx().copy_text(text);
            }
            if (view.revision != self.revision || self.texture_context.as_ref() != Some(ui.ctx()))
                && let Some(image) = view.image.clone()
            {
                if self.texture_context.as_ref() != Some(ui.ctx()) {
                    self.texture = None;
                }
                match &mut self.texture {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.texture = Some(ui.ctx().load_texture(
                            "remote desktop",
                            image,
                            egui::TextureOptions::LINEAR,
                        ))
                    }
                }
                self.revision = view.revision;
                self.texture_context = Some(ui.ctx().clone());
            }
            (
                view.size,
                view.ready,
                view.stage,
                view.note.clone(),
                view.reconnecting,
                view.trusted,
            )
        };
        self.shown = Some((stage, ready));
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Controlling {}", self.address));
            let leave = match stage {
                Stage::Connecting => "Cancel",
                Stage::Reconnecting(_) => "Disconnect",
                Stage::Live | Stage::Ended(_) => "Return to local Strobe",
            };
            if ui.button(leave).clicked() {
                actions.push(crate::ui::Action::DisconnectDesktop);
            }
        });
        if let Stage::Live = stage {
            ui.label(&note);
        }
        if !self.input_note.is_empty() {
            ui.label(&self.input_note);
        }
        match stage {
            Stage::Connecting => {
                let waited = self.started.map_or(Duration::ZERO, |began| began.elapsed());
                // The count moves on without any input arriving: repaint
                // when it next changes, once a second.
                ui.ctx().request_repaint_after(
                    Duration::from_secs(1)
                        .saturating_sub(Duration::from_nanos(u64::from(waited.subsec_nanos()))),
                );
                let second = waited.as_secs();
                if self.waiting.as_ref().is_none_or(|(at, ..)| *at != second) {
                    self.waiting = Some((
                        second,
                        format!("Connecting to {}", self.address),
                        waiting_text(waited, self.approval_limit),
                    ));
                }
                if let Some((_, heading, line)) = &self.waiting {
                    card(
                        ui.ctx(),
                        heading,
                        &[line, wait_hint(reconnecting, trusted)],
                        None,
                    );
                }
            }
            Stage::Reconnecting(attempt) => {
                // The attempt changes on its own schedule, with no input.
                ui.ctx().request_repaint_after(Duration::from_millis(500));
                if let Some(texture) = &self.texture {
                    dimmed_image(ui, texture.id(), size);
                }
                let heading = format!("Reconnecting\u{2026} (attempt {attempt})");
                let advice = if self.texture.is_some() {
                    "The dimmed picture is the last image the host sent. It is read-only."
                } else {
                    "Nothing has been received from the host yet."
                };
                card(ui.ctx(), &heading, &[note.as_str(), advice], None);
            }
            Stage::Ended(how) => {
                let (heading, advice) = match how {
                    End::TimedOut => ("No approval from the host", None),
                    End::Refused => ("The host refused this desktop", None),
                    End::Lost => (
                        "Connection lost",
                        Some(
                            "The dimmed picture is the last image the host sent. It is \
                             read-only.",
                        ),
                    ),
                    End::Unreachable => ("Could not connect to the host", None),
                };
                if let Some(texture) = &self.texture {
                    dimmed_image(ui, texture.id(), size);
                }
                // A trusted connection needs no pairing held: pressing
                // Reconnect proves the file again.
                let reconnect = self.trusted || self.remembers_current();
                let with_advice;
                let alone;
                let lines: &[&str] = match advice.filter(|_| self.texture.is_some()) {
                    Some(advice) => {
                        with_advice = [note.as_str(), advice];
                        &with_advice
                    }
                    None => {
                        alone = [note.as_str()];
                        &alone
                    }
                };
                let mut offer = |ui: &mut egui::Ui| {
                    if reconnect {
                        if ui.button("Reconnect").clicked() {
                            actions.push(crate::ui::Action::ConnectDesktop);
                        }
                    } else if ui.button("Ask again").clicked() {
                        actions.push(crate::ui::Action::ConnectDesktop);
                    }
                };
                card(ui.ctx(), heading, lines, Some(&mut offer));
            }
            Stage::Live => self.live(ui, size, ready),
        }
    }

    /// Whether a blank code would reconnect to the address shown.
    fn remembers_current(&self) -> bool {
        parse_host(&self.address).is_ok_and(|address| self.remembers(address))
    }

    /// The host's window: interactive while frames arrive, dimmed while
    /// they are paused.
    fn live(&mut self, ui: &mut egui::Ui, size: [u32; 2], ready: bool) {
        let Some(texture) = &self.texture else {
            return;
        };
        if !ready {
            dimmed_image(ui, texture.id(), size);
            card(
                ui.ctx(),
                "Waiting for the host's image",
                &["Input is off until fresh frames arrive. The picture is the last one received."],
                None,
            );
            return;
        }
        let fit = fit_image(ui.available_size(), size);
        let response =
            ui.add(egui::Image::new((texture.id(), fit)).sense(egui::Sense::click_and_drag()));
        if response.clicked()
            || response.drag_started()
            || (response.hovered() && ui.input(|input| input.pointer.any_pressed()))
            || (self.focused && ui.input(|input| input.focused && !input.pointer.any_pressed()))
        {
            response.request_focus();
        }
        let frame = ui.ctx().cumulative_frame_nr();
        if self
            .input_frame
            .as_ref()
            .is_some_and(|(ctx, number)| ctx == ui.ctx() && *number == frame)
        {
            return;
        }
        self.input_frame = Some((ui.ctx().clone(), frame));
        let focused = response.has_focus();
        if self.focused && !focused {
            self.send(Input::Reset {});
        }
        self.focused = focused;
        let events = ui.input(|input| input.events.clone());
        for event in events {
            if let Event::Paste(text) | Event::Text(text) | Event::Ime(egui::ImeEvent::Commit(text)) =
                &event
                && (text.len() > 64 * 1024 || text.contains('\0'))
            {
                self.input_note =
                    "Text was not sent: paste at most 64 KiB at a time, without NUL characters."
                        .into();
                continue;
            }
            if matches!(event, Event::MouseWheel { .. }) && !response.hovered() {
                continue;
            }
            for input in outgoing(&event, response.rect, focused) {
                self.send(input);
            }
        }
    }
}

/// The largest rectangle of the host window's aspect that fits `available`.
fn fit_image(available: Vec2, size: [u32; 2]) -> Vec2 {
    let aspect = size[0] as f32 / size[1].max(1) as f32;
    Vec2::new(
        available.x.min(available.y * aspect).max(1.0),
        available.y.min(available.x / aspect).max(1.0),
    )
}

/// The last image from the host, darkened and without any input.
fn dimmed_image(ui: &mut egui::Ui, texture: egui::TextureId, size: [u32; 2]) {
    let fit = fit_image(ui.available_size(), size);
    ui.add(egui::Image::new((texture, fit)).tint(egui::Color32::from_gray(70)));
}

/// A message card in the middle of the window, over whatever is drawn
/// there, with `buttons` under the text. It is local UI: nothing in it is
/// sent to the host.
fn card(
    ctx: &egui::Context,
    heading: &str,
    lines: &[&str],
    buttons: Option<&mut dyn FnMut(&mut egui::Ui)>,
) {
    let width = (ctx.content_rect().width() - 48.0).clamp(120.0, 460.0);
    egui::Area::new(egui::Id::new("remote desktop notice"))
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::window(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                ui.label(egui::RichText::new(heading).strong().size(15.0));
                for line in lines {
                    ui.label(*line);
                }
                if let Some(buttons) = buttons {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(buttons);
                }
            });
        });
}

/// A state of the viewer to draw without a host (tests and screenshots).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Staged {
    /// Waiting for approval, this many seconds in.
    Waiting(u64),
    /// Reconnecting with a saved pairing, this many seconds in.
    Reconnecting(u64),
    /// Live, frames arriving.
    Live,
    /// Live, but the host's image paused.
    Paused,
    /// The host dropped this desktop after frames had arrived.
    Dropped,
    /// Dropped by the network before a single frame arrived.
    Unreachable,
    /// Nobody approved in time.
    TimedOut,
    /// A saved pairing the host no longer knows.
    Refused,
    /// A reconnect that timed out, with its pairing still held.
    TimedOutReconnect,
    /// A trusted connection (D294) that dropped after frames had arrived
    /// and is on this retry attempt.
    Retrying(u32),
    /// A trusted connection the host refused.
    TrustRefused,
    /// Live, over a trusted connection.
    TrustedLive,
}

#[cfg(test)]
impl Controller {
    /// A viewer in `how` talking to `address`, with a gradient standing in
    /// for the host's window where one would have arrived. It has no
    /// connection: nothing is sent or received.
    pub(crate) fn staged(address: &str, how: Staged) -> Self {
        let mut controller = Self::default();
        controller.address = address.into();
        // A worker that has already finished: the viewer is "active".
        controller.worker = std::thread::Builder::new().spawn(|| {}).ok();
        let waited = match how {
            Staged::Waiting(seconds) | Staged::Reconnecting(seconds) => seconds,
            Staged::Live
            | Staged::Paused
            | Staged::Dropped
            | Staged::Unreachable
            | Staged::TimedOut
            | Staged::Refused
            | Staged::TimedOutReconnect
            | Staged::Retrying(_)
            | Staged::TrustRefused
            | Staged::TrustedLive => 0,
        };
        controller.started = Instant::now().checked_sub(Duration::from_secs(waited));
        let with_image = matches!(
            how,
            Staged::Live
                | Staged::Paused
                | Staged::Dropped
                | Staged::Retrying(_)
                | Staged::TrustedLive
        );
        controller.trusted = matches!(
            how,
            Staged::Retrying(_) | Staged::TrustRefused | Staged::TrustedLive
        );
        let reconnecting = matches!(how, Staged::Reconnecting(_) | Staged::TimedOutReconnect);
        if let (Some(address), true) = (
            parse_host(address).ok(),
            matches!(how, Staged::Dropped | Staged::TimedOutReconnect),
        ) {
            *controller
                .credential
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Credential {
                address,
                token: "held".into(),
            });
        }
        let limit = controller.approval_limit;
        let mut view = controller
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        view.reconnecting = reconnecting;
        view.trusted = controller.trusted;
        if with_image {
            let size = [640_usize, 360];
            let mut pixels = Vec::with_capacity(size[0] * size[1]);
            for y in 0..size[1] {
                for x in 0..size[0] {
                    pixels.push(egui::Color32::from_rgb(
                        (x * 255 / size[0]) as u8,
                        (y * 255 / size[1]) as u8,
                        160,
                    ));
                }
            }
            view.image = Some(Arc::new(egui::ColorImage {
                size,
                source_size: Vec2::new(size[0] as f32, size[1] as f32),
                pixels,
            }));
            view.size = [size[0] as u32, size[1] as u32];
            view.revision = 1;
        }
        match how {
            Staged::Waiting(_) | Staged::Reconnecting(_) => {
                view.note = "Connecting".into();
            }
            Staged::Live | Staged::TrustedLive => {
                view.stage = Stage::Live;
                view.ready = true;
                view.note = "Full desktop control. Edits and file paths apply on the host.".into();
            }
            Staged::Paused => {
                view.stage = Stage::Live;
                view.note =
                    "Host image paused. Input is disabled until fresh frames arrive.".into();
            }
            Staged::Dropped => {
                view.stage = Stage::Ended(End::Lost);
                view.note = lost_text("Host closed desktop control");
            }
            Staged::Unreachable => {
                view.stage = Stage::Ended(End::Unreachable);
                view.note = lost_text("connection refused (os error 111)");
            }
            Staged::TimedOut | Staged::TimedOutReconnect => {
                view.stage = Stage::Ended(End::TimedOut);
                view.note = timed_out_text(limit, reconnecting);
            }
            Staged::Refused => {
                view.stage = Stage::Ended(End::Refused);
                view.note = refusal_text(ErrorCode::Unauthorized, "not paired, or revoked");
            }
            Staged::Retrying(attempt) => {
                view.stage = Stage::Reconnecting(attempt);
                view.note =
                    trusted::retry_text("Connection lost: host closed desktop control.", attempt);
            }
            Staged::TrustRefused => {
                view.stage = Stage::Ended(End::Refused);
                view.note = trusted::refusal_text(
                    ErrorCode::PairingRejected,
                    "that trust file does not match this host",
                );
            }
        }
        drop(view);
        controller
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// What a controller worker shares with the app thread.
struct Link {
    address: SocketAddr,
    code: String,
    /// This desktop's name in a code-free request (D468).
    name: String,
    approval_limit: Duration,
    view: Arc<Mutex<View>>,
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
    credential: Arc<Mutex<Option<Credential>>>,
    /// Prove this trust file instead of a code, and retry on a drop.
    trust: Option<trusted::Plan>,
}

/// How long the worker sleeps between looks at `stop` while it waits out a
/// retry.
const RETRY_SLICE: Duration = Duration::from_millis(50);

/// Sleeps for `wait` unless `stop` is set first. Returns whether to go on.
fn pause(stop: &AtomicBool, wait: Duration) -> bool {
    let until = Instant::now() + wait;
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(RETRY_SLICE));
    }
}

/// The worker thread's whole life: one connection, and for a trusted one
/// (D294) as many more as it takes.
///
/// A code-paired connection that fails ends the viewer's story, as before
/// (D286): the person chooses Reconnect. A trusted connection that drops, or
/// cannot be made yet, becomes [`Stage::Reconnecting`] and is tried again
/// after [`trusted::backoff`], for ever, until `stop` is set by Disconnect.
/// A refusal, or any other ending `control` has already put into the view as
/// [`Stage::Ended`], is final: retrying a wrong file only feeds the host's
/// failed-proof limiter.
fn work(link: &Link, inputs: &Receiver<Input>) {
    let mut attempt: u32 = 0;
    loop {
        let mut pictured = false;
        let ended = control(link, inputs, &mut pictured);
        // However the connection ended, its socket closes now. The
        // interrupter in the slot is a second handle on it, so dropping the
        // connection alone would leave it open: after a timeout the host
        // would go on listing a request nobody here waits for, and could
        // still approve it.
        if let Some(open) = link
            .socket
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            // A socket the peer or `disconnect` already closed is equally
            // closed.
            let _ = open.shutdown(Shutdown::Both);
        }
        let Err(error) = ended else {
            return;
        };
        // A cancel closes the socket itself: that is not news.
        if link.stop.load(Ordering::Relaxed) {
            return;
        }
        let stage = link
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stage;
        if matches!(stage, Stage::Ended(_)) {
            return;
        }
        let started = stage == Stage::Live;
        let reason = lost_text(&error.to_string());
        if link.trust.is_none() {
            let how = if started { End::Lost } else { End::Unreachable };
            end(&link.view, how, reason);
            return;
        }
        // A connection that brought the host's picture starts the schedule
        // again; one that never got that far keeps counting. Being paired is
        // not enough: a host that pairs this laptop and then refuses it the
        // lease (another controller holds it) would otherwise be asked again
        // every second for the whole show.
        attempt = if pictured {
            1
        } else {
            attempt.saturating_add(1)
        };
        let wait = trusted::backoff(attempt);
        if attempt <= 4 || attempt.is_multiple_of(20) {
            log::info!(
                "desktop controller: {} ({}); attempt {attempt} in {} s",
                if started {
                    "connection to the host lost"
                } else {
                    "could not connect to the host"
                },
                error,
                wait.as_secs()
            );
        }
        {
            let mut view = link.view.lock().unwrap_or_else(PoisonError::into_inner);
            view.stage = Stage::Reconnecting(attempt);
            view.ready = false;
            view.note = trusted::retry_text(&reason, attempt);
        }
        if !pause(&link.stop, wait) {
            return;
        }
        // Input typed while the host was away is stale; it is never
        // replayed into the new connection (D275).
        while inputs.try_recv().is_ok() {}
    }
}

/// One connection, until it ends or `stop` is set. `pictured` is set once a
/// frame from the host has arrived on it.
fn control(link: &Link, inputs: &Receiver<Input>, pictured: &mut bool) -> io::Result<()> {
    let Link {
        address,
        code,
        name,
        approval_limit,
        view,
        stop,
        socket,
        credential,
        trust,
    } = link;
    let address = *address;
    let approval_limit = *approval_limit;
    // The proof is tied to this connection and the `Host` it sends.
    let mut handshake = trust.as_ref().map(|plan| {
        trusted::Handshake::new(plan.clone(), strobe_remote::client::host_header(address))
    });
    let began = Instant::now();
    let mut connection = strobe_remote::client::Connection::connect(address)?;
    *socket.lock().unwrap_or_else(PoisonError::into_inner) = Some(connection.interrupter()?);
    connection.send(&ClientMessage::Hello {
        protocol_version: strobe_remote::PROTOCOL_VERSION,
    })?;
    let mut next_send = Instant::now();
    let mut next_ping = Instant::now() + Duration::from_secs(5);
    let mut authenticated = false;
    let mut heard = Instant::now();
    let mut last_frame = Instant::now();
    let mut paused = false;
    let mut batch = Vec::new();
    let mut pending = None;
    // When this controller last sent input: the host's clipboard text is
    // taken only shortly after, so a host cannot write into this computer's
    // clipboard unasked.
    let mut last_input: Option<Instant> = None;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if authenticated && now.duration_since(last_frame) > Duration::from_secs(3) && !paused {
            let mut view = view.lock().unwrap_or_else(PoisonError::into_inner);
            view.ready = false;
            view.note = "Host image paused. Input is disabled until fresh frames arrive.".into();
            connection.send(&ClientMessage::DesktopInput {
                events: vec![Input::Reset {}],
            })?;
            paused = true;
        }
        if now.duration_since(heard) > Duration::from_secs(15) && authenticated {
            // A trusted connection reconnects by itself; only a code needs
            // pairing again.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                if trust.is_some() {
                    "the host stopped responding"
                } else {
                    "Host stopped responding; pair again"
                },
            ));
        }
        let message = connection.receive()?;
        // The approval deadline is checked only after a poll that brought
        // nothing: an approval the host sent just before it is already in
        // the socket and is read, not thrown away by a timeout that races it.
        if message.is_none() && !authenticated && approval_expired(began.elapsed(), approval_limit)
        {
            if trust.is_none() {
                let reconnecting = view
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .reconnecting;
                end(
                    view,
                    End::TimedOut,
                    timed_out_text(approval_limit, reconnecting),
                );
            }
            // A trusted connection needs no approval, so a host that does
            // not answer is one that is not there: the worker retries.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                if trust.is_some() {
                    "the host did not answer the trust file in time"
                } else {
                    "the host did not approve in time"
                },
            ));
        }
        if let Some(message) = message {
            heard = Instant::now();
            match message {
                ServerMessage::Hello {
                    protocol_version, ..
                } => {
                    if strobe_remote::negotiate(protocol_version).is_err() {
                        // No retry will change this.
                        end(
                            view,
                            End::Refused,
                            "The host speaks a different protocol version. Update Strobe on both \
                             computers."
                                .into(),
                        );
                        return Err(io::Error::other("Host uses an incompatible protocol"));
                    }
                    if let Some(handshake) = handshake.as_mut() {
                        // Venue mode: ask for a challenge instead of
                        // sending a code or a token. Sent once, only here.
                        if let Some(start) = handshake.start() {
                            connection.send(&start)?;
                        }
                    } else {
                        let token = credential
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .as_ref()
                            .filter(|remembered| {
                                remembered.address == address && code.trim().is_empty()
                            })
                            .map(|remembered| remembered.token.clone());
                        match token {
                            Some(token) => {
                                connection.send(&ClientMessage::Authenticate { token })?;
                            }
                            None if code.trim().is_empty() => {
                                connection
                                    .send(&ClientMessage::RequestDesktop { name: name.clone() })?;
                            }
                            None => {
                                connection
                                    .send(&ClientMessage::PairDesktop { code: code.clone() })?;
                            }
                        }
                    }
                }
                ServerMessage::Authenticated { .. } => {
                    authenticated = true;
                    last_frame = Instant::now();
                    {
                        let mut view = view.lock().unwrap_or_else(PoisonError::into_inner);
                        view.stage = Stage::Live;
                        view.note = if trust.is_some() {
                            "Trusted by the host. Waiting for its first image.".into()
                        } else {
                            "Approved. Waiting for the host's first image.".into()
                        };
                    }
                    connection.send(&ClientMessage::DesktopOpen {})?;
                }
                ServerMessage::DesktopFrame {
                    revision,
                    size,
                    jpeg,
                } => {
                    let image = decode(size, &jpeg)?;
                    *pictured = true;
                    last_frame = Instant::now();
                    paused = false;
                    let mut view = view.lock().unwrap_or_else(PoisonError::into_inner);
                    view.image = Some(Arc::new(image));
                    view.size = size;
                    view.revision = revision;
                    view.ready = true;
                    view.note =
                        "Full desktop control. Edits and file paths apply on the host.".into();
                }
                ServerMessage::Error { code, message } => {
                    if trust.is_some() {
                        // A trusted connection retries unless the host said
                        // no, and then it says why. A pause for failed
                        // proofs ends by itself, and an error after the
                        // session began is a drop like any other: the
                        // retry finds out whether the host still allows
                        // us.
                        if !authenticated && !trusted::is_transient(code) {
                            end(view, End::Refused, trusted::refusal_text(code, &message));
                        }
                        return Err(io::Error::other(message));
                    }
                    if code == ErrorCode::Unauthorized {
                        *credential.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    }
                    if authenticated {
                        end(view, End::Lost, lost_text(&message));
                    } else {
                        end(view, End::Refused, refusal_text(code, &message));
                    }
                    return Err(io::Error::other(message));
                }
                ServerMessage::DesktopClipboard { text } => {
                    if text.len() > 64 * 1024 {
                        return Err(io::Error::other("Host clipboard exceeds desktop limits"));
                    }
                    if last_input.is_some_and(|sent| sent.elapsed() <= Duration::from_secs(3)) {
                        view.lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .clipboard = Some(text);
                    }
                }
                ServerMessage::Paired { token, .. } => {
                    if token.is_empty() || token.len() > strobe_remote::MAX_SECRET_LEN {
                        return Err(io::Error::other("Invalid host credential"));
                    }
                    *credential.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(Credential { address, token });
                }
                ServerMessage::TrustChallenge { nonce } => {
                    // Signed only if it answers the trust_start this very
                    // connection sent, and only once; otherwise this
                    // controller would sign for whoever asked.
                    let reply = handshake
                        .as_mut()
                        .ok_or(trusted::Unanswered::Unsolicited)
                        .and_then(|handshake| handshake.answer(&nonce));
                    match reply {
                        Ok(proof) => connection.send(&proof)?,
                        Err(why) => {
                            end(
                                view,
                                End::Refused,
                                match why {
                                    trusted::Unanswered::Unsolicited => {
                                        "The host sent a trust challenge this computer did not \
                                         ask for, so it was not answered."
                                    }
                                    trusted::Unanswered::Malformed => {
                                        "The host sent a trust challenge that is not valid, so it \
                                         was not answered."
                                    }
                                }
                                .into(),
                            );
                            return Err(io::Error::other("unanswered trust challenge"));
                        }
                    }
                }
                ServerMessage::State { .. }
                | ServerMessage::CommandResult { .. }
                | ServerMessage::Pong {} => {}
            }
        }
        if authenticated && now >= next_send {
            next_batch(&mut pending, inputs, paused, &mut batch)?;
            if !batch.is_empty() {
                connection.send(&ClientMessage::DesktopInput {
                    events: std::mem::take(&mut batch),
                })?;
                last_input = Some(now);
                next_send = now + Duration::from_millis(150);
            }
        }
        if now >= next_ping {
            connection.send(&ClientMessage::Ping {})?;
            next_ping = now + Duration::from_secs(5);
        }
    }
    Ok(())
}

/// Fills `batch` with the input waiting in `inputs`, at most 32 events and
/// one message's bytes; an event that does not fit waits in `pending`.
/// While `paused`, what is waiting is dropped instead.
///
/// A pointer move that follows another move replaces it, so a run of
/// moves goes as its latest position. A batch leaves every 150 ms, and a
/// 500 or 1000 Hz mouse makes more moves than 32 a batch can carry: sent
/// one each, they filled the queue from the frame loop and ended the
/// connection. Edges and text keep their own slots and their order.
fn next_batch(
    pending: &mut Option<Input>,
    inputs: &Receiver<Input>,
    paused: bool,
    batch: &mut Vec<Input>,
) -> io::Result<()> {
    let mut bytes = 128;
    while batch.len() < 32 {
        let Some(event) = pending.take().or_else(|| inputs.try_recv().ok()) else {
            break;
        };
        if paused {
            continue;
        }
        let length = serde_json::to_vec(&event).map_err(io::Error::other)?.len() + 1;
        if matches!(event, Input::Move { .. })
            && let Some(last @ Input::Move { .. }) = batch.last_mut()
        {
            let replaced = serde_json::to_vec(&*last).map_err(io::Error::other)?.len() + 1;
            if bytes - replaced + length <= strobe_remote::MAX_MESSAGE_BYTES {
                bytes = bytes - replaced + length;
                *last = event;
                continue;
            }
        }
        if bytes + length > strobe_remote::MAX_MESSAGE_BYTES {
            *pending = Some(event);
            break;
        }
        bytes += length;
        batch.push(event);
    }
    Ok(())
}

fn decode(size: [u32; 2], jpeg: &str) -> io::Result<egui::ColorImage> {
    if size.contains(&0)
        || size.iter().any(|edge| *edge > MAX_EDGE)
        || jpeg.len() > strobe_remote::desktop::MAX_FRAME
    {
        return Err(io::Error::other("Host image exceeds desktop limits"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(jpeg)
        .map_err(|_| io::Error::other("Invalid host image encoding"))?;
    if bytes.len() > MAX_JPEG {
        return Err(io::Error::other("Host JPEG exceeds desktop limits"));
    }
    let dimensions = image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Jpeg)
        .into_dimensions()
        .map_err(io::Error::other)?;
    if dimensions != (size[0], size[1]) {
        return Err(io::Error::other("Host image dimensions do not match"));
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(io::Error::other)?.to_rgba8();
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [size[0] as usize, size[1] as usize],
        image.as_raw(),
    ))
}

fn modifiers(value: egui::Modifiers) -> Modifiers {
    Modifiers {
        alt: value.alt,
        ctrl: value.ctrl,
        shift: value.shift,
        mac_cmd: value.mac_cmd,
    }
}
fn egui_modifiers(value: Modifiers) -> egui::Modifiers {
    egui::Modifiers {
        alt: value.alt,
        ctrl: value.ctrl,
        shift: value.shift,
        mac_cmd: value.mac_cmd,
        command: value.ctrl || value.mac_cmd,
    }
}

/// Translate viewer input, excluding the local toolbar and letterbox.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "only pointer, key and committed text are transferable; local viewport and device events are not"
)]
fn outgoing(event: &Event, rect: egui::Rect, focused: bool) -> Vec<Input> {
    // An image squeezed to no width or height (the window dragged down to
    // nothing) puts every point at its near edge rather than at NaN, which
    // the host would refuse by ending the connection (D404).
    let fraction = |offset: f32, extent: f32| {
        let value = offset / extent;
        if extent > 0.0 && value.is_finite() {
            value
        } else {
            0.0
        }
    };
    let point = |pos: Pos2| {
        (
            fraction(pos.x - rect.min.x, rect.width()),
            fraction(pos.y - rect.min.y, rect.height()),
        )
    };
    match event {
        Event::PointerMoved(pos) if rect.contains(*pos) => {
            let (x, y) = point(*pos);
            vec![Input::Move { x, y }]
        }
        Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: mods,
        } if rect.contains(*pos) || (!pressed && focused) => {
            let button = match button {
                egui::PointerButton::Primary => 0,
                egui::PointerButton::Secondary => 1,
                egui::PointerButton::Middle => 2,
                egui::PointerButton::Extra1 | egui::PointerButton::Extra2 => return Vec::new(),
            };
            let (x, y) = point(*pos);
            vec![Input::Button {
                x: x.clamp(0.0, 1.0),
                y: y.clamp(0.0, 1.0),
                button,
                down: *pressed,
                modifiers: modifiers(*mods),
            }]
        }
        Event::Key {
            key,
            pressed,
            modifiers: mods,
            ..
        } if focused => vec![Input::Key {
            key: key.name().into(),
            down: *pressed,
            modifiers: modifiers(*mods),
        }],
        Event::Text(text) if focused => text_chunks(text, false),
        Event::Paste(text) if focused => text_chunks(text, true),
        Event::Copy if focused => vec![Input::Copy {}],
        Event::Cut if focused => vec![Input::Cut {}],
        Event::Ime(egui::ImeEvent::Commit(text)) if focused => text_chunks(text, false),
        Event::MouseWheel {
            unit,
            delta,
            modifiers: mods,
            ..
        } if focused => {
            let factor = match unit {
                egui::MouseWheelUnit::Point => 1.0,
                egui::MouseWheelUnit::Line => 24.0,
                egui::MouseWheelUnit::Page => rect.height(),
            };
            vec![Input::Scroll {
                x: (delta.x * factor).clamp(-4096.0, 4096.0),
                y: (delta.y * factor).clamp(-4096.0, 4096.0),
                modifiers: modifiers(*mods),
            }]
        }
        Event::WindowFocused(false) | Event::PointerGone if focused => vec![Input::Reset {}],
        _ => Vec::new(),
    }
}

fn text_chunks(text: &str, paste: bool) -> Vec<Input> {
    let mut rest = text;
    let mut events = Vec::new();
    while !rest.is_empty() {
        let mut end = rest.len().min(512);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let text = rest[..end].to_owned();
        events.push(if paste {
            Input::Paste { text }
        } else {
            Input::Text { text }
        });
        rest = &rest[end..];
    }
    events
}

/// Track injected edges to release them on disconnection or focus loss.
#[derive(Default)]
pub struct HostInput {
    keys: Vec<(egui::Key, egui::Modifiers)>,
    buttons: Vec<(egui::PointerButton, Pos2, egui::Modifiers)>,
    /// A click with modifiers ended with nothing else held: they are let
    /// go before the next input, in a later pass than the release, so the
    /// click itself still sees them (D405).
    release_modifiers: bool,
}

impl HostInput {
    /// Whether any injected edge needs release when remote access stops.
    pub fn held(&self) -> bool {
        !self.keys.is_empty() || !self.buttons.is_empty()
    }
    /// Convert validated wire inputs to the host's ordinary egui events.
    pub fn translate(&mut self, events: &[Input], size: Vec2, out: &mut Vec<Event>) {
        if std::mem::take(&mut self.release_modifiers) {
            out.push(Event::ModifiersChanged(egui::Modifiers::NONE));
        }
        for input in events {
            match input {
                Input::Move { x, y } => {
                    out.push(Event::PointerMoved(Pos2::new(x * size.x, y * size.y)))
                }
                Input::Button {
                    x,
                    y,
                    button,
                    down,
                    modifiers: mods,
                } => {
                    let button = match button {
                        0 => egui::PointerButton::Primary,
                        1 => egui::PointerButton::Secondary,
                        2 => egui::PointerButton::Middle,
                        _ => continue,
                    };
                    let pos = Pos2::new(x * size.x, y * size.y);
                    let mods = egui_modifiers(*mods);
                    out.push(Event::ModifiersChanged(mods));
                    self.buttons.retain(|(held, _, _)| *held != button);
                    if *down {
                        self.buttons.push((button, pos, mods));
                    }
                    out.push(Event::PointerMoved(pos));
                    out.push(Event::PointerButton {
                        pos,
                        button,
                        pressed: *down,
                        modifiers: mods,
                    });
                    // Moves carry no modifiers, so the ones this click set
                    // would stay held at the host until the next key. With
                    // nothing else held they go before the next input; not
                    // in this pass, where a Shift+click or Ctrl+click reads
                    // them on the release (D405).
                    self.release_modifiers = !*down
                        && mods != egui::Modifiers::NONE
                        && self.buttons.is_empty()
                        && self.keys.is_empty();
                }
                Input::Scroll {
                    x,
                    y,
                    modifiers: mods,
                } => out.push(Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(*x, *y),
                    modifiers: egui_modifiers(*mods),
                }),
                Input::Key {
                    key,
                    down,
                    modifiers: mods,
                } => {
                    if let Some(key) = egui::Key::from_name(key) {
                        let mods = egui_modifiers(*mods);
                        out.push(Event::ModifiersChanged(mods));
                        // A key states the modifiers itself.
                        self.release_modifiers = false;
                        let repeat = *down && self.keys.iter().any(|(held, _)| *held == key);
                        self.keys.retain(|(held, _)| *held != key);
                        if *down {
                            self.keys.push((key, mods));
                        }
                        out.push(Event::Key {
                            key,
                            physical_key: None,
                            pressed: *down,
                            repeat,
                            modifiers: mods,
                        });
                    }
                }
                Input::Text { text } => out.push(Event::Text(text.clone())),
                Input::Copy {} => out.push(Event::Copy),
                Input::Cut {} => out.push(Event::Cut),
                Input::Paste { text } => out.push(Event::Paste(text.clone())),
                Input::Reset {} => {
                    for (key, mods) in self.keys.drain(..) {
                        out.push(Event::Key {
                            key,
                            physical_key: None,
                            pressed: false,
                            repeat: false,
                            modifiers: mods,
                        });
                    }
                    for (button, pos, mods) in self.buttons.drain(..) {
                        out.push(Event::PointerButton {
                            pos,
                            button,
                            pressed: false,
                            modifiers: mods,
                        });
                    }
                    out.push(Event::PointerGone);
                    out.push(Event::ModifiersChanged(egui::Modifiers::default()));
                    self.release_modifiers = false;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_remote_pass_cannot_widen_who_reaches_this_computer() {
        use crate::ui::Action;
        let refused = [
            Action::SetLanRemote(true),
            Action::SetDesktopAccess(true),
            Action::DecideRemotePairing {
                id: 1,
                approve: true,
            },
            Action::ConnectDesktop,
            Action::SetOsc(strobe_io::config::OscSettings {
                enabled: true,
                all_interfaces: true,
                ..strobe_io::config::OscSettings::default()
            }),
            Action::SetImportListener(true),
        ];
        for action in &refused {
            assert!(host_only(action), "host-only: {action:?}");
        }
        let allowed = [
            Action::SetLanRemote(false),
            Action::SetDesktopAccess(false),
            Action::DecideRemotePairing {
                id: 1,
                approve: false,
            },
            Action::RevokeRemoteClient("c00000001".into()),
            Action::RevokeAllRemoteClients,
            Action::SetImportListener(false),
            Action::EditorSave,
            Action::DisconnectDesktop,
            // Ends this computer's own viewer, if any, and opens Settings
            // at the form, whose Connect stays host-only.
            Action::ControlAnotherDesktop,
        ];
        for action in &allowed {
            assert!(!host_only(action), "a remote may: {action:?}");
        }
    }

    #[test]
    fn leaving_a_desktop_never_blocks_the_frame_loop() {
        let mut controller = Controller::default();
        // A private address nothing answers: the worker sits in its
        // connect (or fails at once where the route is refused).
        controller.address = "10.255.255.1:47812".into();
        controller.code = "123456".into();
        controller.connect();
        assert!(controller.active(), "connecting");
        let started = Instant::now();
        controller.disconnect();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "disconnect returned in {:?}, not after the connect timeout",
            started.elapsed()
        );
        assert!(!controller.active(), "the viewer is gone at once");
    }

    #[test]
    fn viewer_keeps_tab_for_the_host_and_has_a_local_keyboard_exit() {
        let mut controller = Controller::default();
        let (sender, receiver) = sync_channel(128);
        controller.sender = Some(sender);
        {
            let mut view = controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            view.stage = Stage::Live;
            view.ready = true;
            view.revision = 1;
            view.size = [32, 32];
            view.image = Some(Arc::new(egui::ColorImage::filled(
                [32, 32],
                egui::Color32::WHITE,
            )));
        }
        let ctx = egui::Context::default();
        let mut actions = Vec::new();
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(640.0, 480.0),
            )),
            focused: true,
            events,
            ..Default::default()
        };
        let click = Event::PointerButton {
            pos: Pos2::new(100.0, 180.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        ctx.run_ui(input(Vec::new()), |ui| controller.show(ui, &mut actions))
            .drop_without_applying_deltas();
        ctx.run_ui(
            input(vec![Event::PointerMoved(Pos2::new(100.0, 180.0)), click]),
            |ui| controller.show(ui, &mut actions),
        )
        .drop_without_applying_deltas();
        assert!(controller.focused, "canvas is focused");
        let tab = Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        ctx.run_ui(input(vec![tab]), |ui| controller.show(ui, &mut actions))
            .drop_without_applying_deltas();
        let sent: Vec<Input> = receiver.try_iter().collect();
        assert!(
            sent.iter()
                .any(|event| matches!(event, Input::Key { key, down: true, .. } if key == "Tab")),
            "Tab reaches the host editor instead of local focus navigation"
        );
        let exit = Event::Key {
            key: egui::Key::F12,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        };
        ctx.run_ui(input(vec![exit]), |ui| controller.show(ui, &mut actions))
            .drop_without_applying_deltas();
        assert!(
            actions.contains(&crate::ui::Action::DisconnectDesktop),
            "keyboard exit remains local"
        );
        assert!(
            !receiver
                .try_iter()
                .any(|event| matches!(event, Input::Key { key, .. } if key == "F12")),
            "exit is not forwarded"
        );
    }

    /// One pass of the viewer in a 640x480 window: the text it drew, the
    /// texture rectangles (with their tint), and the actions it asked for.
    struct Seen {
        text: String,
        images: Vec<egui::Color32>,
        actions: Vec<crate::ui::Action>,
    }

    fn look(controller: &mut Controller, ctx: &egui::Context, size: Vec2) -> Seen {
        let mut actions = Vec::new();
        let mut shapes = Vec::new();
        // An egui area is sized in its first pass and drawn from the second.
        for _ in 0..3 {
            actions.clear();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)),
                    focused: true,
                    ..Default::default()
                },
                |ui| controller.show(ui, &mut actions),
            );
            shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
        }
        fn walk(shape: &egui::Shape, text: &mut String, images: &mut Vec<egui::Color32>) {
            match shape {
                egui::Shape::Text(t) => {
                    text.push_str(t.galley.text());
                    text.push('\n');
                }
                egui::Shape::Rect(rect) if rect.fill_texture_id() != egui::TextureId::default() => {
                    images.push(rect.fill);
                }
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                    images.push(egui::Color32::WHITE);
                }
                egui::Shape::Vec(inner) => {
                    for shape in inner {
                        walk(shape, text, images);
                    }
                }
                egui::Shape::Noop
                | egui::Shape::Circle(_)
                | egui::Shape::Ellipse(_)
                | egui::Shape::LineSegment { .. }
                | egui::Shape::Path(_)
                | egui::Shape::Rect(_)
                | egui::Shape::Mesh(_)
                | egui::Shape::QuadraticBezier(_)
                | egui::Shape::CubicBezier(_)
                | egui::Shape::Callback(_) => {}
            }
        }
        let mut text = String::new();
        let mut images = Vec::new();
        for clipped in &shapes {
            walk(&clipped.shape, &mut text, &mut images);
        }
        Seen {
            text,
            images,
            actions,
        }
    }

    #[test]
    fn a_host_address_may_leave_out_the_port() {
        let at = |a: u8, b: u8, c: u8, d: u8, port: u16| SocketAddr::from(([a, b, c, d], port));
        assert_eq!(parse_host("192.168.1.20"), Ok(at(192, 168, 1, 20, 47812)));
        assert_eq!(
            parse_host("  192.168.1.20 "),
            Ok(at(192, 168, 1, 20, 47812))
        );
        assert_eq!(parse_host("10.0.0.5:9000"), Ok(at(10, 0, 0, 5, 9000)));
        for bad in [
            "",
            "   ",
            "192.168.1.20:",
            "my-laptop",
            "192.168.1",
            "1.2.3.4:x",
        ] {
            assert!(parse_host(bad).is_err(), "{bad:?} is not an address");
        }
    }

    #[test]
    fn waiting_for_approval_shows_the_time_and_ends() {
        let limit = APPROVAL_TIMEOUT;
        assert_eq!(limit, Duration::from_secs(60), "about a minute");
        assert!(
            !approval_expired(Duration::from_secs(59), limit),
            "still waiting"
        );
        assert!(approval_expired(limit, limit), "gives up at the limit");
        assert_eq!(
            waiting_text(Duration::from_secs(12), limit),
            "Waiting 12 s of 60 s"
        );
        assert_eq!(
            waiting_text(Duration::from_secs(75), limit),
            "Waiting 60 s of 60 s",
            "a late frame never reads past the limit"
        );
        assert!(
            wait_hint(false, false).contains("Settings > Remote")
                && wait_hint(false, false).contains("Approve"),
            "{}",
            wait_hint(false, false)
        );
        assert!(
            !wait_hint(true, false).contains("Approve"),
            "a reconnect needs no approval: {}",
            wait_hint(true, false)
        );
        let expired = timed_out_text(limit, false);
        assert!(
            expired.contains("60 s")
                && expired.contains("Approve")
                && expired.contains("Ask again"),
            "says how long, what to press and how to try again: {expired}"
        );
    }

    #[test]
    fn a_refused_reconnect_says_to_ask_again_and_approve_on_the_host() {
        let text = refusal_text(ErrorCode::Unauthorized, "not paired, or revoked");
        assert!(
            text.contains("ask again") && text.contains("Approve"),
            "says to ask again and approve: {text}"
        );
        assert!(
            text.contains("Settings > Remote"),
            "and names where on the host: {text}"
        );
        assert!(
            text.contains("revoked"),
            "and still says why it was refused: {text}"
        );
        let declined = refusal_text(ErrorCode::PairingRejected, "declined on the desktop");
        assert!(
            declined.contains("declined on the desktop") && declined.contains("Approve"),
            "{declined}"
        );
        assert_eq!(
            refusal_text(
                ErrorCode::Unavailable,
                "Enable full desktop control on the host before pairing this desktop"
            ),
            "Enable full desktop control on the host before pairing this desktop",
            "a host message that already says what to do is kept"
        );
        assert_eq!(
            lost_text("Host closed desktop control."),
            "Host closed desktop control.",
            "no doubled full stop"
        );
        assert_eq!(
            lost_text("connection refused"),
            "Connection refused.",
            "a sentence"
        );
    }

    #[test]
    fn the_viewer_asks_for_a_redraw_only_for_something_new() {
        let ctx = egui::Context::default();
        let size = Vec2::new(640.0, 480.0);
        let mut viewer = Controller::staged("192.168.1.20:47812", Staged::Live);
        assert!(viewer.needs_redraw(), "never drawn");
        look(&mut viewer, &ctx, size);
        assert!(!viewer.needs_redraw(), "drawn, and nothing has changed");
        viewer
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .revision = 2;
        assert!(viewer.needs_redraw(), "a new image from the host");
        look(&mut viewer, &ctx, size);
        assert!(!viewer.needs_redraw(), "that image drawn");
        viewer
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .ready = false;
        assert!(viewer.needs_redraw(), "the host's image paused");
        look(&mut viewer, &ctx, size);
        end(&viewer.view, End::Lost, "gone".into());
        assert!(viewer.needs_redraw(), "the connection ended");
    }

    #[test]
    fn a_dropped_viewer_keeps_showing_the_last_image_dimmed() {
        let ctx = egui::Context::default();
        let size = Vec2::new(640.0, 480.0);
        let mut live = Controller::staged("192.168.1.20:47812", Staged::Live);
        let seen = look(&mut live, &ctx, size);
        assert_eq!(
            seen.images,
            vec![egui::Color32::WHITE],
            "live: at full strength"
        );
        assert!(seen.actions.is_empty(), "nothing asked for");

        let mut dropped = Controller::staged("192.168.1.20:47812", Staged::Dropped);
        let seen = look(&mut dropped, &ctx, size);
        assert_eq!(seen.images.len(), 1, "the last image is still drawn");
        assert!(
            seen.images.iter().all(|tint| tint.r() < 128),
            "and dimmed: {:?}",
            seen.images
        );
        assert!(
            seen.text.contains("Connection lost")
                && seen.text.contains("Host closed desktop control")
                && seen.text.contains("last image")
                && seen.text.contains("read-only"),
            "and the text promises only what is drawn: {}",
            seen.text
        );
        assert!(
            seen.text.contains("Reconnect"),
            "a held pairing offers Reconnect: {}",
            seen.text
        );

        let mut paused = Controller::staged("192.168.1.20:47812", Staged::Paused);
        let seen = look(&mut paused, &ctx, size);
        assert!(
            seen.images.iter().all(|tint| tint.r() < 128) && !seen.images.is_empty(),
            "a paused host image is dimmed too: {:?}",
            seen.images
        );
    }

    #[test]
    fn an_ended_viewer_without_an_image_does_not_promise_one() {
        let ctx = egui::Context::default();
        let size = Vec2::new(640.0, 480.0);
        let mut unreachable = Controller::staged("192.168.1.20:47812", Staged::Unreachable);
        let seen = look(&mut unreachable, &ctx, size);
        assert!(seen.images.is_empty(), "nothing was ever received");
        assert!(
            seen.text.contains("Could not connect") && !seen.text.contains("last image"),
            "{}",
            seen.text
        );
    }

    #[test]
    fn a_waiting_viewer_shows_elapsed_time_a_hint_and_a_way_out() {
        let ctx = egui::Context::default();
        let size = Vec2::new(640.0, 480.0);
        let mut waiting = Controller::staged("192.168.1.20:47812", Staged::Waiting(12));
        let seen = look(&mut waiting, &ctx, size);
        assert!(
            seen.text.contains("Waiting 12 s of 60 s")
                || seen.text.contains("Waiting 13 s of 60 s"),
            "{}",
            seen.text
        );
        assert!(
            seen.text.contains("Approve") && seen.text.contains("Settings > Remote"),
            "the hint says what to do on the host: {}",
            seen.text
        );
        assert!(
            seen.text.contains("Cancel") && !seen.text.contains("Return to local Strobe"),
            "Cancel while it connects: {}",
            seen.text
        );

        let mut reconnecting = Controller::staged("192.168.1.20:47812", Staged::Reconnecting(3));
        let seen = look(&mut reconnecting, &ctx, size);
        assert!(
            seen.text.contains("needs no approval"),
            "a reconnect says so: {}",
            seen.text
        );
    }

    #[test]
    fn a_failed_connection_offers_reconnect_only_when_a_pairing_is_held() {
        let ctx = egui::Context::default();
        let size = Vec2::new(640.0, 480.0);
        for (how, held) in [
            (Staged::TimedOut, false),
            (Staged::Refused, false),
            (Staged::Unreachable, false),
            (Staged::TimedOutReconnect, true),
            (Staged::Dropped, true),
        ] {
            let mut controller = Controller::staged("192.168.1.20:47812", how);
            let seen = look(&mut controller, &ctx, size);
            assert_eq!(
                seen.text.contains("Reconnect"),
                held,
                "{how:?}: {}",
                seen.text
            );
            assert_eq!(
                seen.text.contains("Ask again"),
                !held,
                "{how:?}: {}",
                seen.text
            );
        }
    }

    #[test]
    fn connecting_and_leaving_leave_the_form_with_a_true_note() {
        let mut controller = Controller::default();
        controller.address = "not an address".into();
        controller.connect();
        assert!(
            controller.note().contains("192.168.1.20"),
            "a bad address shows an example: {}",
            controller.note()
        );
        assert!(!controller.active(), "nothing started");
        controller.address = "10.255.255.1".into();
        controller.connect();
        assert!(
            controller.active(),
            "no code and no saved pairing asks the host to approve (D468)"
        );
        assert_eq!(
            controller.address, "10.255.255.1:47812",
            "the form shows the address as used"
        );
        controller.disconnect();
        assert_eq!(
            controller.note(),
            "Connection cancelled.",
            "not the stale 'Connecting' line"
        );
    }

    #[test]
    fn an_unapproved_connection_gives_up_and_says_so() -> Result<()> {
        use strobe_remote::{
            bridge,
            clients::Clients,
            server::{Limits, Server, ServerConfig},
        };
        let (remote, app) = bridge::bridge(4);
        app.desktop.enable(true);
        let mut config = ServerConfig::new(std::net::IpAddr::from([127, 0, 0, 1]), 0);
        config.limits = Limits {
            tick: Duration::from_millis(5),
            ..Limits::default()
        };
        let server = Server::start(config, remote, Clients::in_memory())?;
        let mut controller = Controller::default();
        controller.approval_limit = Duration::from_millis(400);
        controller.address = server.addr().to_string();
        controller.code = server.open_pairing()?;
        controller.connect();
        eventually(|| !server.pairing_requests().is_empty())?;
        let stage = |controller: &Controller| {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stage
        };
        assert_eq!(
            stage(&controller),
            Stage::Connecting,
            "waiting while the host has not decided"
        );
        // Nobody approves.
        eventually(|| stage(&controller) == Stage::Ended(End::TimedOut))?;
        assert!(
            controller.note().contains("did not approve"),
            "{}",
            controller.note()
        );
        assert!(controller.active(), "the viewer stays up to say so");
        // The worker closed its socket, so the host withdrew the request
        // rather than keep it for an Approve nobody is waiting on.
        eventually(|| server.pairing_requests().is_empty() && server.connections().is_empty())?;
        controller.disconnect();
        server.stop();
        Ok(())
    }

    #[test]
    fn a_desktop_with_no_code_is_put_to_the_host_by_name_and_approved() -> Result<()> {
        use strobe_remote::{
            bridge,
            clients::Clients,
            server::{Limits, Server, ServerConfig},
        };
        let (remote, app) = bridge::bridge(4);
        app.desktop.enable(true);
        let mut config = ServerConfig::new(std::net::IpAddr::from([127, 0, 0, 1]), 0);
        config.limits = Limits {
            tick: Duration::from_millis(5),
            ..Limits::default()
        };
        let server = Server::start(config, remote, Clients::in_memory())?;
        let mut controller = Controller::default();
        controller.controller_id = "foh-laptop-1a2b3c4d".into();
        controller.address = server.addr().to_string();
        // No code: what Connect on a Strobe found on the network does.
        controller.connect();
        eventually(|| !server.pairing_requests().is_empty())?;
        let request = server
            .pairing_requests()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no request"))?;
        assert_eq!(
            request.label, "Full desktop: foh-laptop-1a2b3c4d at 127.0.0.1",
            "the host sees this desktop's name and address"
        );
        assert!(server.decide(request.id, true), "approved on the host");
        eventually(|| {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stage
                == Stage::Live
        })?;
        assert!(
            controller.remembers(server.addr()),
            "the approval left a pairing for Reconnect"
        );
        controller.disconnect();
        server.stop();
        Ok(())
    }

    #[test]
    fn input_that_cannot_reach_the_host_ends_the_connection() {
        let mut controller = Controller::staged("192.168.1.20:47812", Staged::Live);
        let (sender, receiver) = sync_channel(1);
        drop(receiver);
        controller.sender = Some(sender);
        controller.send(Input::Reset {});
        let view = controller
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        assert_eq!(
            view.stage,
            Stage::Ended(End::Lost),
            "ended, not left waiting for a frame that cannot come"
        );
        assert!(!view.ready, "input is off");
    }

    #[test]
    fn a_blank_code_after_the_host_forgot_us_asks_for_a_new_one() -> Result<()> {
        use strobe_remote::{
            bridge,
            clients::Clients,
            server::{Limits, Server, ServerConfig},
        };
        let (remote, app) = bridge::bridge(4);
        app.desktop.enable(true);
        let clients = Clients::in_memory();
        let mut config = ServerConfig::new(std::net::IpAddr::from([127, 0, 0, 1]), 0);
        config.limits = Limits {
            tick: Duration::from_millis(5),
            ..Limits::default()
        };
        let server = Server::start(config, remote, clients.clone())?;
        let mut controller = Controller::default();
        controller.address = server.addr().to_string();
        controller.code = server.open_pairing()?;
        controller.connect();
        eventually(|| !server.pairing_requests().is_empty())?;
        let request = server
            .pairing_requests()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("approval request missing"))?;
        assert!(server.decide(request.id, true), "approved on the host");
        eventually(|| app.desktop.active())?;
        controller.disconnect();
        eventually(|| !app.desktop.active())?;
        assert!(
            controller.remembers(server.addr()),
            "the controller holds its rotated pairing"
        );
        // The host revokes it, or restarts and loses it.
        server.revoke_all();
        controller.connect();
        eventually(|| {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .stage
                == Stage::Ended(End::Refused)
        })?;
        let note = controller.note();
        assert!(
            note.contains("ask again") && note.contains("Approve"),
            "says to ask again and approve on the host: {note}"
        );
        assert!(
            !controller.remembers(server.addr()),
            "the refused pairing is forgotten, so Reconnect is not offered"
        );
        controller.disconnect();
        server.stop();
        Ok(())
    }

    fn eventually(mut check: impl FnMut() -> bool) -> Result<()> {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if check() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        anyhow::bail!("desktop test timed out")
    }

    #[test]
    fn native_controller_pairs_decodes_edits_host_files_and_reconnects_in_memory() -> Result<()> {
        use strobe_remote::{
            bridge,
            clients::Clients,
            server::{Limits, Server, ServerConfig},
        };
        let (remote, app) = bridge::bridge(4);
        app.desktop.enable(true);
        let clients = Clients::in_memory();
        let mut config = ServerConfig::new(std::net::IpAddr::from([127, 0, 0, 1]), 0);
        config.limits = Limits {
            tick: Duration::from_millis(5),
            ..Limits::default()
        };
        let server = Server::start(config, remote, clients.clone())?;
        let mut controller = Controller::default();
        controller.address = server.addr().to_string();
        controller.code = server.open_pairing()?;
        controller.connect();
        eventually(|| !server.pairing_requests().is_empty())?;
        let request = server
            .pairing_requests()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("approval request missing"))?;
        assert!(server.decide(request.id, true), "approved on the host");
        eventually(|| app.desktop.active())?;
        let jpeg = strobe_remote::client::encode_image(
            &encode(
                [32, 32],
                &mut vec![200; 32 * 32 * 4],
                &mut Ladder::default(),
            )?
            .jpeg,
        );
        app.desktop.publish([32, 32], jpeg.clone());
        eventually(|| {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ready
        })?;
        let image = controller
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .image
            .take()
            .ok_or_else(|| anyhow::anyhow!("decoded host image missing"))?;
        assert_eq!(
            image.size,
            [32, 32],
            "native worker decoded the host capture"
        );
        controller.send(Input::Key {
            key: "A".into(),
            down: true,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });
        controller.send(Input::Key {
            key: "A".into(),
            down: false,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });
        controller.send(Input::Paste {
            text: "// edited remotely\nvec4 renderMain() { return vec4(0.5); }".into(),
        });
        let mut wire = Vec::new();
        eventually(|| {
            app.desktop.drain(&mut wire);
            wire.len() >= 3
        })?;

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let folder =
            std::env::temp_dir().join(format!("strobe-desktop-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&folder)?;
        struct Scratch(std::path::PathBuf);
        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let scratch = Scratch(folder);
        let scene = scratch.0.join("scene");
        std::fs::create_dir(&scene)?;
        std::fs::write(scene.join("main.glsl"), "original shader")?;
        let mut doc = crate::editor::Doc::open(&scene, "scene")?;
        let ctx = egui::Context::default();
        let field = egui::Id::new("host code");
        let size = Vec2::new(800.0, 600.0);
        let mut host = HostInput::default();
        let input = |events| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, size)),
            focused: true,
            events,
            ..egui::RawInput::default()
        };
        ctx.run_ui(input(Vec::new()), |ui| {
            ui.add(egui::TextEdit::multiline(&mut doc.tabs[0].text).id(field));
        })
        .drop_without_applying_deltas();
        ctx.memory_mut(|memory| memory.request_focus(field));
        let mut events = Vec::new();
        host.translate(&wire, size, &mut events);
        ctx.run_ui(input(events), |ui| {
            ui.add(egui::TextEdit::multiline(&mut doc.tabs[0].text).id(field));
        })
        .drop_without_applying_deltas();
        assert_eq!(
            doc.tabs[0].text, "// edited remotely\nvec4 renderMain() { return vec4(0.5); }",
            "network input edited the host widget"
        );
        assert_eq!(
            doc.save(&scratch.0.join("backups"))?,
            1,
            "same editor save and backup path"
        );
        assert_eq!(
            std::fs::read_to_string(scene.join("main.glsl"))?,
            doc.tabs[0].text,
            "edit saved on host filesystem"
        );
        controller.disconnect();
        eventually(|| !app.desktop.active())?;
        controller.connect();
        eventually(|| app.desktop.active())?;
        app.desktop.publish([32, 32], jpeg);
        eventually(|| {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ready
        })?;
        assert_eq!(
            clients.list().len(),
            1,
            "reconnect authenticates and rotates the same in-memory credential"
        );
        controller.disconnect();
        server.stop();
        Ok(())
    }

    // ------------------------------------------------ trusted connections

    use std::path::{Path, PathBuf};
    use strobe_remote::bridge::AppEnd;
    use strobe_remote::server::Server;
    use strobe_remote::trust::{self as venue_trust, TrustSecret};

    /// A scratch folder that deletes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Result<Self> {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "strobe-trusted-{tag}-{}-{stamp}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A host on loopback that trusts `secret`, with full desktop access on
    /// and quick connection polling. Port 0 picks a free one; a restart
    /// names the port it had, and tries again if the system has not yet
    /// let go of it.
    fn trusted_host(secret: &TrustSecret, port: u16) -> Result<(Server, AppEnd)> {
        use strobe_remote::{
            bridge,
            clients::Clients,
            server::{Limits, ServerConfig},
        };
        let mut last = None;
        for _ in 0..50 {
            let (remote, app) = bridge::bridge(4);
            app.desktop.enable(true);
            let mut config = ServerConfig::new(IpAddr::from([127, 0, 0, 1]), port);
            config.limits = Limits {
                tick: Duration::from_millis(5),
                ..Limits::default()
            };
            config.trust = Some(secret.clone());
            match Server::start(config, remote, Clients::in_memory()) {
                Ok(server) => return Ok((server, app)),
                Err(error) => last = Some(error),
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(anyhow::anyhow!("the host did not start: {last:?}"))
    }

    /// A controller set up as the form leaves it with a trust file chosen.
    fn trusted_controller(file: &Path, address: SocketAddr) -> Controller {
        let mut controller = Controller::default();
        controller.controller_id = "t480-1a2b3c4d".into();
        controller.use_trust = true;
        controller.trust_path = file.display().to_string();
        controller.address = address.to_string();
        controller
    }

    fn stage_of(controller: &Controller) -> Stage {
        controller
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .stage
    }

    fn within(limit: Duration, mut check: impl FnMut() -> bool) -> Result<()> {
        let until = Instant::now() + limit;
        while Instant::now() < until {
            if check() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        anyhow::bail!("desktop test timed out after {limit:?}")
    }

    fn a_frame() -> Result<String> {
        Ok(strobe_remote::client::encode_image(
            &encode(
                [32, 32],
                &mut vec![200; 32 * 32 * 4],
                &mut Ladder::default(),
            )?
            .jpeg,
        ))
    }

    #[test]
    fn a_trusted_controller_connects_with_no_approval_and_keeps_the_secret_to_itself() -> Result<()>
    {
        let scratch = Scratch::new("connect")?;
        let file = scratch.0.join("venue.trust");
        let secret = venue_trust::create_file(&file)?;
        let (server, app) = trusted_host(&secret, 0)?;
        let mut controller = trusted_controller(&file, server.addr());
        controller.connect();
        within(Duration::from_secs(5), || app.desktop.active())?;
        assert!(
            server.pairing_requests().is_empty(),
            "nobody was asked to approve anything"
        );
        app.desktop.publish([32, 32], a_frame()?);
        within(Duration::from_secs(5), || {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ready
        })?;
        assert_eq!(stage_of(&controller), Stage::Live, "live");
        let listed = server.clients().list();
        assert!(
            listed
                .iter()
                .any(|client| client.label.starts_with(&format!(
                    "{}t480-1a2b3c4d",
                    strobe_remote::desktop::TRUSTED_LABEL_PREFIX
                ))),
            "the host lists this laptop as trusted by its controller id: {listed:?}"
        );
        assert!(
            !controller.remembers(server.addr()),
            "no token was issued, so nothing but the file can get back in"
        );
        // Neither the viewer's text nor anything printable holds the secret.
        let text = std::fs::read_to_string(&file)?;
        let hex = text
            .lines()
            .find_map(|line| line.strip_prefix("secret "))
            .ok_or_else(|| anyhow::anyhow!("the file has a secret line"))?;
        assert!(!controller.note().contains(hex), "the secret is not shown");
        controller.disconnect();
        server.stop();
        Ok(())
    }

    #[test]
    fn the_wrong_trust_file_is_refused_once_and_says_why_without_retrying() -> Result<()> {
        let scratch = Scratch::new("wrong")?;
        let host_file = scratch.0.join("host.trust");
        let other_file = scratch.0.join("other.trust");
        let secret = venue_trust::create_file(&host_file)?;
        venue_trust::create_file(&other_file)?;
        let (server, app) = trusted_host(&secret, 0)?;
        let mut controller = trusted_controller(&other_file, server.addr());
        controller.connect();
        within(Duration::from_secs(5), || {
            stage_of(&controller) == Stage::Ended(End::Refused)
        })?;
        let note = controller.note();
        assert!(
            note.contains("does not accept this trust file")
                && note.contains("--make-remote-trust"),
            "{note}"
        );
        // Longer than the first backoff (1 s): a retry loop would have shown.
        std::thread::sleep(trusted::backoff(1) + Duration::from_millis(600));
        assert_eq!(
            stage_of(&controller),
            Stage::Ended(End::Refused),
            "still refused, not Reconnecting"
        );
        assert!(
            controller
                .worker
                .as_ref()
                .is_some_and(JoinHandle::is_finished),
            "the worker has ended: nothing is trying again"
        );
        assert!(!app.desktop.active(), "and the host gave no control");
        assert!(server.connections().is_empty(), "no socket is left open");
        controller.disconnect();
        server.stop();
        Ok(())
    }

    #[test]
    fn a_trusted_controller_finds_a_restarted_host_by_itself_and_takes_its_lease_back() -> Result<()>
    {
        let scratch = Scratch::new("restart")?;
        let file = scratch.0.join("venue.trust");
        let secret = venue_trust::create_file(&file)?;
        let (server, app) = trusted_host(&secret, 0)?;
        let address = server.addr();
        let mut controller = trusted_controller(&file, address);
        controller.connect();
        within(Duration::from_secs(5), || app.desktop.active())?;
        app.desktop.publish([32, 32], a_frame()?);
        within(Duration::from_secs(5), || {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ready
        })?;

        // The host goes away: the show machine reboots.
        server.stop();
        drop(app);
        within(Duration::from_secs(5), || {
            matches!(stage_of(&controller), Stage::Reconnecting(_))
        })?;
        {
            let view = controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            assert!(view.image.is_some(), "the last image is kept");
            assert!(!view.ready, "and read-only");
            assert!(
                view.note.contains("the wait grows to 15 s"),
                "{}",
                view.note
            );
        }
        // It keeps trying while the host is down, counting its attempts.
        within(
            Duration::from_secs(8),
            || matches!(stage_of(&controller), Stage::Reconnecting(n) if n >= 2),
        )?;

        // The host comes back on the same port.
        let (server, app) = trusted_host(&secret, address.port())?;
        within(Duration::from_secs(12), || {
            stage_of(&controller) == Stage::Live && app.desktop.active()
        })?;
        assert!(
            server.pairing_requests().is_empty(),
            "no approval was asked of anyone"
        );
        app.desktop.publish([32, 32], a_frame()?);
        within(Duration::from_secs(5), || {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ready
        })?;
        controller.disconnect();
        within(Duration::from_secs(5), || !app.desktop.active())?;
        server.stop();
        Ok(())
    }

    #[test]
    fn disconnect_stops_the_retrying() -> Result<()> {
        let scratch = Scratch::new("stop")?;
        let file = scratch.0.join("venue.trust");
        let secret = venue_trust::create_file(&file)?;
        let (server, app) = trusted_host(&secret, 0)?;
        let address = server.addr();
        let mut controller = trusted_controller(&file, address);
        controller.connect();
        within(Duration::from_secs(5), || app.desktop.active())?;
        server.stop();
        drop(app);
        within(Duration::from_secs(5), || {
            matches!(stage_of(&controller), Stage::Reconnecting(_))
        })?;

        controller.disconnect();
        assert!(!controller.active(), "the viewer is gone");
        // The host comes back; a controller still trying would be in by now
        // (its first retry is after 1 s).
        let (server, app) = trusted_host(&secret, address.port())?;
        std::thread::sleep(trusted::backoff(1) + trusted::backoff(2));
        assert!(!app.desktop.active(), "nobody took the lease");
        assert!(server.connections().is_empty(), "nobody is connected");
        server.stop();
        Ok(())
    }

    #[test]
    fn a_trusted_controller_shut_out_of_the_lease_backs_off_rather_than_hammering() -> Result<()> {
        // Two laptops with the same file but different ids: the host pairs
        // the second, then refuses it the lease (one desktop at a time) and
        // it is dropped. Being paired is not being live; it must not restart
        // the schedule at 1 s every time, or it would pair and drop once a
        // second for the whole show.
        let scratch = Scratch::new("lease")?;
        let file = scratch.0.join("venue.trust");
        let secret = venue_trust::create_file(&file)?;
        let (server, app) = trusted_host(&secret, 0)?;
        let mut first = trusted_controller(&file, server.addr());
        first.connect();
        within(Duration::from_secs(5), || app.desktop.active())?;
        let mut second = trusted_controller(&file, server.addr());
        second.controller_id = "other-laptop-00000000".into();
        second.connect();
        within(
            Duration::from_secs(10),
            || matches!(stage_of(&second), Stage::Reconnecting(n) if n >= 3),
        )
        .map_err(|error| {
            anyhow::anyhow!(
                "{error}: the attempts should count up (1 s, 2 s, ...), not start again at 1; \
                 stuck at {:?}",
                stage_of(&second)
            )
        })?;
        assert!(app.desktop.active(), "the first laptop keeps the lease");
        second.disconnect();
        first.disconnect();
        server.stop();
        Ok(())
    }

    #[test]
    fn the_kill_switch_is_retried_once_and_then_refused() -> Result<()> {
        let scratch = Scratch::new("kill")?;
        let file = scratch.0.join("venue.trust");
        let secret = venue_trust::create_file(&file)?;
        let (server, app) = trusted_host(&secret, 0)?;
        let mut controller = trusted_controller(&file, server.addr());
        controller.connect();
        within(Duration::from_secs(5), || app.desktop.active())?;
        app.desktop.publish([32, 32], a_frame()?);
        within(Duration::from_secs(5), || {
            controller
                .view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .ready
        })?;
        // What Lan::set_desktop(false) does at the host.
        app.desktop.enable(false);
        server.revoke_desktop();
        within(Duration::from_secs(5), || {
            stage_of(&controller) == Stage::Ended(End::Refused)
        })?;
        let note = controller.note();
        assert!(
            note.contains("does not offer trusted pairing right now"),
            "{note}"
        );
        assert!(!app.desktop.active(), "and nothing holds the lease");
        controller.disconnect();
        server.stop();
        Ok(())
    }

    /// A host that sends a trust challenge nobody asked for, and records the
    /// type of every message the controller sends it.
    fn rogue_host() -> Result<(SocketAddr, JoinHandle<Vec<String>>)> {
        use sha1::{Digest as _, Sha1};
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let address = listener.local_addr()?;
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            let Ok((mut stream, _)) = listener.accept() else {
                return seen;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && head.len() < 4096 {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    Ok(_) => return seen,
                    Err(_) => {}
                }
            }
            let head = String::from_utf8_lossy(&head).to_string();
            let Some(key) = head.lines().find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("sec-websocket-key"))
                    .map(|(_, value)| value.trim().to_string())
            }) else {
                return seen;
            };
            let mut hasher = Sha1::new();
            hasher.update(key.as_bytes());
            hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
            let accept = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
            let reply = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            let send = |stream: &mut TcpStream, message: &ServerMessage| -> bool {
                let Ok(text) = serde_json::to_string(message) else {
                    return false;
                };
                let Ok(length) = u8::try_from(text.len()) else {
                    return false;
                };
                if length > 125 {
                    return false;
                }
                let mut frame = vec![0x81, length];
                frame.extend_from_slice(text.as_bytes());
                stream.write_all(&frame).is_ok()
            };
            if stream.write_all(reply.as_bytes()).is_err()
                || !send(
                    &mut stream,
                    &ServerMessage::Hello {
                        protocol_version: strobe_remote::PROTOCOL_VERSION,
                        paired: false,
                    },
                )
            {
                return seen;
            }
            let mut buffered: Vec<u8> = Vec::new();
            let mut challenged = false;
            let until = Instant::now() + Duration::from_secs(3);
            let mut chunk = [0_u8; 512];
            while Instant::now() < until {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => buffered.extend_from_slice(&chunk[..n]),
                    Err(_) => {}
                }
                // Masked client frames, each a short text message.
                while buffered.len() >= 2 {
                    let length = usize::from(buffered[1] & 0x7f);
                    if length >= 126 || buffered.len() < 6 + length {
                        break;
                    }
                    let mask = [buffered[2], buffered[3], buffered[4], buffered[5]];
                    let text: Vec<u8> = buffered[6..6 + length]
                        .iter()
                        .zip(mask.iter().cycle())
                        .map(|(byte, key)| byte ^ key)
                        .collect();
                    buffered.drain(..6 + length);
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&text)
                        && let Some(kind) = value["type"].as_str()
                    {
                        seen.push(kind.to_string());
                    }
                    if !challenged {
                        challenged = true;
                        // Unprompted: this controller never sent trust_start.
                        let _ = send(
                            &mut stream,
                            &ServerMessage::TrustChallenge {
                                nonce: "ab".repeat(32),
                            },
                        );
                    }
                }
            }
            seen
        });
        Ok((address, handle))
    }

    #[test]
    fn a_challenge_nobody_asked_for_is_not_signed_and_ends_the_connection() -> Result<()> {
        let (address, host) = rogue_host()?;
        // A controller pairing by code has a trust file nowhere near it, and
        // a trusted one has not sent trust_start on this path: either way a
        // volunteered challenge must not be answered. The code controller
        // is the plainest case.
        let mut controller = Controller::default();
        controller.address = address.to_string();
        controller.code = "123456".into();
        controller.connect();
        within(Duration::from_secs(5), || {
            stage_of(&controller) == Stage::Ended(End::Refused)
        })?;
        assert!(
            controller.note().contains("did not ask for"),
            "{}",
            controller.note()
        );
        controller.disconnect();
        let seen = host
            .join()
            .map_err(|_| anyhow::anyhow!("the rogue host panicked"))?;
        assert_eq!(
            seen,
            ["hello", "pair_desktop"],
            "the controller answered the hello and nothing more: no proof was made"
        );
        Ok(())
    }

    #[test]
    fn the_viewer_says_reconnecting_with_the_attempt_and_keeps_the_last_image() {
        let ctx = egui::Context::default();
        let size = Vec2::new(640.0, 480.0);
        let mut retrying = Controller::staged("192.168.1.20:47812", Staged::Retrying(3));
        let seen = look(&mut retrying, &ctx, size);
        assert!(
            seen.text.contains("Reconnecting\u{2026} (attempt 3)"),
            "{}",
            seen.text
        );
        assert!(
            seen.text.contains("Disconnect") && seen.text.contains("last image"),
            "a way out, and only the promise the picture keeps: {}",
            seen.text
        );
        assert_eq!(seen.images.len(), 1, "the last image is still drawn");
        assert!(
            seen.images.iter().all(|tint| tint.r() < 128),
            "dimmed: {:?}",
            seen.images
        );
        assert!(
            !seen.text.contains("pairing code"),
            "a trusted connection never asks for a code: {}",
            seen.text
        );
    }

    #[test]
    fn a_refused_trust_file_offers_reconnect_not_a_new_code() {
        let ctx = egui::Context::default();
        let size = Vec2::new(640.0, 480.0);
        let mut refused = Controller::staged("192.168.1.20:47812", Staged::TrustRefused);
        let seen = look(&mut refused, &ctx, size);
        assert!(
            seen.text.contains("does not accept this trust file")
                && seen.text.contains("Reconnect")
                && !seen.text.contains("Enter a new pairing code"),
            "{}",
            seen.text
        );
    }

    #[test]
    fn a_trust_file_typed_as_a_relative_path_is_shown_and_remembered_whole() -> Result<()> {
        // Typed as `venue.trust` in a run started from its folder, it was
        // remembered as that and looked for in whatever folder the next
        // launch started in; the form's failure did not say where (D463).
        let host = SocketAddr::from(([192, 168, 1, 50], 47812));
        let mut form = trusted_controller(Path::new("  \"no-such-venue.trust\" "), host);
        form.connect();
        let whole = std::env::current_dir()?
            .join("no-such-venue.trust")
            .display()
            .to_string();
        assert!(!form.active(), "a missing file connects nothing");
        assert_eq!(
            form.trust_path, whole,
            "the form shows the file it looked for"
        );
        let note = form
            .view
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .note
            .clone();
        assert!(note.contains(&whole), "the failure names it: {note}");
        let mut saved = crate::prefs::RemotePref::default();
        form.remembered(&mut saved);
        assert_eq!(
            saved.trust_file, whole,
            "remembered whole, for a launch from another folder"
        );
        Ok(())
    }

    #[test]
    fn the_form_remembers_a_path_and_a_launch_with_control_decides_the_first_action() {
        use crate::prefs::RemotePref;
        use crate::ui::Action;
        let remembered = RemotePref {
            trust_file: "C:/Users/me/venue.trust".into(),
            use_trust: true,
            controller_id: "t480-1a2b3c4d".into(),
        };
        let host = SocketAddr::from(([192, 168, 1, 50], 47812));

        let (form, first) = Controller::at_launch(&remembered, None, None);
        assert_eq!(first, None, "no --control, no first action");
        assert!(
            form.use_trust && form.trust_path == "C:/Users/me/venue.trust",
            "the form comes back as it was left"
        );
        assert_eq!(form.controller_id, "t480-1a2b3c4d", "the same laptop id");

        let (viewer, first) = Controller::at_launch(
            &remembered,
            Some(host),
            Some(Path::new("D:/other/show.trust")),
        );
        assert_eq!(
            first,
            Some(Action::ConnectDesktop),
            "straight into the viewer"
        );
        assert!(
            viewer.use_trust
                && viewer.trust_path == "D:/other/show.trust"
                && viewer.address == "192.168.1.50:47812",
            "with the file named on the command line"
        );

        let (prefilled, first) = Controller::at_launch(&remembered, Some(host), None);
        assert_eq!(first, Some(Action::ControlAnotherDesktop), "the form opens");
        assert!(
            !prefilled.use_trust && prefilled.address == "192.168.1.50:47812",
            "address filled in, code pairing, even with a remembered file"
        );

        let mut saved = RemotePref::default();
        let mut typed = Controller::default();
        typed.use_trust = true;
        typed.trust_path = "  \"C:/Users/me/venue.trust\"  ".into();
        typed.remembered(&mut saved);
        assert_eq!(
            (saved.trust_file.as_str(), saved.use_trust),
            ("C:/Users/me/venue.trust", true),
            "the path as a clean path, nothing else about the file"
        );
    }

    #[test]
    fn unicode_paste_is_not_truncated_and_toolbar_clicks_stay_local() {
        let text = "界".repeat(21_000);
        let events = text_chunks(&text, true);
        let restored: String = events
            .iter()
            .filter_map(|event| {
                if let Input::Paste { text } = event {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(restored, text, "all Unicode chunks survive");
        assert!(
            events
                .iter()
                .all(|event| strobe_remote::desktop::valid_inputs(std::slice::from_ref(event))),
            "each chunk is a valid bounded input"
        );
        let event = Event::PointerButton {
            pos: Pos2::new(20.0, 10.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(
            outgoing(
                &event,
                egui::Rect::from_min_max(Pos2::new(0.0, 70.0), Pos2::new(640.0, 480.0)),
                true
            )
            .is_empty(),
            "local disconnect toolbar is never sent to host"
        );
    }

    #[test]
    fn a_viewer_image_with_no_width_or_height_sends_only_valid_input() {
        // The controller's window dragged down to nothing: egui fits the
        // image to no room at all. A pointer on its edge, or a release
        // after focus, must not become NaN, which the host refuses by
        // ending the connection (D404).
        let at = Pos2::new(40.0, 30.0);
        let flat = egui::Rect::from_min_max(at, Pos2::new(40.0, 300.0));
        let thin = egui::Rect::from_min_max(at, Pos2::new(400.0, 30.0));
        let point = egui::Rect::from_min_max(at, at);
        let events = [
            Event::PointerMoved(at),
            Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            Event::PointerButton {
                pos: Pos2::new(10.0, 10.0),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        for rect in [flat, thin, point] {
            for event in &events {
                let sent = outgoing(event, rect, true);
                assert!(
                    sent.is_empty() || strobe_remote::desktop::valid_inputs(&sent),
                    "{event:?} over {rect:?} sends valid input: {sent:?}"
                );
            }
        }
        let release = events.get(2).map(|event| outgoing(event, point, true));
        assert!(
            matches!(
                release.as_deref(),
                Some([Input::Button { down: false, .. }])
            ),
            "a release is still sent, so the host does not keep the button down: {release:?}"
        );
    }

    #[test]
    fn a_control_window_driven_at_the_stream_rate_captures_on_every_frame() {
        // While a lease is held, a covered control window is run by the event
        // loop every STREAM_PERIOD (D290), apart as the loop schedules them.
        // The capture reads its clock after make_current, which can wait up
        // to a vertical blank for the output, so one frame's reading can be
        // a few ms later than the next's. That next frame was then not due,
        // by those few ms, and the controller got an image every 400 ms.
        let mut driven = Capture::default();
        let mut in_view = Capture::default();
        let start = Instant::now();
        // What make_current waited in each frame, in ms.
        let waits = [12_u64, 1, 15, 0, 9, 2, 16, 3];
        let captured = (0_u32..)
            .zip(waits)
            .filter(|(n, wait)| {
                driven.take_due(start + STREAM_PERIOD * *n + Duration::from_millis(*wait))
            })
            .count();
        assert_eq!(
            captured,
            waits.len(),
            "a frame driven at the stream's rate captures every time"
        );
        // In view, the window redraws at 30 Hz: still about five a second.
        let captured = (0_u64..60)
            .filter(|n| in_view.take_due(start + Duration::from_millis(33 * n)))
            .count();
        assert!(
            (9..=11).contains(&captured),
            "{captured} captures in two seconds of 30 Hz frames"
        );
    }

    #[test]
    fn a_fast_mouse_is_sent_as_its_latest_position_and_never_fills_the_queue() -> Result<()> {
        // A batch leaves every 150 ms with at most 32 events, so 213 a
        // second. A 500 or 1000 Hz mouse makes more pointer moves than
        // that, and the 512-event queue between the frame loop and the
        // worker filled within a second or two of steady movement: the
        // connection was then ended as "input could not reach the host".
        let (sender, inputs) = sync_channel(512);
        let at = |n: u16| {
            let x = f32::from(n) / 1000.0;
            Input::Move { x, y: x }
        };
        let press = Input::Button {
            x: 0.5,
            y: 0.5,
            button: 0,
            down: true,
            modifiers: Modifiers::default(),
        };
        for n in 0..400 {
            sender.try_send(at(n))?;
        }
        sender.try_send(press.clone())?;
        for n in 400..500 {
            sender.try_send(at(n))?;
        }
        let mut pending = None;
        let mut batch = Vec::new();
        next_batch(&mut pending, &inputs, false, &mut batch)?;
        assert_eq!(
            batch,
            vec![at(399), press, at(499)],
            "each run of moves is its last position, and the press keeps its place"
        );
        assert!(
            inputs.try_recv().is_err() && pending.is_none(),
            "nothing is left behind"
        );
        Ok(())
    }

    #[test]
    fn a_remote_ctrl_click_leaves_no_ctrl_held_at_the_host() {
        // Pointer moves carry no modifiers, so the Ctrl a click brought
        // used to stay in the host's egui until the next key (D405).
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::default()
        };
        let click = |down| Input::Button {
            x: 0.5,
            y: 0.5,
            button: 0,
            down,
            modifiers: ctrl,
        };
        let ctx = egui::Context::default();
        let mut host = HostInput::default();
        let mut held_after = Vec::new();
        for wire in [
            vec![click(true)],
            vec![Input::Move { x: 0.6, y: 0.5 }],
            vec![click(false)],
            vec![Input::Move { x: 0.7, y: 0.5 }],
        ] {
            let mut events = Vec::new();
            host.translate(&wire, Vec2::new(800.0, 600.0), &mut events);
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        Vec2::new(800.0, 600.0),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |_| {},
            );
            output.drop_without_applying_deltas();
            held_after.push(ctx.input(|input| input.modifiers.ctrl));
        }
        assert_eq!(
            held_after,
            [true, true, true, false],
            "Ctrl is held from the press through the drag and the release (a \
             Ctrl+click reads it there), and let go before the next input"
        );
    }

    #[test]
    fn reset_releases_keys_and_buttons_at_host_coordinates() {
        let mut host = HostInput::default();
        let mut out = Vec::new();
        host.translate(
            &[
                Input::Button {
                    x: 0.25,
                    y: 0.5,
                    button: 0,
                    down: true,
                    modifiers: Modifiers::default(),
                },
                Input::Key {
                    key: "A".into(),
                    down: true,
                    modifiers: Modifiers::default(),
                },
                Input::Reset {},
            ],
            Vec2::new(800.0, 600.0),
            &mut out,
        );
        assert!(out.iter().any(|event| matches!(event, Event::PointerButton { pos, pressed: false, .. } if *pos == Pos2::new(200.0, 300.0))), "button released at host coordinates");
        assert!(
            out.iter().any(|event| matches!(
                event,
                Event::Key {
                    key: egui::Key::A,
                    pressed: false,
                    ..
                }
            )),
            "key released"
        );
        assert!(
            host.keys.is_empty() && host.buttons.is_empty(),
            "nothing stuck"
        );
    }

    /// A window of noise, bottom-up RGBA as the readback gives it: the
    /// worst a noisy preview can be.
    fn noise(size: [u32; 2]) -> Vec<u8> {
        let mut state = 0x2545_f491_u32;
        (0..size[0] as usize * size[1] as usize * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state.to_le_bytes()[0]
            })
            .collect()
    }

    #[test]
    fn a_capture_too_big_for_the_wire_is_sent_smaller_rather_than_ending_the_session() -> Result<()>
    {
        // A 2560x1600 window of noise is over 3 MiB at quality 90 (4K is
        // about 7 MiB); it used to end the session
        // ("capture exceeds JPEG limit", D406).
        let size = [2560, 1600];
        let mut ladder = Ladder::default();
        let encoded = encode(size, &mut noise(size), &mut ladder)?;
        assert!(encoded.jpeg.len() <= MAX_JPEG, "within the limit");
        assert!(ladder.step > 0, "on a lower step: {ladder:?}");
        let decoded = decode(
            encoded.size,
            &strobe_remote::client::encode_image(&encoded.jpeg),
        )?;
        assert_eq!(
            decoded.size,
            [encoded.size[0] as usize, encoded.size[1] as usize],
            "the viewer reads the size that was sent"
        );
        // The next capture starts where this one fitted: one encode.
        let settled = ladder.step;
        let again = encode(size, &mut noise(size), &mut ladder)?;
        assert_eq!(again.size, encoded.size, "the same step");
        assert_eq!(ladder.step, settled, "and it stays there");

        // A quiet picture afterwards climbs back after a run of easy
        // captures, one step at a time (small, so the test stays quick).
        for _ in 0..CLIMB_AFTER {
            let _ = encode([64, 64], &mut vec![90_u8; 64 * 64 * 4], &mut ladder)?;
        }
        assert_eq!(ladder.step, settled - 1, "one step back up");
        Ok(())
    }

    #[test]
    fn a_shrunk_capture_averages_its_blocks() {
        // Two by two pixels, top row black and white, bottom row white.
        let rgb = [0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255];
        let (small, size) = shrink(&rgb, [2, 2], 2);
        assert_eq!(size, [1, 1], "half on each edge");
        assert_eq!(small, [191, 191, 191], "the mean of the block");
        // An odd edge drops the column that does not fill a block.
        let (_, size) = shrink(&[0; 5 * 3 * 3], [5, 3], 2);
        assert_eq!(size, [2, 1], "floor of each edge");
    }

    #[test]
    fn jpeg_round_trip_flips_bottom_up_capture_and_checks_dimensions() -> Result<()> {
        let mut pixels = Vec::new();
        for y in 0..32 {
            for _ in 0..32 {
                pixels.extend_from_slice(if y < 16 {
                    &[255, 0, 0, 255]
                } else {
                    &[0, 0, 255, 255]
                });
            }
        }
        let bytes = encode([32, 32], &mut pixels, &mut Ladder::default())?.jpeg;
        let jpeg = strobe_remote::client::encode_image(&bytes);
        let decoded = decode([32, 32], &jpeg)?;
        assert!(
            decoded.pixels[0].b() > 200 && decoded.pixels[31 * 32].r() > 200,
            "top and bottom are flipped"
        );
        assert!(
            decode([64, 32], &jpeg).is_err(),
            "header dimensions checked before decode"
        );
        Ok(())
    }
}
