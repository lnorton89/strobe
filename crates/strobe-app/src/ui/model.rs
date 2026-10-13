// ============================================================================
// Module:       strobe_app::ui::model
// Description:  What the panels read each frame, and the actions they hand
//               back to the app.
//
// Dependencies: strobe-scene (ControlBank); crate::{library, console, stats,
//               import, script}; super::{state, widgets}
// ============================================================================

//! The UI's inputs and outputs.
//!
//! [`Model`] borrows from the app for one frame. Three things in it are
//! mutable: the control bank, which is the point of the inspector (the one
//! sanctioned direct write into runtime state, because the bank *is* the
//! control model and a second copy would be the fake the plan forbids); the
//! [`UiState`](super::state::UiState), which is presentation state (pane
//! visibility, the browser query) that the app owns and persists; and the
//! render-scale slider's working value, which exists so a drag does not
//! reallocate every render target on every pointer move.
//!
//! Everything else the panels want done comes back as an [`Action`], and the
//! app carries it out after the frame is painted. Widgets never reach into
//! the renderer, the audio input or the windows.

use super::state::UiState;
use super::widgets::Tone;
use crate::console::Console;
use crate::import::Status;
use crate::library::{Health, Library};
use crate::remote::Remote;
use crate::script::ScriptState;
use crate::show::Show;
use crate::show::generate::GenerateForm;
use crate::stats::FrameStats;
use strobe_audio::capture::{DeviceChoice, DeviceInfo};
use strobe_io::OscSettings;
use strobe_playlist::Generated;
use strobe_scene::control::ControlBank;

/// Something the UI asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Load the library entry at this index (the scene becomes live).
    Load(usize),
    /// Prepare the entry at this index in the browser's preview, without
    /// changing what is live (UI_IMPLEMENTATION_PLAN section 8).
    Preview(usize),
    /// Throw away the browser's preview.
    ClosePreview,
    /// Reload the current scene from disk.
    Reload,
    /// Generate controls for the playing scene in a worker process (D202).
    GenerateControls,
    /// Stop the generate worker.
    CancelGenerate,
    /// Load the scene the worker wrote.
    OpenGenerated,
    /// Delete the scene the worker wrote.
    DiscardGenerated,
    /// Put the worker's result away.
    DismissGenerated,
    /// Open a scene in the Editor: the library entry by name, or the
    /// playing scene when `None` (D204).
    OpenEditor(Option<String>),
    /// Write these tags into the playing scene's `scene.json` (D205).
    SetSceneTags(Vec<String>),
    /// Save the playing scene's live frame as its thumbnail (D222).
    CaptureThumbnail,
    /// Make the same setting edit to several cues (D230).
    EditCues {
        /// The cues, as authored.
        ats: Vec<crate::show::edit::At>,
        /// The change.
        edit: crate::show::edit::CueEdit,
    },
    /// Import a Shadertoy shader (a URL, an id or a file) and add it as a
    /// cue after this one (D229).
    ImportCueAfter {
        /// The cue it goes after.
        at: crate::show::edit::At,
        /// What was typed.
        text: String,
    },
    /// A macro's key went down or up (D228).
    MacroKey {
        /// The macro.
        id: u32,
        /// Down, or up.
        down: bool,
    },
    /// Show this picture or folder of pictures behind `syn_Media`, or give
    /// each scene its own back with `None` (D217).
    SetMedia(Option<std::path::PathBuf>),
    /// Change one cue of the loaded playlist (D213).
    EditCue {
        /// The cue, as authored.
        at: crate::show::edit::At,
        /// The change.
        edit: crate::show::edit::CueEdit,
    },
    /// Replace the edited playlist with its file, which changed on disk.
    ReloadPlaylistFromDisk,
    /// Give a command this shortcut, or none (D208). `steal` takes it from
    /// a command that holds it, which is then left with none.
    SetHotkey {
        /// The command.
        command: crate::commands::CommandId,
        /// The new chord; `None` for no shortcut.
        chord: Option<crate::commands::Chord>,
        /// Take the chord from its holder.
        steal: bool,
    },
    /// Put a command back on its default shortcut.
    ResetHotkey(crate::commands::CommandId),
    /// Pin a control as a Performance Macro, or unpin its macro (D210).
    PinMacro {
        /// The control's name.
        name: String,
        /// A meta control rather than one of the scene's.
        meta: bool,
        /// Pin (`true`) or unpin.
        pin: bool,
    },
    /// Write the Editor's unsaved files.
    EditorSave,
    /// Write them and load the scene again.
    EditorSaveReload,
    /// Throw away the Editor's unsaved edits.
    EditorRevert,
    /// Put the active file's backup text in the Editor.
    EditorRestoreBackup,
    /// List the scene folder again.
    RescanLibrary,
    /// Step to the next scene in the library.
    NextScene,
    /// Step to the previous scene in the library.
    PrevScene,
    /// Hold or resume the cue. With a playlist loaded this freezes the
    /// cue's clock; without one there is nothing to hold and the app says
    /// so.
    ToggleHold,
    /// Load the playlist file at this path (the LIST pane's field).
    LoadPlaylist(String),
    /// Unload the playlist and go back to library order.
    UnloadPlaylist,
    /// Add 30 seconds to the current cue.
    ExtendCue,
    /// Start the current cue again.
    RestartCue,
    /// Go to the first cue of the next section.
    SkipSection,
    /// Go to this timeline cue.
    JumpToCue(usize),
    /// Run the Generate dialog's form and show the result as a preview.
    GeneratePreview,
    /// Add the previewed section to the playlist.
    GenerateCommit,
    /// Write the playlist to its file.
    SavePlaylist,
    /// Black the output out or bring it back.
    SetBlackout(bool),
    /// Apply this render scale.
    SetRenderScale(f32),
    /// Show or hide the output window. The scene keeps rendering either way.
    SetOutputVisible(bool),
    /// Toggle borderless fullscreen on the output window (showing it first
    /// if it is hidden).
    ToggleFullscreen,
    /// Move the output window to the monitor at this index.
    MoveToMonitor(usize),
    /// Empty the console.
    ClearConsole,
    /// Capture from this audio input.
    SelectAudioInput(DeviceChoice),
    /// List the capturable audio devices again.
    RefreshAudioDevices,
    /// Set the gain on audio levels and hits.
    SetReactivity(f32),
    /// Set the gain on the audio time clocks.
    SetAudioSpeed(f32),
    /// Register a tap-tempo tap.
    TapTempo,
    /// Turn adaptive resolution on or off.
    SetAdaptive(bool),
    /// Import this file path, Shadertoy id or URL.
    Import(String),
    /// Replace the scene the import panel is asking about.
    ConfirmReplace,
    /// Leave the existing scene alone.
    CancelReplace,
    /// Start or stop the browser listener.
    SetImportListener(bool),
    /// Quit the app (the control window is the app, D96).
    Quit,
    /// Draw Strobe's title bar (true) or the system's (false).
    SetCustomTitleBar(bool),
    /// Dismiss the scene-failure banner.
    DismissFailure,
    /// Dismiss the notice banner.
    DismissNotice,
    /// Apply these OSC settings (and save them).
    SetOsc(OscSettings),
    /// Open MIDI input devices at all, or none.
    SetMidiEnabled(bool),
    /// Use or ignore one MIDI device, by its key.
    SetMidiDevice(String, bool),
    /// List the MIDI ports again now.
    RescanMidi,
    /// Save the controls as a preset of this name in the scene folder.
    SavePreset(String),
    /// Recall a preset of the playing scene.
    RecallPreset(String),
    /// Delete a preset file of the playing scene.
    DeletePreset(String),
    /// Rename a preset of the playing scene (P-07).
    RenamePreset {
        /// Its name now.
        from: String,
        /// The new name.
        to: String,
    },
    /// Store the playing scene (and its active preset) in fav slot `n`,
    /// counted from 0.
    StoreFav(usize),
    /// Empty fav slot `n`.
    ClearFav(usize),
    /// Recall fav slot `n`.
    RecallFav(usize),
    /// Run a command from the command table by its id (a macro or the MAP
    /// pane's test button; keys, MIDI and OSC reach the same place).
    RunCommand(String),
    /// Default or randomize a bank, a group or a control (Synesthesia's
    /// `/default` and `/random`, D247).
    ControlAction(strobe_io::ControlAction),
    /// Start or stop the LAN remote (D254). Stop drops every phone at once.
    SetLanRemote(bool),
    /// Show a pairing code for a phone to type.
    OpenRemotePairing,
    /// Withdraw the pairing code.
    CloseRemotePairing,
    /// Approve or deny a phone that typed the code.
    DecideRemotePairing {
        /// The request.
        id: u64,
        /// Approve (`true`) or deny.
        approve: bool,
    },
    /// Revoke one paired device, by its id.
    RevokeRemoteClient(String),
    /// Revoke every paired device.
    RevokeAllRemoteClients,
}

/// What the performance readouts and the adaptive toggle show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerfPanel {
    /// Whether adaptive resolution is switched on.
    pub adaptive: bool,
    /// Whether the GPU can be timed on this context at all.
    pub has_timer: bool,
    /// Smoothed GPU milliseconds per frame, once measured.
    pub gpu_ms: Option<f32>,
    /// The per-frame budget in milliseconds.
    pub budget_ms: f32,
    /// The render scale in force.
    pub scale: f32,
}

/// The audio settings panel's inputs.
pub struct AudioPanel<'a> {
    /// The status line: which source is active, and on which device.
    pub source: &'a str,
    /// Whether a live capture is the active source.
    pub live: bool,
    /// Whether capture was asked for (`--audio synthetic` says no).
    pub can_capture: bool,
    /// Capturable devices, inputs and (on Windows) outputs for loopback.
    pub devices: &'a [DeviceInfo],
    /// The device chosen.
    pub choice: &'a DeviceChoice,
    /// Reactivity slider's value.
    pub reactivity: &'a mut f32,
    /// Audio-speed slider's value.
    pub speed: &'a mut f32,
    /// The tempo in the latest frame.
    pub bpm: f32,
}

/// The browser listener's row in the import panel.
pub struct ListenerPanel<'a> {
    /// Whether the person has it switched on.
    pub enabled: bool,
    /// One line: where it listens, or why it does not.
    pub state: &'a str,
    /// Whether `state` is an error.
    pub failed: bool,
    /// The token to paste into the userscript, once the listener has one.
    pub token: Option<&'a str>,
}

/// The LAN remote, for Settings and the title bar.
pub struct LanPanel<'a> {
    /// What it is doing, made by the app this frame.
    pub view: &'a crate::lan::LanView,
    /// The address field.
    pub address: &'a mut String,
    /// The port field.
    pub port: &'a mut String,
}

/// The import panel's inputs.
pub struct ImportPanel<'a> {
    /// The path/id field.
    pub text: &'a mut String,
    /// The result line.
    pub status: &'a Status,
    /// The scene the panel is asking whether to replace.
    pub replace_title: Option<&'a str>,
    /// An import is queued or running.
    pub busy: bool,
    /// A file is being dragged over the window.
    pub drop_hover: bool,
    /// The browser listener.
    pub listener: ListenerPanel<'a>,
    /// The playing scene is the one this pane just imported, so it may
    /// offer "Generate controls" for it (U-03, D246).
    pub imported_playing: bool,
    /// A generate-controls worker is running.
    pub generating: bool,
}

/// A texture of the scene's output, for a preview.
#[derive(Debug, Clone, Copy)]
pub struct Preview {
    /// The registered texture.
    pub texture: egui::TextureId,
    /// Width over height of the rendered frame.
    pub aspect: f32,
    /// The render resolution, for the readout.
    pub size: [i32; 2],
}

/// The scene the browser is previewing without playing it.
#[derive(Debug, Clone, Copy)]
pub struct ScenePreview<'a> {
    /// Its library index.
    pub index: usize,
    /// Its name.
    pub name: &'a str,
    /// Its picture.
    pub image: Preview,
}

/// The playlist cue the transport is on, when a playlist is loaded.
#[derive(Debug, Clone, Copy)]
pub struct CueTransport<'a> {
    /// Position in the timeline, from 0.
    pub index: usize,
    /// How many cues the timeline has.
    pub total: usize,
    /// The section it is in.
    pub section: &'a str,
    /// Seconds the cue has played.
    pub elapsed: f32,
    /// Seconds until it ends, or `None` when only a person ends it.
    pub remaining: Option<f32>,
    /// The cue's clock is frozen.
    pub held: bool,
    /// How far the transition into this cue has got, 0 to 1.
    pub transition: Option<f32>,
    /// Seconds until an anchored start, while the show waits for it.
    pub waiting: Option<f32>,
}

/// The transport strip's inputs.
#[derive(Debug, Clone, Copy)]
pub struct TransportPanel<'a> {
    /// The playing scene's name.
    pub now: Option<&'a str>,
    /// The scene a Next press would load: the playlist's next cue, or the
    /// next in library order when there is no playlist.
    pub next: Option<&'a str>,
    /// The playlist cue, when a playlist is loaded.
    pub cue: Option<CueTransport<'a>>,
    /// The playing scene's clock, in seconds.
    pub scene_time: f32,
    /// Whether the output is blacked out.
    pub blackout: bool,
    /// Whether there is more than one scene to step through.
    pub can_step: bool,
}

/// What the LIST pane keeps between frames. Presentation state: none of it
/// is saved.
#[derive(Debug, Default)]
pub struct PlaylistUi {
    /// The load field.
    pub path: String,
    /// The Generate dialog is open.
    pub generate_open: bool,
    /// The Generate dialog's fields.
    pub form: GenerateForm,
    /// The last Generate preview, or why there is none.
    pub preview: Option<Result<Generated, String>>,
    /// The row highlighted in the timeline.
    pub selected: Option<usize>,
    /// More rows Ctrl-clicked into the selection (D230); the card's setting
    /// edits go to these too.
    pub also: Vec<usize>,
    /// The current cue the list last scrolled to, so it scrolls once per
    /// cue rather than fighting a person who scrolled away.
    pub scrolled_to: Option<usize>,
    /// The last load's outcome: whether it is an error, and the message.
    pub status: Option<(bool, String)>,
    /// The playlist differs from its file.
    pub dirty: bool,
    /// The selected cue's fields while they are edited (D213).
    pub cue: CueDraft,
    /// The playlist's file changed on disk while the playlist had unsaved
    /// edits: the path, so the pane can offer to reload it.
    pub external_change: Option<String>,
}

/// The cue card's fields, as typed. Refilled from the cue whenever another
/// cue is selected or the playlist changes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CueDraft {
    /// The cue these fields were filled from, and the rows' revision.
    pub for_cue: Option<(crate::show::edit::At, u64)>,
    /// The length, as typed (`3m`, `32 bars`); empty to inherit.
    pub length: String,
    /// The transition's kind.
    pub transition: crate::show::generate::TransitionChoice,
    /// The transition's length.
    pub transition_length: String,
    /// The preset's name; empty for none.
    pub preset: String,
    /// The cue's media path; empty for none (D217).
    pub media: String,
    /// A Shadertoy URL or id to import and add after the cue (D229).
    pub shadertoy: String,
}

/// Where the picture behind `syn_Media` comes from (D217).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFrom {
    /// The scene's own `MEDIA`, or nothing for a scene without one.
    Scene,
    /// The playing cue's `media`.
    Cue,
    /// The operator's choice.
    Chosen,
}

/// The Inspector's media row.
#[derive(Debug, Clone, Copy)]
pub struct MediaPanel<'a> {
    /// What shows, or is on its way; `None` for nothing.
    pub showing: Option<&'a std::path::Path>,
    /// Where that comes from.
    pub from: MediaFrom,
    /// Whether it is still decoding.
    pub loading: bool,
    /// Whether the scene reads `syn_Media` at all.
    pub read: bool,
    /// Whether the operator has chosen media, which shows wherever a cue
    /// names none.
    pub chosen: bool,
}

/// The LIST pane's inputs.
pub struct PlaylistPanel<'a> {
    /// The pane's own state.
    pub ui: &'a mut PlaylistUi,
    /// The playlist being played, if one is loaded.
    pub show: Option<&'a Show>,
}

/// What a new mapping or macro is being made of, in the MAP pane's form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AddKind {
    /// A control of the playing scene.
    #[default]
    SceneControl,
    /// A meta control.
    Meta,
    /// A transport or window command.
    Command,
    /// A playlist cue number.
    Cue,
    /// A fav slot.
    Fav,
    /// A preset of the playing scene.
    Preset,
    /// An existing Performance Macro.
    Macro,
}

/// The MAP pane's "add" form.
#[derive(Debug, Default, Clone)]
pub struct AddForm {
    /// The form is open.
    pub open: bool,
    /// What kind of target.
    pub kind: AddKind,
    /// The control, command or preset name, or the cue or slot number.
    pub text: String,
    /// The component for a pad or colour, from 0.
    pub component: u8,
    /// Make a macro for it instead of a mapping.
    pub as_macro: bool,
}

/// What the MAP pane keeps between frames. Presentation state: none of it
/// is saved.
#[derive(Debug, Default)]
pub struct MapUi {
    /// The mapping row whose options are open.
    pub selected: Option<usize>,
    /// The macro row whose options are open.
    pub selected_macro: Option<u32>,
    /// The search box.
    pub search: String,
    /// The OSC port field while it is being edited.
    pub port_text: String,
    /// The feedback host field while it is being edited.
    pub feedback_host_text: String,
    /// The feedback port field while it is being edited.
    pub feedback_port_text: String,
    /// The add form.
    pub add: AddForm,
    /// The preset name field of the controls pane.
    pub preset_name: String,
    /// The preset whose delete button was clicked once: the second click
    /// deletes (P-07).
    pub delete_armed: Option<String>,
    /// The macro waiting for its key to be pressed (D228).
    pub key_capture: Option<u32>,
    /// Why the last key offered to a macro was refused.
    pub key_problem: Option<String>,
}

/// Show control, for the MAP pane, the macro strip and the preset row.
pub struct RemotePanel<'a> {
    /// The pane's own state.
    pub ui: &'a mut MapUi,
    /// The mapper, the devices, the fav slots and the activity log.
    pub remote: &'a mut Remote,
    /// The playing scene's preset names.
    pub presets: &'a [String],
    /// The preset last recalled or saved.
    pub active_preset: Option<&'a str>,
    /// The playing scene's library name, for fav slots.
    pub scene_name: Option<&'a str>,
    /// The keyboard shortcuts in force.
    pub bindings: &'a crate::commands::Bindings,
}

/// A scene that did not load.
#[derive(Debug, Clone, Copy)]
pub struct FailurePanel<'a> {
    /// Which scene.
    pub scene: &'a str,
    /// The first lines of why.
    pub summary: &'a str,
    /// The scene still playing, if there is one.
    pub kept: Option<&'a str>,
}

/// A banner that is not a failure: a recovered context, say.
#[derive(Debug, Clone, Copy)]
pub struct NoticePanel<'a> {
    /// How serious.
    pub tone: Tone,
    /// What to say.
    pub text: &'a str,
}

/// Everything the panels need for one frame.
pub struct Model<'a> {
    /// Presentation state: panes, the browser query. Persisted by the app.
    pub ui: &'a mut UiState,
    /// The scenes on disk.
    pub library: &'a Library,
    /// What is known of each scene's health, parallel to the library.
    pub health: &'a [Health],
    /// Bumped whenever the library or its health changes, so the browser
    /// knows when to refilter.
    pub library_rev: u64,
    /// Why the last library scan failed, if it did.
    pub scan_error: Option<&'a str>,
    /// The library entry currently playing.
    pub playing: Option<usize>,
    /// The playing scene's title, from its definition.
    pub scene_title: &'a str,
    /// The playing scene's controls.
    pub controls: Option<&'a mut ControlBank>,
    /// What the playing scene shows behind `syn_Media` (D217).
    pub media: Option<MediaPanel<'a>>,
    /// The output-wide meta controls of the playing scene's session (D180).
    pub meta: Option<&'a mut ControlBank>,
    /// The live preview, once a scene has rendered.
    pub preview: Option<Preview>,
    /// The browser's preview of a scene that is not live.
    pub scene_preview: Option<ScenePreview<'a>>,
    /// The scene being compiled right now, shown for the frame before the
    /// UI thread blocks on it.
    pub loading: Option<&'a str>,
    /// The last scene that failed to load.
    pub failure: Option<FailurePanel<'a>>,
    /// A notice to show.
    pub notice: Option<NoticePanel<'a>>,
    /// What the file hovering over the window would do if dropped (D218).
    pub drop_hint: Option<&'a str>,
    /// The transport strip.
    pub transport: TransportPanel<'a>,
    /// Console lines.
    pub console: &'a Console,
    /// Frame timings.
    pub stats: &'a FrameStats,
    /// The render-scale slider's working value.
    pub render_scale: &'a mut f32,
    /// Monitor labels, for the output picker.
    pub monitors: &'a [String],
    /// The monitor the output was last sent to.
    pub output_monitor: Option<usize>,
    /// Whether the output window is shown.
    pub output_visible: bool,
    /// Whether the output window is fullscreen.
    pub fullscreen: bool,
    /// Whether the control window is maximised.
    pub maximized: bool,
    /// The GL backend, renderer and version, one line.
    pub gl_summary: &'a str,
    /// Audio source and settings.
    pub audio: AudioPanel<'a>,
    /// The playing scene's script, if it has one.
    pub script: Option<&'a ScriptState>,
    /// GPU time, budget and adaptive resolution.
    pub perf: PerfPanel,
    /// Shadertoy import.
    pub import: ImportPanel<'a>,
    /// The playlist pane.
    pub playlist: PlaylistPanel<'a>,
    /// MIDI, OSC, mappings, macros, presets.
    pub remote: RemotePanel<'a>,
    /// Generated controls for the playing scene.
    pub generate: GeneratePanel<'a>,
    /// The Editor's open scene.
    pub editor: EditorPanel<'a>,
    /// The LAN remote.
    pub lan: LanPanel<'a>,
}

/// The Editor pane's inputs.
#[derive(Debug, Default)]
pub struct EditorPanel<'a> {
    /// The scene open in the Editor, if any.
    pub doc: Option<&'a mut crate::editor::Doc>,
    /// The folder of the scene on screen.
    pub playing_dir: Option<&'a std::path::Path>,
}

/// The Inspector's "Generate controls" strip.
#[derive(Clone, Copy, Debug, Default)]
pub struct GeneratePanel<'a> {
    /// The scene a worker is measuring, and for how many seconds.
    pub running: Option<(&'a str, f32)>,
    /// The last worker's result, until it is opened, discarded or put away.
    pub outcome: Option<&'a crate::generate::Outcome>,
    /// The playing scene has controls a person wrote, so generation adds to
    /// them.
    pub authored: bool,
}
