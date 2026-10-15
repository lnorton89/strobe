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
            | Action::OpenRemotePairing
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
        if now < self.due || self.gpu.as_ref().is_some_and(|gpu| gpu.pending.is_some()) {
            return;
        }
        self.due = now + STREAM_PERIOD;
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
    while let Ok(mut frame) = frames.recv() {
        if frame.desktop.active() {
            match encode(frame.size, &mut frame.pixels) {
                Ok(bytes) => frame
                    .desktop
                    .publish(frame.size, strobe_remote::client::encode_image(&bytes)),
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

fn encode(size: [u32; 2], pixels: &mut [u8]) -> Result<Vec<u8>, image::ImageError> {
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
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode(
        &rgb,
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;
    if jpeg.len() > MAX_JPEG {
        return Err(image::ImageError::IoError(io::Error::new(
            io::ErrorKind::InvalidData,
            "capture exceeds JPEG limit",
        )));
    }
    Ok(jpeg)
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
fn parse_host(text: &str) -> Result<SocketAddr, &'static str> {
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
fn wait_hint(reconnecting: bool) -> &'static str {
    if reconnecting {
        "Reconnecting with the saved pairing needs no approval. If this takes more than a few \
         seconds, check that Strobe is running on the host and on the same network."
    } else {
        "On the host, open Settings > Remote. A request from this desktop appears there once \
         the code is accepted: press Approve."
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
            "The host did not approve this desktop within {} s. On the host, open Settings > \
             Remote and press Approve when the request appears. The code is used up, so ask \
             for a new one with Pair a device.",
            limit.as_secs()
        )
    }
}

/// What the viewer says when the host answers no before the session starts.
///
/// The host's own words ("not paired, or revoked") are right for a phone
/// and no help to the person at this desktop, so each code says what to do
/// and where the code appears.
fn refusal_text(code: ErrorCode, host_message: &str) -> String {
    const WHERE: &str = "On the host, open Settings > Remote, switch on Allow full desktop \
                         control, and press Pair a device: the six-digit code appears there.";
    match code {
        ErrorCode::Unauthorized => format!(
            "The host no longer knows this desktop (not paired, or the pairing was revoked). \
             A new pairing code from the host is needed. {WHERE}"
        ),
        ErrorCode::PairingRejected => format!(
            "The host refused the pairing code ({host_message}). Enter the current code, which \
             lasts two minutes. {WHERE}"
        ),
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
}

impl Default for Controller {
    fn default() -> Self {
        Self {
            address: String::new(),
            code: String::new(),
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
        }
    }
}

impl Controller {
    /// Whether the remote view has replaced the local shell.
    pub fn active(&self) -> bool {
        self.worker.is_some()
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
        let code = std::mem::take(&mut self.code);
        let remembered = self.remembers(address);
        if code.trim().is_empty() && !remembered {
            self.set_note(
                "Enter the pairing code from the host: Settings > Remote > Pair a device, with \
                 Allow full desktop control on.",
            );
            return;
        }
        self.view = Arc::new(Mutex::new(View {
            note: "Connecting".into(),
            reconnecting: code.trim().is_empty(),
            ..View::default()
        }));
        self.started = Some(Instant::now());
        self.stop = Arc::new(AtomicBool::new(false));
        // A fresh slot, so a worker still ending from the last connection
        // cannot overwrite this one's interrupter.
        self.socket = Arc::new(Mutex::new(None));
        let link = Link {
            address,
            code,
            approval_limit: self.approval_limit,
            view: Arc::clone(&self.view),
            stop: Arc::clone(&self.stop),
            socket: Arc::clone(&self.socket),
            credential: Arc::clone(&self.credential),
        };
        let (sender, inputs) = sync_channel(512);
        let view = Arc::clone(&self.view);
        let stop = Arc::clone(&self.stop);
        match std::thread::Builder::new()
            .name("desktop controller".into())
            .spawn(move || {
                if let Err(error) = control(link, inputs) {
                    // A cancel closes the socket itself: that is not news.
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let started =
                        view.lock().unwrap_or_else(PoisonError::into_inner).stage == Stage::Live;
                    let how = if started { End::Lost } else { End::Unreachable };
                    end(&view, how, lost_text(&error.to_string()));
                }
            }) {
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
                    Stage::Ended(_) => {}
                }
            }
        }
        self.started = None;
        self.texture = None;
        self.texture_context = None;
        self.revision = 0;
        self.focused = false;
        self.input_note.clear();
        self.input_frame = None;
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
                    self.stop.store(true, Ordering::Relaxed);
                    if let Some(socket) = self
                        .socket
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .as_ref()
                    {
                        // A peer-closed socket already satisfies the interruption.
                        let _ = socket.shutdown(Shutdown::Both);
                    }
                    let mut view = self.view.lock().unwrap_or_else(PoisonError::into_inner);
                    view.ready = false;
                    view.note =
                        "Desktop connection interrupted; return to local Strobe and pair again"
                            .into();
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
        let (size, ready, stage, note, reconnecting) = {
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
            )
        };
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Controlling {}", self.address));
            let leave = match stage {
                Stage::Connecting => "Cancel",
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
                // The time moves on without any input arriving.
                ui.ctx().request_repaint_after(Duration::from_millis(500));
                let heading = format!("Connecting to {}", self.address);
                card(
                    ui.ctx(),
                    &heading,
                    &[
                        &waiting_text(waited, self.approval_limit),
                        wait_hint(reconnecting),
                    ],
                    None,
                );
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
                let reconnect = self.remembers_current();
                let mut lines = vec![note.as_str()];
                lines.extend(advice.filter(|_| self.texture.is_some()));
                let mut offer = |ui: &mut egui::Ui| {
                    if reconnect {
                        if ui.button("Reconnect").clicked() {
                            actions.push(crate::ui::Action::ConnectDesktop);
                        }
                    } else if ui.button("Enter a new pairing code").clicked() {
                        actions.push(crate::ui::Action::ControlAnotherDesktop);
                    }
                };
                card(ui.ctx(), heading, &lines, Some(&mut offer));
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
            | Staged::TimedOutReconnect => 0,
        };
        controller.started = Instant::now().checked_sub(Duration::from_secs(waited));
        let with_image = matches!(how, Staged::Live | Staged::Paused | Staged::Dropped);
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
            Staged::Live => {
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
    approval_limit: Duration,
    view: Arc<Mutex<View>>,
    stop: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
    credential: Arc<Mutex<Option<Credential>>>,
}

fn control(link: Link, inputs: Receiver<Input>) -> io::Result<()> {
    let Link {
        address,
        code,
        approval_limit,
        view,
        stop,
        socket,
        credential,
    } = link;
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
        if !authenticated && approval_expired(now.duration_since(began), approval_limit) {
            let reconnecting = view
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .reconnecting;
            end(
                &view,
                End::TimedOut,
                timed_out_text(approval_limit, reconnecting),
            );
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the host did not approve in time",
            ));
        }
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
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Host stopped responding; pair again",
            ));
        }
        if let Some(message) = connection.receive()? {
            heard = Instant::now();
            match message {
                ServerMessage::Hello {
                    protocol_version, ..
                } => {
                    if strobe_remote::negotiate(protocol_version).is_err() {
                        return Err(io::Error::other("Host uses an incompatible protocol"));
                    }
                    let token = credential
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .as_ref()
                        .filter(|remembered| {
                            remembered.address == address && code.trim().is_empty()
                        })
                        .map(|remembered| remembered.token.clone());
                    match token {
                        Some(token) => connection.send(&ClientMessage::Authenticate { token })?,
                        None => {
                            connection.send(&ClientMessage::PairDesktop { code: code.clone() })?
                        }
                    }
                }
                ServerMessage::Authenticated { .. } => {
                    authenticated = true;
                    last_frame = Instant::now();
                    {
                        let mut view = view.lock().unwrap_or_else(PoisonError::into_inner);
                        view.stage = Stage::Live;
                        view.note = "Approved. Waiting for the host's first image.".into();
                    }
                    connection.send(&ClientMessage::DesktopOpen {})?;
                }
                ServerMessage::DesktopFrame {
                    revision,
                    size,
                    jpeg,
                } => {
                    let image = decode(size, &jpeg)?;
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
                    if code == ErrorCode::Unauthorized {
                        *credential.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    }
                    if authenticated {
                        end(&view, End::Lost, lost_text(&message));
                    } else {
                        end(&view, End::Refused, refusal_text(code, &message));
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
                ServerMessage::State { .. }
                | ServerMessage::CommandResult { .. }
                | ServerMessage::Pong {} => {}
            }
        }
        if authenticated && now >= next_send {
            let mut bytes = 128;
            while batch.len() < 32 {
                let Some(event) = pending.take().or_else(|| inputs.try_recv().ok()) else {
                    break;
                };
                if paused {
                    continue;
                }
                let length = serde_json::to_vec(&event).map_err(io::Error::other)?.len() + 1;
                if bytes + length > strobe_remote::MAX_MESSAGE_BYTES {
                    pending = Some(event);
                    break;
                }
                bytes += length;
                batch.push(event);
            }
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
    let point = |pos: Pos2| {
        (
            (pos.x - rect.min.x) / rect.width(),
            (pos.y - rect.min.y) / rect.height(),
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
}

impl HostInput {
    /// Whether any injected edge needs release when remote access stops.
    pub fn held(&self) -> bool {
        !self.keys.is_empty() || !self.buttons.is_empty()
    }
    /// Convert validated wire inputs to the host's ordinary egui events.
    pub fn translate(&mut self, events: &[Input], size: Vec2, out: &mut Vec<Event>) {
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
            Action::OpenRemotePairing,
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
            Action::CloseRemotePairing,
            Action::DecideRemotePairing {
                id: 1,
                approve: false,
            },
            Action::RevokeRemoteClient("c00000001".into()),
            Action::RevokeAllRemoteClients,
            Action::SetImportListener(false),
            Action::EditorSave,
            Action::DisconnectDesktop,
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
            wait_hint(false).contains("Settings > Remote") && wait_hint(false).contains("Approve"),
            "{}",
            wait_hint(false)
        );
        assert!(
            !wait_hint(true).contains("Approve"),
            "a reconnect needs no approval: {}",
            wait_hint(true)
        );
        let expired = timed_out_text(limit, false);
        assert!(
            expired.contains("60 s") && expired.contains("Approve") && expired.contains("new one"),
            "says how long, what to press and that the code is spent: {expired}"
        );
    }

    #[test]
    fn a_refused_reconnect_says_a_new_code_is_needed_and_where_it_appears() {
        let text = refusal_text(ErrorCode::Unauthorized, "not paired, or revoked");
        assert!(
            text.contains("new pairing code"),
            "asks for a new code: {text}"
        );
        assert!(
            text.contains("Settings > Remote") && text.contains("Pair a device"),
            "and names where it appears on the host: {text}"
        );
        assert!(
            text.contains("revoked"),
            "and still says why it was refused: {text}"
        );
        let wrong = refusal_text(ErrorCode::PairingRejected, "wrong code");
        assert!(
            wrong.contains("wrong code") && wrong.contains("Pair a device"),
            "{wrong}"
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
                seen.text.contains("Enter a new pairing code"),
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
            controller.note().contains("pairing code"),
            "no code and no saved pairing: {}",
            controller.note()
        );
        assert!(!controller.active(), "nothing started");
        controller.code = "123456".into();
        controller.connect();
        assert!(controller.active(), "connecting");
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
        controller.disconnect();
        server.stop();
        Ok(())
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
            note.contains("new pairing code") && note.contains("Pair a device"),
            "{note}"
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
        let jpeg =
            strobe_remote::client::encode_image(&encode([32, 32], &mut vec![200; 32 * 32 * 4])?);
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
        let bytes = encode([32, 32], &mut pixels)?;
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
