// ============================================================================
// Module:       strobe_app::ui::shell_tests
// Description:  The whole shell drawn headless at the sizes the plan names and
//               in every state the audit lists. Test code only.
//
// Dependencies: egui (headless context); super::{shell, model, state},
//               crate::{library, console, stats, script}; strobe-gfx,
//               egui_glow and image for the opt-in screenshots
// ============================================================================

//! The shell, end to end, with no GPU.
//!
//! egui lays out and tessellates without a window, so the states STATUS_AUDIT
//! P1 lists (empty and loading, failure, missing audio, script fault, output
//! hidden, a narrow window) are drawn here at 1920x1080, 1440x900, 1366x768,
//! 1280x720, 900x600 and 640x480, and at the logical sizes the T480's small
//! windows have at 150% (U-09), and each must fit its window: the root
//! UI's used rectangle may not exceed the screen. That is the "no horizontal
//! scrolling" acceptance criterion (UI_IMPLEMENTATION_PLAN section 23) as an
//! assertion rather than a screenshot. Two more say what T480 Task 35 saw go
//! wrong cannot: nothing is painted over BLACKOUT, and no text is drawn over
//! other text unless an opaque pane covers the first.
//!
//! The screenshots in `docs/images/perform/` are the other half: they show
//! it looks right. `STROBE_UI_SHOTS=<dir> cargo test -p strobe-app --lib
//! screenshots_when_asked` paints every state at every size on a headless
//! GL context into that folder, for looking without a window.

use super::shell::{ShellState, show};
use super::{
    Action, AudioPanel, CueTransport, EditorPanel, FailurePanel, GeneratePanel, ImportPanel,
    LanPanel, ListenerPanel, MapUi, Model, NoticePanel, Offer, PerfPanel, PlaylistPanel,
    PlaylistUi, Preview, RemotePanel, ScenePreview, TransportPanel, UiState,
};
use crate::console::Console;
use crate::import::Status;
use crate::library::{Entry, Health, Library};
use crate::remote::{Overrides, Remote};
use crate::script::ScriptState;
use crate::stats::FrameStats;
use crate::ui::widgets::Tone;
use crate::workspace::Pane;
use anyhow::Result;
use egui::{Rect, Vec2};
use std::path::PathBuf;
use strobe_audio::capture::DeviceChoice;
use strobe_scene::control::ControlBank;
use strobe_scene::scene::SceneDef;

/// The sizes UI_DESIGN section 22 and the plan name, plus two small ones.
const SIZES: [(f32, f32); 6] = [
    (1920.0, 1080.0),
    (1440.0, 900.0),
    (1366.0, 768.0),
    (1280.0, 720.0),
    (900.0, 600.0),
    (640.0, 480.0),
];

/// Windows the T480 measured (U-09, Task 35) in logical points: at its 150%
/// scale, 1024x640, 800x600, 640x480 and a 480x800 portrait window.
const SMALL_SIZES: [(f32, f32); 4] = [
    (683.0, 427.0),
    (533.0, 400.0),
    (427.0, 320.0),
    (320.0, 533.0),
];

/// One state of the app to draw.
#[derive(Clone, Copy, Debug)]
struct Scenario {
    name: &'static str,
    scenes: usize,
    playing: bool,
    failed: bool,
    blackout: bool,
    output_visible: bool,
    audio_live: bool,
    audio_capture: bool,
    script: Option<&'static str>,
    loading: bool,
    notice: bool,
    /// The notice is several sentences long: it must wrap inside the main
    /// view and never push a panel over BLACKOUT.
    long_notice: bool,
    previewing: bool,
    replace: bool,
    /// A playlist is loaded (its scenes `02-scene-1` and `ghost` cannot play).
    playlist: bool,
    /// Show control has mappings, macros and fav slots in every state.
    mapped: bool,
    /// A generate-controls worker is running.
    generating: bool,
    /// A generate-controls result is waiting for review.
    generated: bool,
    /// The playing scene was just imported, so Import offers to generate
    /// controls for it (D246).
    imported: bool,
    /// The shortcut editor waits for a key for this command.
    capturing: Option<crate::commands::CommandId>,
    /// The MAP pane's key field waits for a key for this macro, which is
    /// not the macro selected there (D228).
    macro_capture: Option<u32>,
    /// The shortcut editor shows a clash (tap tempo wanted blackout's B).
    clash: bool,
    /// A cue is selected in the LIST pane, so its card shows (D213), and
    /// the file changed on disk under unsaved edits.
    cue_selected: bool,
    /// The window's density (D220).
    density: crate::ui::theme::Density,
    /// The LAN remote is on, with a phone waiting for approval (D254).
    lan: bool,
    /// Full desktop access enabled on the host.
    desktop_access: bool,
    /// The command palette is open with this typed (D262).
    palette: Option<&'static str>,
    /// Something asked for the "Control another Strobe" form to be shown
    /// (the palette entry, or a failed connection's retry): Settings
    /// scrolls to it and focuses the address.
    reveal_connect: bool,
    /// The connect form has "Use a trust file" ticked, with a path in it
    /// (venue mode, D294).
    trust_form: bool,
    /// The Editor refused a Save and reload: its `scene.json` does not
    /// parse (D263).
    editor_invalid: bool,
    /// Tag fields are open: the Inspector's for the playing scene, the
    /// browser's for its selected scene (the third) and the Import pane's
    /// for the scene just imported (D264).
    tagging: bool,
    /// The LIST pane's Generate card is open with an exclusion, previewed
    /// (D264).
    generate: bool,
    /// The browser has its third scene selected, so its tags show (D264).
    browser_selected: bool,
    /// Settings' watched folder: whether the app refused it, and its line
    /// (D265).
    watch: Option<(bool, &'static str)>,
    /// The side panel's tab (D283).
    side_tab: crate::ui::side::SideTab,
    /// The library grid's card size (D282).
    cards: crate::ui::browser::CardSize,
    /// The grid is sorted by author, descending, with the favourites toggle
    /// on, and the scenes carry authors (D282).
    ordered: bool,
    /// Synesthesia's own scenes are listed after the library's (D271):
    /// `01-scene-0` (a name the library has too), `glow` and `ink` from the
    /// install, and `deep-sea` from a folder in Synesthesia's settings. With
    /// `browser_selected`, the browser selects the first of them.
    synesthesia: bool,
    /// The playing scene, the Editor's and a refused write are Synesthesia's
    /// `01-scene-0`: read-only rows, and a notice offering the copy.
    on_synesthesia: bool,
    /// The open panes: the newest main pane is the main view, and
    /// `Pane::Inspector` is the side panel (D281).
    panes: &'static [Pane],
    /// The library card with keyboard focus, by library index (D291).
    focus_card: Option<usize>,
    /// Three recent playlists, one with a long path (D481).
    recent: bool,
    /// The Settings page, where the scenario's other state does not decide
    /// it.
    settings_page: Option<crate::ui::settings::SettingsPage>,
    /// The MAP pane has the first macro's options, the first mapping's
    /// options and the add form open (D467).
    map_open: bool,
    /// The About page's list of every crate is open (D491).
    credits_open: bool,
}

const BASE: Scenario = Scenario {
    name: "playing",
    scenes: 6,
    playing: true,
    failed: false,
    blackout: false,
    output_visible: true,
    audio_live: true,
    audio_capture: true,
    script: None,
    loading: false,
    notice: false,
    long_notice: false,
    previewing: false,
    replace: false,
    playlist: false,
    mapped: false,
    generating: false,
    generated: false,
    imported: false,
    capturing: None,
    macro_capture: None,
    clash: false,
    cue_selected: false,
    density: crate::ui::theme::Density::Comfortable,
    lan: false,
    desktop_access: false,
    palette: None,
    reveal_connect: false,
    trust_form: false,
    editor_invalid: false,
    tagging: false,
    generate: false,
    browser_selected: false,
    watch: None,
    ordered: false,
    synesthesia: false,
    on_synesthesia: false,
    side_tab: crate::ui::side::SideTab::Controls,
    cards: crate::ui::browser::CardSize::Medium,
    panes: &[Pane::Inspector, Pane::Browser],
    focus_card: None,
    recent: false,
    settings_page: None,
    map_open: false,
    credits_open: false,
};

/// Settings at About, with the credits (D491).
const ABOUT: Scenario = Scenario {
    name: "about",
    settings_page: Some(crate::ui::settings::SettingsPage::About),
    panes: &[Pane::Settings],
    ..BASE
};

/// The Controls view over the library, with the side panel (D479).
const CONTROLS: &[Pane] = &[Pane::Inspector, Pane::Browser, Pane::Controls];

fn scenarios() -> Vec<Scenario> {
    vec![
        BASE,
        ABOUT,
        Scenario {
            name: "about with every crate listed",
            credits_open: true,
            ..ABOUT
        },
        Scenario {
            name: "about, compact, with every crate listed",
            credits_open: true,
            density: crate::ui::theme::Density::Compact,
            ..ABOUT
        },
        Scenario {
            name: "full desktop access enabled",
            lan: true,
            desktop_access: true,
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "settings at the control another strobe form",
            lan: true,
            reveal_connect: true,
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "the connect form with a trust file",
            lan: true,
            reveal_connect: true,
            trust_form: true,
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "palette finds control another strobe",
            palette: Some("another strobe"),
            ..BASE
        },
        Scenario {
            name: "the playlist page with recent playlists",
            panes: &[Pane::Inspector, Pane::Playlist],
            recent: true,
            ..BASE
        },
        Scenario {
            name: "the controls view",
            panes: CONTROLS,
            ..BASE
        },
        Scenario {
            name: "the controls view, compact, playlist tab",
            panes: CONTROLS,
            density: crate::ui::theme::Density::Compact,
            side_tab: crate::ui::side::SideTab::Playlist,
            playlist: true,
            ..BASE
        },
        Scenario {
            name: "the controls view, no scene",
            panes: CONTROLS,
            playing: false,
            ..BASE
        },
        Scenario {
            name: "the controls view, side panel collapsed",
            panes: &[Pane::Browser, Pane::Controls],
            ..BASE
        },
        Scenario {
            name: "library with a focused card",
            focus_card: Some(2),
            ..BASE
        },
        Scenario {
            name: "compact, small cards",
            density: crate::ui::theme::Density::Compact,
            cards: crate::ui::browser::CardSize::Small,
            ..BASE
        },
        Scenario {
            name: "large cards, failed scene",
            cards: crate::ui::browser::CardSize::Large,
            failed: true,
            ..BASE
        },
        Scenario {
            name: "side panel on the playlist tab",
            playlist: true,
            side_tab: crate::ui::side::SideTab::Playlist,
            ..BASE
        },
        Scenario {
            name: "side panel on the playlist tab, no playlist",
            side_tab: crate::ui::side::SideTab::Playlist,
            ..BASE
        },
        Scenario {
            name: "side panel on the presets tab",
            side_tab: crate::ui::side::SideTab::Presets,
            ..BASE
        },
        Scenario {
            name: "side panel on the presets tab, no scene",
            side_tab: crate::ui::side::SideTab::Presets,
            playing: false,
            ..BASE
        },
        Scenario {
            name: "side panel on the edit tab",
            side_tab: crate::ui::side::SideTab::Edit,
            tagging: true,
            ..BASE
        },
        Scenario {
            name: "side panel on the edit tab, no scene",
            side_tab: crate::ui::side::SideTab::Edit,
            playing: false,
            ..BASE
        },
        Scenario {
            name: "library sorted by author, favourites only",
            ordered: true,
            browser_selected: true,
            ..BASE
        },
        Scenario {
            name: "library sorted by author, favourites only, compact",
            ordered: true,
            density: crate::ui::theme::Density::Compact,
            cards: crate::ui::browser::CardSize::Small,
            ..BASE
        },
        Scenario {
            name: "a long notice over the library",
            long_notice: true,
            ..BASE
        },
        Scenario {
            name: "a long notice over the play view",
            long_notice: true,
            panes: &[Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "side panel collapsed",
            panes: &[Pane::Browser],
            ..BASE
        },
        Scenario {
            name: "side panel collapsed, blackout, output hidden",
            blackout: true,
            output_visible: false,
            panes: &[Pane::Browser],
            ..BASE
        },
        Scenario {
            name: "play view with the side panel",
            panes: &[Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "compact playlist",
            density: crate::ui::theme::Density::Compact,
            playlist: true,
            panes: &[Pane::Playlist],
            ..BASE
        },
        Scenario {
            name: "settings with the LAN remote on and a phone to approve",
            lan: true,
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "settings watching a folder",
            watch: Some((false, "Watching D:\\VJ\\Strobe inbox")),
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "settings with a refused watched folder",
            watch: Some((
                true,
                "D:\\VJ\\scenes is the library itself: watch a folder scenes are copied into \
                 before they join it.",
            )),
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "settings with a shortcut clash",
            clash: true,
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "editor open with an unsaved edit",
            failed: true,
            panes: &[Pane::Editor, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "editor refused a reload, scene.json does not parse",
            editor_invalid: true,
            panes: &[Pane::Editor],
            ..BASE
        },
        Scenario {
            name: "tagging in the browser and the inspector",
            tagging: true,
            panes: &[Pane::Browser, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "browser with a scene selected",
            browser_selected: true,
            panes: &[Pane::Browser, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "tagging the scene just imported",
            imported: true,
            tagging: true,
            panes: &[Pane::Import, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "generate with an exclusion, previewed",
            generate: true,
            panes: &[Pane::Playlist],
            ..BASE
        },
        Scenario {
            name: "generating controls",
            generating: true,
            ..BASE
        },
        Scenario {
            name: "generated controls to review",
            generated: true,
            ..BASE
        },
        Scenario {
            name: "generated controls to review in the controls view",
            generated: true,
            panes: CONTROLS,
            ..BASE
        },
        Scenario {
            name: "empty library",
            scenes: 0,
            playing: false,
            panes: &[Pane::Browser, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "no scene yet",
            playing: false,
            ..BASE
        },
        Scenario {
            name: "loading",
            loading: true,
            ..BASE
        },
        Scenario {
            name: "scene failure",
            failed: true,
            panes: &[Pane::Browser],
            ..BASE
        },
        Scenario {
            name: "missing audio",
            audio_live: false,
            audio_capture: true,
            ..BASE
        },
        Scenario {
            name: "synthetic audio",
            audio_live: false,
            audio_capture: false,
            ..BASE
        },
        Scenario {
            name: "script fault",
            script: Some("fault"),
            ..BASE
        },
        Scenario {
            name: "script disabled",
            script: Some("disabled"),
            ..BASE
        },
        Scenario {
            name: "blackout",
            blackout: true,
            ..BASE
        },
        Scenario {
            name: "output hidden",
            output_visible: false,
            ..BASE
        },
        Scenario {
            name: "notice",
            notice: true,
            ..BASE
        },
        Scenario {
            name: "browser with a preview",
            previewing: true,
            panes: &[Pane::Browser, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "replace question",
            replace: true,
            panes: &[Pane::Import],
            ..BASE
        },
        Scenario {
            name: "diagnostics",
            panes: &[Pane::Diagnostics],
            ..BASE
        },
        Scenario {
            name: "settings",
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "import",
            panes: &[Pane::Import, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "just imported, offering controls",
            imported: true,
            panes: &[Pane::Import, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "playlist pane, nothing loaded",
            panes: &[Pane::Playlist],
            ..BASE
        },
        Scenario {
            name: "playlist loaded",
            playlist: true,
            panes: &[Pane::Playlist],
            ..BASE
        },
        Scenario {
            name: "playlist with a cue selected and a changed file",
            playlist: true,
            cue_selected: true,
            panes: &[Pane::Playlist, Pane::Browser],
            ..BASE
        },
        Scenario {
            name: "playlist loaded, narrow strip",
            playlist: true,
            panes: &[Pane::Playlist, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "mappings pane, nothing mapped",
            panes: &[Pane::Map],
            ..BASE
        },
        Scenario {
            name: "mappings pane with every state",
            mapped: true,
            panes: &[Pane::Map],
            ..BASE
        },
        Scenario {
            name: "mappings pane with a macro, a mapping and the add form open",
            mapped: true,
            map_open: true,
            panes: &[Pane::Map],
            ..BASE
        },
        Scenario {
            name: "mappings pane, no scene playing",
            mapped: true,
            playing: false,
            panes: &[Pane::Map],
            ..BASE
        },
        Scenario {
            name: "macro strip with unavailable macros and the presets row",
            mapped: true,
            panes: &[Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "palette listing scenes and presets",
            palette: Some("scene"),
            panes: &[Pane::Browser, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "palette with nothing typed",
            palette: Some(""),
            ..BASE
        },
        Scenario {
            name: "library with Synesthesia's scenes, one selected",
            synesthesia: true,
            browser_selected: true,
            panes: &[Pane::Browser, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "compact library with Synesthesia's scenes",
            synesthesia: true,
            density: crate::ui::theme::Density::Compact,
            panes: &[Pane::Browser],
            ..BASE
        },
        Scenario {
            name: "playing Synesthesia's scene, a write refused",
            synesthesia: true,
            on_synesthesia: true,
            browser_selected: true,
            panes: &[Pane::Browser, Pane::Inspector],
            ..BASE
        },
        Scenario {
            name: "editing Synesthesia's scene, read-only",
            synesthesia: true,
            on_synesthesia: true,
            panes: &[Pane::Inspector, Pane::Editor],
            ..BASE
        },
        Scenario {
            name: "settings with Synesthesia's scenes",
            synesthesia: true,
            panes: &[Pane::Settings],
            ..BASE
        },
        Scenario {
            name: "palette listing Synesthesia's scenes",
            synesthesia: true,
            palette: Some("scene-0"),
            panes: &[Pane::Browser],
            ..BASE
        },
        Scenario {
            name: "no panes",
            panes: &[],
            ..BASE
        },
        Scenario {
            name: "every pane",
            panes: &[
                Pane::Browser,
                Pane::Import,
                Pane::Inspector,
                Pane::Diagnostics,
                Pane::Settings,
                Pane::Playlist,
                Pane::Map,
            ],
            ..BASE
        },
        Scenario {
            name: "every pane, playlist tab, compact",
            playlist: true,
            side_tab: crate::ui::side::SideTab::Playlist,
            density: crate::ui::theme::Density::Compact,
            panes: &[
                Pane::Browser,
                Pane::Import,
                Pane::Diagnostics,
                Pane::Settings,
                Pane::Playlist,
                Pane::Map,
                Pane::Editor,
                Pane::Inspector,
            ],
            ..BASE
        },
    ]
}

/// Show control with the content the MAP pane and the macro strip need to
/// show every state: nothing, or one of each.
/// A scene open in the Editor, with an unsaved edit, built in memory (no
/// files are read).
/// The LAN remote: off with one paired phone, or on with a code showing,
/// a phone waiting for approval and two paired. With `desktop`, a full
/// desktop is waiting instead and another holds the controls (D275).
fn sample_lan(on: bool, desktop: bool) -> crate::lan::LanView {
    use strobe_remote::clients::{Access, ClientInfo};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    // A phone that asked as `name` from `platform` at `address` (D500).
    let phone = |id: &str, name: &str, platform: &str, address: [u8; 4]| ClientInfo {
        id: id.to_string(),
        label: format!("{name}, {platform} at {}", std::net::IpAddr::from(address)),
        name: Some(name.to_string()),
        platform: Some(platform.to_string()),
        address: Some(std::net::IpAddr::from(address)),
        access: Access::Phone,
        paired_at: now.saturating_sub(3 * 86_400),
        last_seen: now.saturating_sub(25 * 60),
    };
    if !on {
        return crate::lan::LanView {
            note: "Off".to_string(),
            clients: vec![(
                phone("c00000001", "Lighting desk", "iPhone", [192, 168, 1, 23]),
                false,
            )],
            name: "STAGE-PC".to_string(),
            ..crate::lan::LanView::default()
        };
    }
    crate::lan::LanView {
        running: true,
        url: Some("http://192.168.1.20:47812/".to_string()),
        warning: None,
        note: "On: http://192.168.1.20:47812/".to_string(),
        failed: false,
        requests: vec![strobe_remote::pairing::Request {
            id: 1,
            label: if desktop {
                format!(
                    "{}Strobe desktop at 192.168.1.40",
                    strobe_remote::desktop::LABEL_PREFIX
                )
            } else {
                "Android at 192.168.1.31".to_string()
            },
            since: std::time::Instant::now(),
            decision: strobe_remote::pairing::Decision::Waiting,
        }],
        clients: vec![
            (
                phone("c00000001", "Lighting desk", "iPhone", [192, 168, 1, 23]),
                true,
            ),
            (
                phone("c00000002", "Ana's iPad", "iPad", [192, 168, 1, 24]),
                false,
            ),
        ],
        connected: 2,
        desktop_controlled: desktop,
        venue: false,
        advertising: true,
        name: "STAGE-PC".to_string(),
    }
}

fn sample_doc() -> crate::editor::Doc {
    let shader = "// Aurora\n#define BANDS 4\nvec4 renderMain() {\n    float t = TIME * 0.5;\n    \
                  vec3 c = vec3(0.2, 0.5, 0.9) * (0.5 + 0.5 * sin(_uv.x * 6.0 + t));\n    \
                  return vec4(c, 1.0);\n}\n";
    let mut doc = crate::editor::Doc::in_memory(
        "scenes/02-scene-1",
        "02-scene-1",
        &[
            ("main.glsl", shader),
            ("scene.json", "{ \"TITLE\": \"Aurora\" }"),
        ],
    );
    if let Some(tab) = doc.tabs.first_mut() {
        tab.text.push_str("// edited\n");
    }
    doc
}

/// A worker's result for the review card.
fn sample_generated() -> crate::generate::Outcome {
    use crate::generate::{Kept, Outcome, Summary};
    Outcome::Done(Summary {
        scene: "Aurora Field".to_string(),
        source: PathBuf::from("scenes/01-scene-0.synScene"),
        written: Some(PathBuf::from("scenes/01-scene-0-auto.synScene")),
        context: "headless EGL device | llvmpipe".to_string(),
        candidates: 9,
        fidelity: Some(0.0),
        kept: vec![
            Kept {
                candidate: 0,
                name: "speed".to_string(),
                group: "Motion".to_string(),
                kind: "time scale".to_string(),
                default: vec![0.5],
                min: vec![0.0],
                max: vec![2.0],
                score: Some(0.21),
                context: "float t = TIME * 0.5;".to_string(),
            },
            Kept {
                candidate: 0,
                name: "tint".to_string(),
                group: "Colour".to_string(),
                kind: "colour".to_string(),
                default: vec![0.8, 0.3, 0.1],
                min: vec![0.0, 0.0, 0.0],
                max: vec![1.0, 1.0, 1.0],
                score: Some(0.08),
                context: "vec3 tint = vec3(0.8, 0.3, 0.1);".to_string(),
            },
        ],
        suggestions: vec!["speed with syn_BassLevel: motion follows the low end".to_string()],
    })
}

fn sample_remote(mapped: bool) -> Result<Remote> {
    use strobe_io::{Mapping, Source, Target};
    let dir = std::env::temp_dir().join(format!("strobe-shell-tests-{}", std::process::id()));
    let overrides = Overrides {
        workers: false,
        read_only: true,
        ..Overrides::default()
    };
    let mut remote = Remote::new(&dir, &overrides, std::time::Instant::now());
    if !mapped {
        return Ok(remote);
    }
    let cc = |controller: u8| Source::MidiCc {
        device: Some("nanoKONTROL2".into()),
        channel: 1,
        controller,
    };
    let engine = &mut remote.engine;
    engine
        .mappings
        .push(Mapping::new(cc(0), Target::control("speed")));
    engine.mappings.push(Mapping::new(
        cc(1),
        Target::control("missing in this scene"),
    ));
    engine
        .mappings
        .push(Mapping::new(cc(2), Target::command("not_a_command")));
    engine
        .mappings
        .push(Mapping::new(cc(3), Target::meta("hue")));
    engine
        .mappings
        .push(Mapping::new(cc(3), Target::control("warp")));
    engine.mappings.push(Mapping::new(
        Source::Osc {
            address: "/1/fader1".into(),
        },
        Target::command("next_scene"),
    ));
    let mut off = Mapping::new(cc(5), Target::control("warp"));
    off.enabled = false;
    engine.mappings.push(off);
    engine
        .mappings
        .push(Mapping::new(cc(6), Target::control("tint")));
    let speed = engine.macros.pin("Speed", Target::control("speed"));
    // A colour and a reach, so the card's band is drawn at every size (D210).
    engine.macros.set_color(speed, Some(2));
    engine.macros.set_range(speed, Some([0.2, 0.8]));
    engine.macros.set_key(speed, Some("Q".to_string()));
    engine.macros.pin("Hue", Target::meta("hue"));
    engine.macros.pin("Glow", Target::control("glow"));
    engine.macros.pin("Next", Target::command("next_scene"));
    engine.macros.pin("Cue 2", Target::Cue { number: 2 });
    engine
        .mappings
        .push(Mapping::new(cc(7), Target::Macro { id: speed }));
    engine.arm_relearn(5);
    // What a real run puts in the monitor: the long lines are the ones that
    // once pushed the pane wider than its dock.
    remote.osc_state = crate::remote::OscState::Listening("127.0.0.1:9000".parse()?);
    remote.activity.note(
        1.5,
        "/strobe/control/hue_shift 0.800".into(),
        "applied".into(),
        None,
    );
    remote.activity.note(
        2.5,
        "/strobe/control/nope 0.500".into(),
        "unavailable: \"nope\" is not a control of the scene that is playing".into(),
        None,
    );
    remote.activity.note(
        3.5,
        "CC 21 = 64  ch 1  [Launchpad Mini MIDI 1]".into(),
        "waiting for pickup".into(),
        None,
    );
    remote.favs.set(
        0,
        strobe_scene::preset::Fav {
            scene: "01-aurora".into(),
            preset: Some("Slow burn".into()),
        },
    );
    Ok(remote)
}

/// A scene with one control of every kind, in two groups.
fn controls() -> Result<ControlBank> {
    let json = r#"{ "CONTROLS": [
        { "NAME": "speed", "TYPE": "slider", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 0.7, "UI_GROUP": "Motion" },
        { "NAME": "warp", "TYPE": "knob", "UI_GROUP": "Motion" },
        { "NAME": "invert", "TYPE": "toggle", "UI_GROUP": "Colour" },
        { "NAME": "flash", "TYPE": "bang", "UI_GROUP": "Colour" },
        { "NAME": "centre", "TYPE": "xy" },
        { "NAME": "tint", "TYPE": "color" },
        { "NAME": "mode", "TYPE": "dropdown", "LABELS": ["a", "b"], "VALUES": [0, 1] }
    ] }"#;
    Ok(ControlBank::new(SceneDef::parse(json, "t")?.controls)?)
}

/// A three-cue show over `library`, never compiled: its second scene is in
/// the library, its third is not (so one row is drawn as skipped).
fn sample_show(library: &Library) -> Result<crate::show::Show> {
    use strobe_playlist::{Cue, DurationSpec, Playlist, SceneRef, Section};
    let mut playlist = Playlist::new("Example show");
    let mut section = Section::new("Morning");
    for name in ["01-scene-0", "02-scene-1", "ghost"] {
        let mut cue = Cue::new(SceneRef::Library(name.to_string()));
        cue.settings.duration = Some(DurationSpec::Seconds(180.0));
        section.cues.push(cue);
    }
    playlist.sections.push(section);
    Ok(crate::show::Show::new(None, playlist, library, None, None))
}

/// What a drawn scenario leaves behind.
struct Drawn {
    ctx: egui::Context,
    actions: Vec<Action>,
    /// Every frame's actions, in order.
    all_actions: Vec<Action>,
    shapes: Vec<egui::epaint::ClippedShape>,
    /// The panes open after the last frame.
    open_panes: Vec<Pane>,
    /// The main view after the last frame.
    main: Option<Pane>,
    /// Every frame's texture changes, for [`shoot`]; cleared on drop.
    textures: egui::TexturesDelta,
}

impl Drop for Drawn {
    fn drop(&mut self) {
        self.textures.clear();
    }
}

/// Draws `scenario` at `size` for `frames` frames.
fn draw(scenario: Scenario, size: (f32, f32), frames: usize) -> Result<Drawn> {
    draw_events(scenario, size, vec![Vec::new(); frames])
}

/// Draws `scenario` at `size`, one frame per entry of `frames`, each with
/// its input events.
fn draw_events(
    scenario: Scenario,
    size: (f32, f32),
    frames: Vec<Vec<egui::Event>>,
) -> Result<Drawn> {
    let ctx = egui::Context::default();
    crate::ui::theme::apply(&ctx, crate::palette::fallback(), scenario.density);
    if scenario.credits_open {
        crate::ui::credits::open_crates(&ctx);
    }
    let mut library = Library {
        dir: PathBuf::from("scenes"),
        entries: (0..scenario.scenes)
            .map(|i| {
                let entry = Entry::new(format!("{:02}-scene-{i}", i + 1), PathBuf::from("x"));
                // The playing scene carries tags, so the Inspector's chips
                // are measured at every size.
                match i {
                    0 => {
                        entry.with_tags(&["dark", "tunnel", "audio reactive", "3d", "raymarching"])
                    }
                    1 => entry.with_author("Ada Lovelace"),
                    2 => entry.with_author("An author credit long enough to be cut short"),
                    _ => entry,
                }
            })
            .collect(),
    };
    if scenario.synesthesia {
        let tags: [&[&str]; 3] = [&["dark", "feedback"], &["warm", "2d"], &[]];
        for (name, tags) in ["01-scene-0", "glow", "ink"].into_iter().zip(tags) {
            library.entries.push(
                Entry::new(
                    name.to_string(),
                    PathBuf::from(format!(
                        "C:/Program Files (x86)/Synesthesia/release/{name}.synScene"
                    )),
                )
                .with_tags(tags)
                .from_source(crate::library::Source::Synesthesia),
            );
        }
        // One from a scene folder in Synesthesia's own settings: the
        // person's, so writable, and labelled the same.
        library.entries.push(
            Entry::new(
                "deep-sea".to_string(),
                PathBuf::from("D:/VJ/Synesthesia scenes/packs/deep-sea.synScene"),
            )
            .from_source(crate::library::Source::SynesthesiaFolder),
        );
    }
    // Synesthesia's `01-scene-0` when the scenario is on it.
    let playing_index = if scenario.on_synesthesia {
        scenario.scenes
    } else {
        0
    };
    let copy_action = Action::CopyToLibrary(PathBuf::from(
        "C:/Program Files (x86)/Synesthesia/release/01-scene-0.synScene",
    ));
    let health: Vec<Health> = (0..library.entries.len())
        .map(|i| {
            if scenario.failed && i == 1 {
                Health::Failed("main.glsl:42: unknown identifier `foo`".to_string())
            } else if i == 0 {
                Health::Ready
            } else {
                Health::Unknown
            }
        })
        .collect();
    let mut bank = controls()?;
    let mut meta_bank = strobe_scene::meta::bank()?;
    let console = Console::default();
    let stats = FrameStats::default();
    let mut ui_state = UiState::ephemeral();
    ui_state.panes.close_all();
    for pane in scenario.panes {
        ui_state.panes.open(*pane);
    }
    ui_state.hotkeys.capturing = scenario.capturing;
    // Settings shows one page: the one the scenario's state is on.
    ui_state.settings.page = if let Some(page) = scenario.settings_page {
        page
    } else if scenario.capturing.is_some() || scenario.clash {
        crate::ui::settings::SettingsPage::Shortcuts
    } else if scenario.watch.is_some() {
        crate::ui::settings::SettingsPage::Library
    } else if scenario.lan {
        crate::ui::settings::SettingsPage::Remote
    } else {
        crate::ui::settings::SettingsPage::Output
    };
    ui_state.side_tab = scenario.side_tab;
    if scenario.recent {
        for path in [
            "playlists/friday.toml",
            "D:/shows/a very long folder name for the summer festival main stage/closing set.toml",
            "warmup.toml",
        ] {
            ui_state.remember_playlist(path);
        }
    }
    ui_state.browser.cards = scenario.cards;
    if let Some(query) = scenario.palette {
        ui_state.command_palette.open_with(query);
    }
    if scenario.clash {
        ui_state.hotkeys.clash = Some((
            crate::commands::CommandId::TapTempo,
            crate::commands::Chord::plain(crate::commands::Key::B),
            crate::commands::CommandId::ToggleBlackout,
        ));
    }
    let mut scale = 1.0_f32;
    let mut reactivity = 1.0_f32;
    let mut speed = 1.0_f32;
    let mut import_text = String::new();
    let choice = DeviceChoice::DefaultInput;
    let script = scenario.script.map(|kind| {
        if kind == "fault" {
            ScriptState::Faulted {
                consecutive: 2,
                message: "ReferenceError: nope is not defined (script.js:12)".to_string(),
            }
        } else {
            ScriptState::Disabled {
                message: "three faults in a row".to_string(),
            }
        }
    });
    let image = Preview {
        texture: egui::TextureId::default(),
        aspect: 16.0 / 9.0,
        size: [1280, 720],
    };
    let status = Status::Idle;
    let monitors = vec!["1: Display (1920x1080)".to_string()];
    let mut playlist_ui = PlaylistUi::default();
    if scenario.cue_selected {
        playlist_ui.selected = Some(1);
        playlist_ui.dirty = true;
        playlist_ui.external_change = Some("playlists/example.toml".to_string());
        ui_state.browser.selected = Some(2);
    }
    if let Some((failed, note)) = scenario.watch {
        ui_state.watch.draft = if failed {
            "D:\\VJ\\scenes".to_string()
        } else {
            "D:\\VJ\\Strobe inbox".to_string()
        };
        ui_state.watch.watching = !failed;
        ui_state.watch.failed = failed;
        ui_state.watch.note = note.to_string();
    }
    if scenario.browser_selected || scenario.tagging {
        ui_state.browser.selected = Some(if scenario.synesthesia {
            scenario.scenes
        } else {
            2
        });
    }
    if scenario.ordered {
        ui_state.browser.sort = crate::library::SortKey::Author;
        ui_state.browser.descending = true;
        ui_state.browser.filter = crate::library::HealthFilter::Favourites;
        for name in ["02-scene-1", "03-scene-2", "04-scene-3"] {
            ui_state.browser.toggle_favourite(name);
        }
    }
    if scenario.synesthesia {
        ui_state.synesthesia.note = "4 scenes from C:\\Program Files (x86)\\Synesthesia and \
                                     your scene folder in its settings. Its own are read-only."
            .to_string();
    }
    if scenario.tagging {
        let draft = |scene: &str| {
            Some(crate::ui::tags::TagDraft {
                scene: scene.to_string(),
                text: "dark, tunnel, audio reactive, 3d".to_string(),
            })
        };
        ui_state.tag_draft = draft("01-scene-0");
        ui_state.import_tag_draft = draft("01-scene-0");
        ui_state.browser_tag_draft = draft("03-scene-2");
    }
    if scenario.generate {
        playlist_ui.generate_open = true;
        playlist_ui.form.exclude = "tunnel, heavy".to_string();
        let snapshot = crate::show::generate::snapshot(&library);
        let spec = playlist_ui.form.spec().map_err(anyhow::Error::msg)?;
        playlist_ui.excluded = crate::show::generate::excluded(&snapshot, &spec);
        playlist_ui.preview =
            Some(strobe_playlist::generate(&snapshot, &spec).map_err(|error| error.to_string()));
    }
    let mut map_ui = MapUi {
        key_capture: scenario.macro_capture,
        ..MapUi::default()
    };
    if scenario.map_open {
        map_ui.selected = Some(0);
        map_ui.selected_macro = Some(1);
        map_ui.add.open = true;
        map_ui.add.as_macro = true;
    }
    let bindings = crate::commands::Bindings::default();
    let mut remote = sample_remote(scenario.mapped)?;
    let presets = vec!["Slow burn".to_string(), "Peak".to_string()];
    let sample_show_state = scenario
        .playlist
        .then(|| sample_show(&library))
        .transpose()?;
    let generated = scenario.generated.then(sample_generated);
    let mut editor_doc = scenario.panes.contains(&Pane::Editor).then(sample_doc);
    if scenario.editor_invalid
        && let Some(doc) = editor_doc.as_mut()
    {
        if let Some(tab) = doc.tabs.get_mut(1) {
            tab.text = "{ \"TITLE\": \"Aurora\"\n  \"WIDTH\": 640 }".to_string();
        }
        doc.invalid = doc.check().err();
        assert!(
            doc.invalid.is_some(),
            "the sample scene.json must not parse"
        );
    }
    let generate_panel = GeneratePanel {
        running: scenario.generating.then_some(("01-scene-0", 12.0)),
        outcome: generated.as_ref(),
        authored: false,
        read_only: false,
    };
    let lan_view = sample_lan(scenario.lan, scenario.desktop_access);
    let mut lan_address = String::new();
    let projector_view = crate::projectors::ProjectorView::default();
    let mut projector_form = crate::projectors::Form::default();
    // Two Strobes on the LAN for the Remote page to list (D468).
    let found = if scenario.lan {
        vec![
            strobe_remote::discovery::Found {
                id: "a".into(),
                name: "FOH-LAPTOP".into(),
                addr: std::net::SocketAddr::from(([192, 168, 1, 31], 47812)),
                desktop: true,
                seen: std::time::Instant::now(),
            },
            strobe_remote::discovery::Found {
                id: "b".into(),
                name: "BOOTH-PC".into(),
                addr: std::net::SocketAddr::from(([192, 168, 1, 32], 47812)),
                desktop: false,
                seen: std::time::Instant::now(),
            },
        ]
    } else {
        Vec::new()
    };
    let mut lan_port = "47812".to_string();
    let mut desktop_controller = crate::desktop::Controller::default();
    if scenario.reveal_connect {
        desktop_controller.reveal_form();
    }
    if scenario.trust_form {
        desktop_controller.use_trust = true;
        desktop_controller.trust_path = "C:\\Users\\you\\venue.trust".to_string();
    }
    let mut shell_state = ShellState::default();
    let mut actions = Vec::new();
    let mut all_actions = Vec::new();
    let mut shapes = Vec::new();
    let mut textures = egui::TexturesDelta::default();
    let screen = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(size.0, size.1));
    for events in frames {
        actions.clear();
        if let Some(entry) = scenario.focus_card.and_then(|i| library.entries.get(i)) {
            // The id the card registers (browser::card), so it holds the
            // keyboard focus as a Tab to it would leave it.
            let id = egui::Id::unique(("scene-card", &entry.name, entry.dir.as_path()));
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
        let mut model = Model {
            ui: &mut ui_state,
            library: &library,
            health: &health,
            library_rev: 1,
            scan_error: None,
            playing: scenario.playing.then_some(playing_index),
            scene_title: if scenario.playing { "Aurora Field" } else { "" },
            scene_transitions: scenario.playing,
            controls: scenario.playing.then_some(&mut bank),
            media: scenario.playing.then_some(crate::ui::MediaPanel {
                showing: Some(std::path::Path::new("loops/rain")),
                from: crate::ui::MediaFrom::Chosen,
                loading: false,
                read: true,
                chosen: true,
            }),
            meta: scenario.playing.then_some(&mut meta_bank),
            preview: scenario.playing.then_some(image),
            scene_preview: scenario.previewing.then_some(ScenePreview {
                index: 2,
                name: "03-scene-2",
                image,
            }),
            loading: scenario.loading.then_some("02-scene-1"),
            failure: scenario.failed.then_some(FailurePanel {
                scene: "02-scene-1",
                summary: "main.glsl:42: unknown identifier `foo`",
                kept: scenario.playing.then_some("01-scene-0"),
            }),
            drop_hint: None,
            notice: if scenario.on_synesthesia {
                Some(NoticePanel {
                    tone: Tone::Warning,
                    text: "01-scene-0 is Synesthesia's own and read-only. Copy it to your \
                           library to tag it.",
                    offer: Some(Offer {
                        label: crate::ui::tags::COPY_LABEL,
                        action: &copy_action,
                    }),
                })
            } else {
                (scenario.notice || scenario.long_notice).then_some(NoticePanel {
                    tone: Tone::Info,
                    text: if scenario.long_notice {
                        "The graphics context was lost and rebuilt on ANGLE in 181 ms. Two \
                         frames were dropped, the scene was reloaded from its folder and its \
                         controls kept their values; the output window stayed where it was. If \
                         it happens again in the next minute the app stays on ANGLE until you \
                         restart it, which is slower on this machine but does not lose the \
                         context."
                    } else {
                        "The graphics context was lost and rebuilt on ANGLE in 181 ms."
                    },
                    offer: None,
                })
            },
            transport: TransportPanel {
                now: scenario.playing.then_some("01-scene-0"),
                next: (scenario.scenes > 1).then_some("02-scene-1"),
                cue: scenario.playlist.then_some(CueTransport {
                    index: 1,
                    total: 3,
                    section: "Morning",
                    elapsed: 41.0,
                    remaining: Some(139.0),
                    held: true,
                    transition: Some(0.4),
                    waiting: None,
                }),
                scene_time: 41.0,
                blackout: scenario.blackout,
                can_step: scenario.scenes > 1,
            },
            console: &console,
            stats: &stats,
            render_scale: &mut scale,
            monitors: &monitors,
            output_monitor: Some(0),
            output_visible: scenario.output_visible,
            fullscreen: false,
            maximized: false,
            gl_summary: "native | llvmpipe | 4.5 Mesa",
            audio: AudioPanel {
                source: if scenario.audio_live {
                    "live: Default input"
                } else {
                    "synthetic (no input device)"
                },
                live: scenario.audio_live,
                can_capture: scenario.audio_capture,
                devices: &[],
                choice: &choice,
                reactivity: &mut reactivity,
                speed: &mut speed,
                bpm: 128.0,
                input_rms: 0.05,
            },
            script: script.as_ref(),
            perf: PerfPanel {
                adaptive: true,
                has_timer: true,
                gpu_ms: Some(2.1),
                budget_ms: 16.7,
                scale: 0.85,
            },
            picking: false,
            import: ImportPanel {
                text: &mut import_text,
                status: &status,
                replace_title: scenario.replace.then_some("fractal-tunnel"),
                busy: false,
                drop_hover: false,
                imported_playing: scenario.imported,
                imported: scenario.imported.then_some(0),
                generating: scenario.generating,
                listener: ListenerPanel {
                    enabled: false,
                    state: "Off",
                    failed: false,
                    token: None,
                },
            },
            playlist: PlaylistPanel {
                ui: &mut playlist_ui,
                show: sample_show_state.as_ref(),
            },
            remote: RemotePanel {
                ui: &mut map_ui,
                remote: &mut remote,
                presets: if scenario.playing { &presets } else { &[] },
                active_preset: scenario.playing.then_some("Slow burn"),
                scene_name: scenario.playing.then_some("01-aurora"),
                bindings: &bindings,
            },
            generate: generate_panel,
            editor: EditorPanel {
                doc: editor_doc.as_mut(),
                playing_dir: None,
                read_only: scenario.on_synesthesia,
            },
            lan: LanPanel {
                desktop_enabled: scenario.desktop_access,
                controller: &mut desktop_controller,
                view: &lan_view,
                address: &mut lan_address,
                port: &mut lan_port,
                found: &found,
            },
            projectors: crate::ui::model::ProjectorPanel {
                view: &projector_view,
                form: &mut projector_form,
            },
        };
        let pass = crate::ui::testing::run_keeping(
            &ctx,
            crate::ui::testing::input(screen.size(), events),
            |ui| show(ui, &mut shell_state, &mut model, &mut actions),
            &mut textures,
        );
        shapes = pass.shapes;
        all_actions.extend(actions.iter().cloned());
    }
    let open_panes = Pane::ALL
        .into_iter()
        .filter(|pane| ui_state.panes.is_open(*pane))
        .collect();
    Ok(Drawn {
        ctx,
        actions,
        all_actions,
        shapes,
        open_panes,
        main: ui_state.panes.shown_in(crate::workspace::Dock::Main),
        textures,
    })
}

#[test]
fn every_state_draws_at_every_size_without_overflowing_the_window() -> Result<()> {
    for scenario in scenarios() {
        for size in SIZES.iter().copied().chain(SMALL_SIZES) {
            // Several frames: egui lays some things out over two passes.
            let drawn = draw(scenario, size, 3)?;
            assert!(
                !drawn.shapes.is_empty(),
                "{} at {size:?} drew nothing",
                scenario.name
            );
            let used = drawn.ctx.globally_used_rect();
            assert!(
                used.max.x <= size.0 + 0.5 && used.max.y <= size.1 + 0.5,
                "{} at {size:?} wants {used:?}: it overflows the window",
                scenario.name
            );
            for clipped in &drawn.shapes {
                // An empty text (an unfilled search field) has no ink, and
                // egui reports its bounds as an inverted infinite box.
                if let egui::Shape::Text(text) = &clipped.shape
                    && text.galley.rows.iter().all(|row| row.glyphs.is_empty())
                {
                    continue;
                }
                let bounds = clipped.shape.visual_bounding_rect();
                let nan = [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y]
                    .iter()
                    .any(|v| v.is_nan());
                assert!(
                    !nan,
                    "{} at {size:?} painted a NaN shape: {:?}",
                    scenario.name, clipped.shape
                );
            }
        }
    }
    Ok(())
}

#[test]
fn an_idle_frame_asks_for_no_actions() -> Result<()> {
    for scenario in scenarios() {
        let drawn = draw(scenario, (1440.0, 900.0), 3)?;
        assert!(
            drawn.actions.is_empty(),
            "{} produced actions with nobody touching it: {:?}",
            scenario.name,
            drawn.actions
        );
    }
    Ok(())
}

/// A key press, with modifiers.
fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Ctrl+K.
fn ctrl_k() -> egui::Event {
    press(egui::Key::K, egui::Modifiers::COMMAND)
}

/// Typing `text`.
fn typed(text: &str) -> egui::Event {
    egui::Event::Text(text.to_owned())
}

/// Ctrl+K, then (a frame later, once the field has focus) `text`, then
/// Enter, then a quiet frame.
fn palette_run(text: &str) -> Vec<Vec<egui::Event>> {
    vec![
        vec![ctrl_k()],
        Vec::new(),
        vec![typed(text)],
        vec![press(egui::Key::Enter, egui::Modifiers::NONE)],
        Vec::new(),
    ]
}

/// A key let go.
fn release(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn a_macro_key_drives_its_macro_while_held() -> Result<()> {
    let drawn = draw_events(
        Scenario {
            mapped: true,
            ..BASE
        },
        (1440.0, 900.0),
        vec![
            vec![press(egui::Key::Q, egui::Modifiers::NONE)],
            Vec::new(),
            vec![release(egui::Key::Q)],
        ],
    )?;
    let keys: Vec<&Action> = drawn
        .all_actions
        .iter()
        .filter(|a| matches!(a, Action::MacroKey { .. }))
        .collect();
    assert!(
        matches!(
            keys.as_slice(),
            [
                Action::MacroKey { down: true, .. },
                Action::MacroKey { down: false, .. }
            ]
        ),
        "down then up: {:?}",
        drawn.all_actions
    );
    Ok(())
}

/// The macro edges among `actions`, as `true` for down.
fn macro_edges(actions: &[Action]) -> Vec<bool> {
    actions
        .iter()
        .filter_map(|action| {
            if let Action::MacroKey { down, .. } = action {
                Some(*down)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn a_window_that_loses_focus_lets_its_held_macros_go() -> Result<()> {
    // Q is held and the window loses focus (Alt+Tab, a click on the
    // output). macOS and Wayland report no release for it, and egui only
    // forgets the key, so without this the macro stays up (D401).
    let drawn = draw_events(
        Scenario {
            mapped: true,
            ..BASE
        },
        (1440.0, 900.0),
        vec![
            vec![press(egui::Key::Q, egui::Modifiers::NONE)],
            Vec::new(),
            vec![egui::Event::WindowFocused(false)],
            Vec::new(),
        ],
    )?;
    assert_eq!(
        macro_edges(&drawn.all_actions),
        [true, false],
        "down, then up on focus loss: {:?}",
        drawn.all_actions
    );
    // Windows and X11 send the release as well: still one release.
    let drawn = draw_events(
        Scenario {
            mapped: true,
            ..BASE
        },
        (1440.0, 900.0),
        vec![
            vec![press(egui::Key::Q, egui::Modifiers::NONE)],
            vec![release(egui::Key::Q), egui::Event::WindowFocused(false)],
            Vec::new(),
        ],
    )?;
    assert_eq!(
        macro_edges(&drawn.all_actions),
        [true, false],
        "one release, not two: {:?}",
        drawn.all_actions
    );
    Ok(())
}

#[test]
fn a_macro_release_is_never_dropped_in_a_crowded_frame() -> Result<()> {
    // Q is held; then one frame carries more key edges than a frame
    // handles (a stalled control frame, a remote batch) with Q's release
    // last. Presses past the cap may go; the release may not (D402).
    let crowd = [
        egui::Key::A,
        egui::Key::C,
        egui::Key::D,
        egui::Key::G,
        egui::Key::H,
        egui::Key::I,
        egui::Key::J,
        egui::Key::M,
        egui::Key::N,
    ];
    let mut busy: Vec<egui::Event> = crowd
        .iter()
        .map(|key| press(*key, egui::Modifiers::NONE))
        .collect();
    busy.push(release(egui::Key::Q));
    let drawn = draw_events(
        Scenario {
            mapped: true,
            ..BASE
        },
        (1440.0, 900.0),
        vec![
            vec![press(egui::Key::Q, egui::Modifiers::NONE)],
            busy,
            Vec::new(),
        ],
    )?;
    assert_eq!(
        macro_edges(&drawn.all_actions),
        [true, false],
        "the release past the cap still lets the macro go: {:?}",
        drawn.all_actions
    );
    Ok(())
}

#[test]
fn a_macro_key_let_go_while_a_text_field_has_focus_still_lets_the_macro_go() -> Result<()> {
    // Q is held (the macro goes up), Ctrl+K opens the palette, whose field
    // takes focus, and Q is let go while the field has it.
    let drawn = draw_events(
        Scenario {
            mapped: true,
            ..BASE
        },
        (1440.0, 900.0),
        vec![
            vec![press(egui::Key::Q, egui::Modifiers::NONE)],
            Vec::new(),
            vec![ctrl_k()],
            Vec::new(),
            vec![release(egui::Key::Q)],
            Vec::new(),
        ],
    )?;
    let keys: Vec<&Action> = drawn
        .all_actions
        .iter()
        .filter(|a| matches!(a, Action::MacroKey { .. }))
        .collect();
    assert!(
        matches!(
            keys.as_slice(),
            [
                Action::MacroKey { down: true, .. },
                Action::MacroKey { down: false, .. }
            ]
        ),
        "the release reaches the macro, or it stays up: {:?}",
        drawn.all_actions
    );
    // A press into the field is typing, never a macro.
    let typing = draw_events(
        Scenario {
            mapped: true,
            ..BASE
        },
        (1440.0, 900.0),
        vec![
            vec![ctrl_k()],
            Vec::new(),
            vec![press(egui::Key::Q, egui::Modifiers::NONE), typed("q")],
            vec![release(egui::Key::Q)],
            Vec::new(),
        ],
    )?;
    assert!(
        !typing
            .all_actions
            .iter()
            .any(|a| matches!(a, Action::MacroKey { .. })),
        "Q typed into the palette drives no macro: {:?}",
        typing.all_actions
    );
    Ok(())
}

#[test]
fn a_key_pressed_while_capturing_rebinds_and_fires_nothing() -> Result<()> {
    let scenario = Scenario {
        capturing: Some(crate::commands::CommandId::TapTempo),
        panes: &[Pane::Settings],
        ..BASE
    };
    let drawn = draw_events(
        scenario,
        (1440.0, 900.0),
        vec![
            vec![press(egui::Key::B, egui::Modifiers::COMMAND)],
            Vec::new(),
        ],
    )?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::SetHotkey {
            command: crate::commands::CommandId::TapTempo,
            chord: Some(crate::commands::Chord::command(crate::commands::Key::B)),
            steal: false,
        }],
        "Ctrl+B becomes tap tempo's new key; nothing else runs"
    );
    let cancelled = draw_events(
        Scenario {
            capturing: Some(crate::commands::CommandId::TapTempo),
            panes: &[Pane::Settings],
            ..BASE
        },
        (1440.0, 900.0),
        vec![
            vec![press(egui::Key::Escape, egui::Modifiers::NONE)],
            vec![press(egui::Key::B, egui::Modifiers::NONE)],
        ],
    )?;
    assert_eq!(
        cancelled.all_actions,
        vec![Action::SetBlackout(true)],
        "Esc cancels without closing a pane; the next B is blackout again"
    );
    Ok(())
}

#[test]
fn a_capture_whose_field_is_gone_gives_the_keyboard_back() -> Result<()> {
    // Change was clicked for tap tempo, then the Settings pane was closed
    // with its own button before a key came: the field that waited for the
    // key is no longer drawn, so nothing can end the capture there.
    let settings_closed = Scenario {
        capturing: Some(crate::commands::CommandId::TapTempo),
        ..BASE
    };
    let drawn = draw_events(
        settings_closed,
        (1440.0, 900.0),
        vec![vec![press(egui::Key::B, egui::Modifiers::NONE)], Vec::new()],
    )?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::SetBlackout(true)],
        "B is blackout again, not a dead key"
    );
    // The same for a macro's key, Set and then the macro deselected (or
    // MAP closed).
    let macro_deselected = Scenario {
        macro_capture: Some(1),
        mapped: true,
        ..BASE
    };
    let drawn = draw_events(
        macro_deselected,
        (1440.0, 900.0),
        vec![vec![press(egui::Key::B, egui::Modifiers::NONE)], Vec::new()],
    )?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::SetBlackout(true)],
        "B is blackout again after a macro's key capture lost its field"
    );
    Ok(())
}

#[test]
fn the_palette_runs_a_command_by_name() -> Result<()> {
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("black"))?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::SetBlackout(true)],
        "Ctrl+K, \"black\", Enter is the same as pressing B"
    );
    Ok(())
}

#[test]
fn typing_in_the_palette_never_fires_a_shortcut() -> Result<()> {
    // B is Blackout and T is Tap tempo; typed into the palette they are
    // letters, and the arrows move the highlight instead of the scene.
    let frames = vec![
        vec![ctrl_k()],
        Vec::new(),
        vec![
            press(egui::Key::B, egui::Modifiers::NONE),
            typed("b"),
            press(egui::Key::T, egui::Modifiers::NONE),
            typed("t"),
            press(egui::Key::ArrowRight, egui::Modifiers::NONE),
            press(egui::Key::ArrowDown, egui::Modifiers::NONE),
        ],
        Vec::new(),
    ];
    let drawn = draw_events(BASE, (1440.0, 900.0), frames)?;
    assert!(
        drawn.all_actions.is_empty(),
        "nothing runs while typing; got {:?}",
        drawn.all_actions
    );
    Ok(())
}

#[test]
fn escape_closes_the_palette_and_gives_the_keyboard_back() -> Result<()> {
    let frames = vec![
        vec![ctrl_k()],
        Vec::new(),
        vec![typed("hold")],
        vec![press(egui::Key::Escape, egui::Modifiers::NONE)],
        Vec::new(),
        vec![press(egui::Key::B, egui::Modifiers::NONE)],
    ];
    let drawn = draw_events(BASE, (1440.0, 900.0), frames)?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::SetBlackout(true)],
        "Escape ran nothing, and B afterwards is a shortcut again"
    );
    assert_eq!(
        drawn.open_panes,
        vec![Pane::Browser, Pane::Inspector],
        "Escape closed the palette, not the panes behind it"
    );
    Ok(())
}

#[test]
fn ctrl_k_toggles_the_palette() -> Result<()> {
    let frames = vec![
        vec![ctrl_k()],
        Vec::new(),
        vec![ctrl_k()],
        Vec::new(),
        vec![press(egui::Key::B, egui::Modifiers::NONE)],
    ];
    let drawn = draw_events(BASE, (1440.0, 900.0), frames)?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::SetBlackout(true)],
        "the second Ctrl+K closed it, so B is Blackout again"
    );
    Ok(())
}

#[test]
fn an_unavailable_command_is_listed_but_does_not_run() -> Result<()> {
    // BASE has no playlist, so Hold needs one.
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("hold"))?;
    assert!(
        drawn.all_actions.is_empty(),
        "Hold without a playlist does nothing; got {:?}",
        drawn.all_actions
    );
    let with_playlist = Scenario {
        playlist: true,
        ..BASE
    };
    let drawn = draw_events(with_playlist, (1440.0, 900.0), palette_run("hold"))?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::ToggleHold],
        "with a playlist it runs"
    );
    Ok(())
}

#[test]
fn the_palette_opens_panes_and_survives_an_empty_result() -> Result<()> {
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("show mappings"))?;
    assert!(
        drawn.open_panes.contains(&Pane::Map),
        "the Mappings pane opened; open: {:?}",
        drawn.open_panes
    );
    let drawn = draw_events(BASE, (640.0, 480.0), palette_run("qqqq"))?;
    assert!(
        drawn.all_actions.is_empty(),
        "no match, and Enter on nothing does nothing"
    );
    Ok(())
}

#[test]
fn the_palette_loads_previews_and_applies_presets_by_name() -> Result<()> {
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("03-scene-2"))?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::Load(2)],
        "a scene's name, then Enter, loads it"
    );
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("preview 04"))?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::Preview(3)],
        "Preview and part of the name previews it"
    );
    assert!(
        drawn.open_panes.contains(&Pane::Browser),
        "the browser opens to show the preview; open: {:?}",
        drawn.open_panes
    );
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("peak"))?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::RecallPreset("Peak".to_string())],
        "a preset of the playing scene is applied"
    );
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("01-scene-0"))?;
    assert!(
        drawn.all_actions.is_empty(),
        "the playing scene is listed but not loaded again; got {:?}",
        drawn.all_actions
    );
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("black"))?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::SetBlackout(true)],
        "an exact command still comes before every scene"
    );
    Ok(())
}

/// Paints `drawn` on a headless GL context and writes it to `path` as a PNG.
/// Returns false (after saying why) when there is no headless context.
fn shoot(drawn: &mut Drawn, size: (f32, f32), path: &std::path::Path) -> Result<bool> {
    let headless = match strobe_gfx::Headless::new(strobe_gfx::Api::DesktopCore33) {
        Ok(headless) => headless,
        Err(error) => {
            strobe_gfx::skip::note(format_args!(
                "no headless context for a screenshot ({error})"
            ));
            return Ok(false);
        }
    };
    let gl = headless.gl();
    let (width, height) = (size.0.round() as i32, size.1.round() as i32);
    let target = gl
        .texture_2d(
            width,
            height,
            strobe_gfx::gl::Format::Rgba8,
            strobe_gfx::gl::Sampling::LINEAR_CLAMP,
            None,
        )
        .map_err(anyhow::Error::msg)?;
    let fbo = gl.framebuffer(&target).map_err(anyhow::Error::msg)?;
    let mut painter = egui_glow::Painter::new(gl.glow(), "", None, true)
        .map_err(|e| anyhow::anyhow!("egui painter: {e}"))?;
    let primitives = drawn.ctx.tessellate(drawn.shapes.clone(), 1.0);
    let screen = [width as u32, height as u32];
    gl.bind_framebuffer(Some(&fbo));
    painter.clear(screen, [0.0, 0.0, 0.0, 1.0]);
    painter.paint_and_update_textures(screen, 1.0, &primitives, &mut drawn.textures);
    let pixels = gl.read_rgba8(&fbo, width, height);
    painter.destroy();
    let row = width as usize * 4;
    let flipped: Vec<u8> = pixels.chunks_exact(row).rev().flatten().copied().collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::save_buffer(
        path,
        &flipped,
        width as u32,
        height as u32,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(true)
}

/// Writes every scenario at every size, and at the logical sizes a 150%
/// display gives small windows, to the folder `STROBE_UI_SHOTS` names.
/// Without the variable it does nothing: it is for looking, and the
/// assertions above and below are the test.
#[test]
fn screenshots_when_asked() -> Result<()> {
    let Some(dir) = std::env::var_os("STROBE_UI_SHOTS") else {
        return Ok(());
    };
    let dir = PathBuf::from(dir);
    let sizes = SIZES.iter().copied().chain(SMALL_SIZES);
    for size in sizes.clone() {
        for (name, how) in VIEWER_STATES {
            let mut drawn = draw_viewer(how, size, 8)?;
            let file = format!("{name}-{}x{}.png", size.0, size.1);
            if !shoot(&mut drawn, size, &dir.join(file))? {
                return Ok(());
            }
        }
    }
    for size in sizes {
        for scenario in scenarios() {
            // Enough frames for a drawer to finish fading in, so the picture
            // is the settled one.
            let mut drawn = draw(scenario, size, 20)?;
            let name = format!("{}-{}x{}.png", scenario.name, size.0, size.1);
            if !shoot(&mut drawn, size, &dir.join(name))? {
                return Ok(());
            }
        }
    }
    Ok(())
}

/// The states the desktop viewer draws (D275), named for their pictures.
const VIEWER_STATES: [(&str, crate::desktop::Staged); 12] = [
    ("viewer live", crate::desktop::Staged::Live),
    (
        "viewer waiting for approval",
        crate::desktop::Staged::Waiting(12),
    ),
    (
        "viewer reconnecting",
        crate::desktop::Staged::Reconnecting(4),
    ),
    ("viewer host image paused", crate::desktop::Staged::Paused),
    (
        "viewer dropped by the host",
        crate::desktop::Staged::Dropped,
    ),
    (
        "viewer could not connect",
        crate::desktop::Staged::Unreachable,
    ),
    (
        "viewer approval timed out",
        crate::desktop::Staged::TimedOut,
    ),
    (
        "viewer reconnect timed out",
        crate::desktop::Staged::TimedOutReconnect,
    ),
    ("viewer pairing refused", crate::desktop::Staged::Refused),
    (
        "viewer trusted and live",
        crate::desktop::Staged::TrustedLive,
    ),
    (
        "viewer trusted and reconnecting",
        crate::desktop::Staged::Retrying(7),
    ),
    (
        "viewer trust file refused",
        crate::desktop::Staged::TrustRefused,
    ),
];

/// Draws the viewer in `how` at `size` for `frames` frames, as the app does
/// when a controller is active: it replaces the shell.
fn draw_viewer(how: crate::desktop::Staged, size: (f32, f32), frames: usize) -> Result<Drawn> {
    let ctx = egui::Context::default();
    crate::ui::theme::apply(
        &ctx,
        crate::palette::fallback(),
        crate::ui::theme::Density::Comfortable,
    );
    let mut controller = crate::desktop::Controller::staged("192.168.1.20:47812", how);
    let mut actions = Vec::new();
    let mut shapes = Vec::new();
    let mut textures = egui::TexturesDelta::default();
    for _ in 0..frames {
        actions.clear();
        let pass = crate::ui::testing::run_keeping(
            &ctx,
            crate::ui::testing::input(Vec2::new(size.0, size.1), Vec::new()),
            |ui| controller.show(ui, &mut actions),
            &mut textures,
        );
        shapes = pass.shapes;
    }
    Ok(Drawn {
        ctx,
        all_actions: actions.clone(),
        actions,
        shapes,
        open_panes: Vec::new(),
        main: None,
        textures,
    })
}

#[test]
fn every_viewer_state_fits_every_size_without_text_over_text() -> Result<()> {
    let mut failures = Vec::new();
    for size in SIZES.iter().copied().chain(SMALL_SIZES) {
        for (name, how) in VIEWER_STATES {
            let drawn = draw_viewer(how, size, 4)?;
            assert!(!drawn.shapes.is_empty(), "{name} at {size:?} drew nothing");
            let used = drawn.ctx.globally_used_rect();
            assert!(
                used.max.x <= size.0 + 0.5 && used.max.y <= size.1 + 0.5,
                "{name} at {size:?} wants {used:?}: it overflows the window"
            );
            assert!(
                drawn.actions.is_empty(),
                "{name} at {size:?} asked for {:?} with nobody touching it",
                drawn.actions
            );
            let texts = texts(&drawn);
            for (i, (ia, a, ra)) in texts.iter().enumerate() {
                for (ib, b, rb) in texts.iter().skip(i + 1) {
                    let overlap = ra.intersect(*rb);
                    if !overlap.is_positive() || overlap.area() < 4.0 {
                        continue;
                    }
                    let covered = drawn
                        .shapes
                        .get(ia + 1..*ib)
                        .unwrap_or_default()
                        .iter()
                        .any(|between| opaque_over(between, overlap));
                    if !covered {
                        failures.push(format!(
                            "{name} at {size:?}: {a:?} {ra:?} overlaps {b:?} {rb:?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} overlaps:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn the_palette_opens_the_connect_form_from_its_own_entry() -> Result<()> {
    let drawn = draw_events(BASE, (1440.0, 900.0), palette_run("control another"))?;
    assert_eq!(
        drawn.all_actions,
        vec![Action::ControlAnotherDesktop],
        "Ctrl+K, \"control another\", Enter"
    );
    Ok(())
}

#[test]
fn the_connect_form_is_its_own_section_with_a_hint_not_a_value() -> Result<()> {
    let scenario = Scenario {
        lan: true,
        reveal_connect: true,
        panes: &[Pane::Settings],
        ..BASE
    };
    let drawn = draw(scenario, (1440.0, 900.0), 20)?;
    let rows: Vec<String> = texts(&drawn).into_iter().map(|(_, line, _)| line).collect();
    assert!(
        rows.iter()
            .any(|line| line.eq_ignore_ascii_case("Control another Strobe")),
        "its own Settings section: {rows:?}"
    );
    assert!(
        rows.iter().any(|line| line == "e.g. 192.168.1.20"),
        "the empty field hints at what to type: {rows:?}"
    );
    assert!(
        !rows.iter().any(|line| line == "192.168.1.20:47812"),
        "no address in the field that could be mistaken for a value: {rows:?}"
    );
    assert!(
        rows.iter().any(|line| line == "FOH-LAPTOP") && rows.iter().any(|line| line == "BOOTH-PC"),
        "the Strobes found on the network are listed (D468): {rows:?}"
    );
    Ok(())
}

#[test]
fn the_connect_form_asks_for_approval_or_proves_a_trust_file_and_never_wants_a_code() -> Result<()>
{
    let at = |trust_form| Scenario {
        lan: true,
        reveal_connect: true,
        trust_form,
        panes: &[Pane::Settings],
        ..BASE
    };
    let rows = |trust_form| -> Result<Vec<String>> {
        let drawn = draw(at(trust_form), (1440.0, 900.0), 20)?;
        Ok(texts(&drawn).into_iter().map(|(_, line, _)| line).collect())
    };
    let code = rows(false)?;
    assert!(
        code.iter().any(|line| line.contains("Use a trust file")),
        "the option is there, off: {code:?}"
    );
    assert!(
        !code.iter().any(|line| line.starts_with("Pairing code")),
        "a desktop needs no code (D468): {code:?}"
    );
    assert!(
        !code.iter().any(|line| line.starts_with("Trust file")),
        "off: no path field: {code:?}"
    );
    let trust = rows(true)?;
    assert!(
        trust.iter().any(|line| line.contains("venue.trust")),
        "on: the remembered path is in the field: {trust:?}"
    );
    assert!(
        !trust.iter().any(|line| line.starts_with("Pairing code")),
        "on: the code field is gone: {trust:?}"
    );
    assert!(
        trust
            .iter()
            .any(|line| line.contains("--remote-trust") || line.contains("never its contents")),
        "on: says what the host needs and what is remembered: {trust:?}"
    );
    Ok(())
}

#[test]
fn the_mappings_pane_opens_a_macros_and_a_mappings_options_under_their_rows() -> Result<()> {
    let scenario = Scenario {
        mapped: true,
        map_open: true,
        panes: &[Pane::Map],
        ..BASE
    };
    // Tall, so nothing is scrolled out of sight.
    let drawn = draw(scenario, (1440.0, 4000.0), 4)?;
    let lines: Vec<String> = texts(&drawn)
        .into_iter()
        .map(|(_, line, _)| line.trim().to_string())
        .collect();
    for wanted in [
        "Pin it as a macro too",
        "Also try",
        "Reach",
        "Glide",
        "Press at",
        "Curve",
        "Sources",
        "OSC input",
    ] {
        assert!(
            lines.iter().any(|line| line == wanted),
            "{wanted:?} is drawn: {lines:?}"
        );
    }
    Ok(())
}

#[test]
fn every_row_settings_search_finds_is_drawn_on_its_page() -> Result<()> {
    use crate::ui::settings::SettingsPage;
    for page in SettingsPage::ALL {
        // The remote on, so its pairing rows exist; tall, so no row is
        // scrolled out of sight.
        let scenario = Scenario {
            lan: true,
            settings_page: Some(page),
            panes: &[Pane::Settings],
            ..BASE
        };
        let drawn = draw(scenario, (1600.0, 3200.0), 4)?;
        let lines: Vec<String> = texts(&drawn)
            .into_iter()
            .map(|(_, line, _)| line.trim().to_string())
            .collect();
        for (_, title) in crate::ui::settings::tests::indexed().filter(|(on, _)| *on == page) {
            assert!(
                lines.iter().any(|line| line == title),
                "{page:?} lists {title:?} in search but does not draw it: {lines:?}"
            );
        }
    }
    Ok(())
}

/// Every row of text drawn in a pass with its visible rectangle (the row's
/// ink, without the leading space a wrapped label starts with, cut by its
/// clip), in paint order. Rows, not galleys: a label wrapped in a
/// horizontal layout starts mid-line, and its galley's box covers whatever
/// sits before it on the first line.
fn texts(drawn: &Drawn) -> Vec<(usize, String, Rect)> {
    let mut out = Vec::new();
    for (index, clipped) in drawn.shapes.iter().enumerate() {
        let egui::Shape::Text(text) = &clipped.shape else {
            continue;
        };
        for row in &text.galley.rows {
            if row.glyphs.is_empty() {
                continue;
            }
            let rect = row
                .rect_without_leading_space()
                .translate(text.pos.to_vec2())
                .intersect(clipped.clip_rect);
            if rect.is_positive() {
                let line: String = row.glyphs.iter().map(|g| g.chr).collect();
                out.push((index, line, rect));
            }
        }
    }
    out
}

/// Whether `shape` is an opaque filled rectangle over all of `area`. A
/// framed area with a shadow paints its fill inside a `Shape::Vec`.
fn opaque_over(shape: &egui::epaint::ClippedShape, area: Rect) -> bool {
    fn covers(shape: &egui::Shape, clip: Rect, area: Rect) -> bool {
        if let egui::Shape::Rect(rect) = shape {
            return rect.fill.a() == 255 && rect.rect.intersect(clip).contains_rect(area);
        }
        if let egui::Shape::Vec(shapes) = shape {
            return shapes.iter().any(|inner| covers(inner, clip, area));
        }
        false
    }
    covers(&shape.shape, shape.clip_rect, area)
}

#[test]
fn nothing_is_painted_over_blackout_at_any_size() -> Result<()> {
    let sizes = SIZES.iter().copied().chain(SMALL_SIZES);
    for size in sizes {
        for scenario in scenarios() {
            let drawn = draw(scenario, size, 3)?;
            let Some(index) = drawn.shapes.iter().position(|clipped| {
                matches!(&clipped.shape, egui::Shape::Text(t) if t.galley.text().starts_with("BLACKOUT"))
            }) else {
                // The modal question dims everything; nothing else hides it.
                assert!(
                    scenario.replace,
                    "{} at {size:?} draws no BLACKOUT button",
                    scenario.name
                );
                continue;
            };
            let Some(button) = drawn.shapes.get(index) else {
                continue;
            };
            let blackout = button
                .shape
                .visual_bounding_rect()
                .intersect(button.clip_rect);
            assert!(
                blackout.is_positive() && blackout.max.x <= size.0 && blackout.max.y <= size.1,
                "{} at {size:?}: BLACKOUT is outside the window ({blackout:?})",
                scenario.name
            );
            if scenario.replace || scenario.palette.is_some() {
                // The modal's and the palette's scrims cover everything on
                // purpose.
                continue;
            }
            for later in drawn.shapes.iter().skip(index + 1) {
                let rect = later
                    .shape
                    .visual_bounding_rect()
                    .intersect(later.clip_rect);
                assert!(
                    !rect.intersects(blackout.shrink(1.0)),
                    "{} at {size:?}: {:?} is painted over BLACKOUT at {blackout:?}",
                    scenario.name,
                    later.shape
                );
            }
        }
    }
    Ok(())
}

#[test]
fn the_about_page_credits_people_and_lists_every_crate_when_asked() -> Result<()> {
    let lines = |scenario| -> Result<Vec<String>> {
        Ok(texts(&draw(scenario, (1600.0, 40_000.0), 3)?)
            .into_iter()
            .map(|(_, line, _)| line.trim().to_string())
            .collect())
    };
    let closed = lines(ABOUT)?;
    for expected in [
        "Contributors",
        "Lawrence Norton",
        "Built on",
        "ANGLE",
        "Synesthesia",
        "Shadertoy",
        "Every crate",
    ] {
        assert!(
            closed.iter().any(|line| line == expected),
            "About shows {expected:?}: {closed:?}"
        );
    }
    assert!(
        !closed.iter().any(|line| line == "zerocopy"),
        "the crate list starts folded: {closed:?}"
    );
    let opened = lines(Scenario {
        credits_open: true,
        ..ABOUT
    })?;
    assert!(
        opened.iter().any(|line| line == "accesskit")
            && opened.iter().any(|line| line == "zerocopy"),
        "opened, it lists the crates from first to last: {opened:?}"
    );
    Ok(())
}

#[test]
fn no_text_is_drawn_over_other_text_at_any_size() -> Result<()> {
    let sizes = SIZES.iter().copied().chain(SMALL_SIZES);
    let mut failures = Vec::new();
    for size in sizes {
        for scenario in scenarios() {
            // Enough frames for a drawer to finish fading in.
            let drawn = draw(scenario, size, 20)?;
            let texts = texts(&drawn);
            for (i, (ia, a, ra)) in texts.iter().enumerate() {
                for (ib, b, rb) in texts.iter().skip(i + 1) {
                    let overlap = ra.intersect(*rb);
                    if !overlap.is_positive() || overlap.area() < 4.0 {
                        continue;
                    }
                    // A drawer over the preview hides the preview's own
                    // captions on purpose: an opaque fill painted between
                    // the two texts, over their overlap, covers the first.
                    let covered = drawn
                        .shapes
                        .get(ia + 1..*ib)
                        .unwrap_or_default()
                        .iter()
                        .any(|between| opaque_over(between, overlap));
                    if !covered {
                        failures.push(format!(
                            "{} at {size:?}: {a:?} {ra:?} overlaps {b:?} {rb:?}",
                            scenario.name
                        ));
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} overlaps:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

#[test]
fn the_fav_slots_are_in_reach_from_every_tab_where_the_panel_has_room() -> Result<()> {
    // Synesthesia keeps its fav slots on screen whatever the panel shows,
    // and a performer reaches for them mid-show (D176, D283).
    for size in [(1920.0, 1080.0), (1280.0, 720.0), (900.0, 600.0)] {
        for tab in crate::ui::side::SideTab::ALL {
            let scenario = Scenario {
                side_tab: tab,
                ..BASE
            };
            let drawn = draw(scenario, size, 3)?;
            assert!(
                texts(&drawn).iter().any(|(_, line, _)| line == "FAV"),
                "no fav slots on the {tab:?} tab at {size:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_short_drawer_keeps_room_for_the_controls() -> Result<()> {
    // At 427x320 the drawer is too short for the picture and the tab's
    // body both: the picture gives way, so the scene's controls are still
    // there to reach (U-09).
    let drawn = draw(BASE, (427.0, 320.0), 3)?;
    assert!(
        texts(&drawn)
            .iter()
            .any(|(_, line, _)| line == "Aurora Field"),
        "the Controls tab's body is not drawn in the drawer at 427x320"
    );
    Ok(())
}

/// The primary button at `at`, pressed or let go.
fn button(at: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A frame that moves the pointer to `at`, then a press and a release, a
/// frame each: one click.
fn click_at(at: egui::Pos2) -> Vec<Vec<egui::Event>> {
    vec![
        vec![egui::Event::PointerMoved(at)],
        vec![button(at, true)],
        vec![button(at, false)],
    ]
}

/// Where the topmost row of text that reads `line` is drawn in `scenario`
/// at `size`: a card's title sits above the transport strip that may name
/// the same scene.
fn text_at(scenario: Scenario, size: (f32, f32), line: &str) -> Result<egui::Pos2> {
    let drawn = draw(scenario, size, 3)?;
    texts(&drawn)
        .into_iter()
        .filter(|(_, text, _)| text == line)
        .map(|(_, _, rect)| rect.center())
        .min_by(|a, b| a.y.total_cmp(&b.y))
        .ok_or_else(|| anyhow::anyhow!("nothing reads {line:?} in {}", scenario.name))
}

#[test]
fn each_browse_button_asks_for_its_own_dialog() -> Result<()> {
    use crate::picker::Purpose;
    use crate::ui::settings::SettingsPage;
    let cases = [
        (
            Scenario {
                name: "import pane",
                panes: &[Pane::Import],
                ..BASE
            },
            Purpose::Shadertoy,
        ),
        (
            Scenario {
                name: "playlist pane, empty",
                panes: &[Pane::Playlist],
                ..BASE
            },
            Purpose::Playlist,
        ),
        (
            Scenario {
                name: "settings library",
                settings_page: Some(SettingsPage::Library),
                panes: &[Pane::Settings],
                ..BASE
            },
            Purpose::WatchFolder,
        ),
        (
            Scenario {
                name: "settings remote with a trust file",
                lan: true,
                reveal_connect: true,
                trust_form: true,
                panes: &[Pane::Settings],
                ..BASE
            },
            Purpose::TrustFile,
        ),
    ];
    for (scenario, purpose) in cases {
        // Tall, so a row far down the Remote page is drawn.
        let size = (1440.0, 2400.0);
        let at = text_at(scenario, size, "Browse...")?;
        let drawn = draw_events(scenario, size, click_at(at))?;
        assert!(
            drawn.all_actions.contains(&Action::Browse(purpose)),
            "{}: Browse asks for {purpose:?}: {:?}",
            scenario.name,
            drawn.all_actions
        );
    }
    Ok(())
}

/// The scene loads `drawn` asked for, over every frame.
fn loads(drawn: &Drawn) -> Vec<Action> {
    drawn
        .all_actions
        .iter()
        .filter(|action| matches!(action, Action::Load(_)))
        .cloned()
        .collect()
}

#[test]
fn one_click_on_a_card_makes_it_live_and_a_double_click_is_still_one_load() -> Result<()> {
    // As in Synesthesia, a click is the whole gesture (D291).
    let size = (1440.0, 900.0);
    let at = text_at(BASE, size, "03-scene-2")?;
    let mut frames = click_at(at);
    frames.push(Vec::new());
    let drawn = draw_events(BASE, size, frames)?;
    assert_eq!(
        loads(&drawn),
        vec![Action::Load(2)],
        "one click is one load request"
    );

    let mut frames = click_at(at);
    frames.push(vec![button(at, true)]);
    frames.push(vec![button(at, false)]);
    frames.push(Vec::new());
    let drawn = draw_events(BASE, size, frames)?;
    assert_eq!(
        loads(&drawn),
        vec![Action::Load(2)],
        "the second click of a double-click is not a second load"
    );

    let live = text_at(BASE, size, "01-scene-0")?;
    let drawn = draw_events(BASE, size, click_at(live))?;
    assert!(
        loads(&drawn).is_empty(),
        "a click on the live scene asks for nothing: {:?}",
        drawn.all_actions
    );

    let preparing = Scenario {
        loading: true,
        ..BASE
    };
    let at = text_at(preparing, size, "02-scene-1")?;
    let drawn = draw_events(preparing, size, click_at(at))?;
    assert!(
        loads(&drawn).is_empty(),
        "a click on the scene already preparing asks for nothing: {:?}",
        drawn.all_actions
    );
    Ok(())
}

#[test]
fn enter_on_a_focused_card_makes_it_live() -> Result<()> {
    let scenario = Scenario {
        focus_card: Some(2),
        ..BASE
    };
    let drawn = draw_events(
        scenario,
        (1440.0, 900.0),
        vec![
            Vec::new(),
            vec![press(egui::Key::Enter, egui::Modifiers::NONE)],
            Vec::new(),
        ],
    )?;
    assert_eq!(
        loads(&drawn),
        vec![Action::Load(2)],
        "Enter on the focused card loads it"
    );
    Ok(())
}

#[test]
fn controls_is_a_view_on_the_rail_reached_by_click_key_and_palette() -> Result<()> {
    let size = (1440.0, 900.0);

    let at = text_at(BASE, size, "Controls")?;
    let mut frames = click_at(at);
    frames.push(Vec::new());
    let drawn = draw_events(BASE, size, frames)?;
    assert_eq!(
        drawn.main,
        Some(Pane::Controls),
        "the rail's Controls entry"
    );
    assert!(drawn.all_actions.is_empty(), "a presentation change only");

    let ctrl_l = || press(egui::Key::L, egui::Modifiers::COMMAND);
    let drawn = draw_events(BASE, size, vec![vec![ctrl_l()], Vec::new()])?;
    assert_eq!(drawn.main, Some(Pane::Controls), "Ctrl+L shows Controls");
    // Let go between the two presses: egui reads a second press of a key
    // still held as a repeat.
    let drawn = draw_events(
        BASE,
        size,
        vec![
            vec![ctrl_l()],
            vec![release(egui::Key::L)],
            vec![ctrl_l()],
            vec![release(egui::Key::L)],
        ],
    )?;
    assert_eq!(drawn.main, Some(Pane::Browser), "and back to the library");

    let mappings = Scenario {
        panes: &[Pane::Inspector, Pane::Map],
        ..BASE
    };
    let drawn = draw_events(mappings, size, vec![vec![ctrl_l()], Vec::new()])?;
    assert_eq!(
        drawn.main,
        Some(Pane::Controls),
        "from Mappings it shows Controls"
    );

    let drawn = draw_events(BASE, size, palette_run("switch library"))?;
    assert_eq!(
        drawn.main,
        Some(Pane::Controls),
        "Ctrl+K, \"switch library\", Enter"
    );
    assert!(drawn.all_actions.is_empty(), "and nothing else ran");
    Ok(())
}

#[test]
fn generated_controls_are_reviewed_and_saved_from_the_controls_view() -> Result<()> {
    let size = (1600.0, 1000.0);
    let review = Scenario {
        generated: true,
        panes: CONTROLS,
        ..BASE
    };
    let at = text_at(review, size, "Save to this scene")?;
    let mut frames = click_at(at);
    frames.push(Vec::new());
    let drawn = draw_events(review, size, frames)?;
    assert!(
        drawn
            .all_actions
            .iter()
            .any(|a| matches!(a, Action::SaveGeneratedToScene)),
        "the review in the Controls view saves into the scene (D488): {:?}",
        drawn.all_actions
    );
    Ok(())
}

#[test]
fn the_rail_names_no_play_and_a_click_on_the_shown_view_keeps_it() -> Result<()> {
    let size = (1440.0, 900.0);
    let drawn = draw(BASE, size, 2)?;
    assert!(
        !texts(&drawn).iter().any(|(_, line, _)| line == "Play"),
        "the rail has no Play entry (D479)"
    );
    let at = text_at(BASE, size, "Library")?;
    let mut frames = click_at(at);
    frames.push(Vec::new());
    let drawn = draw_events(BASE, size, frames)?;
    assert_eq!(
        drawn.main,
        Some(Pane::Browser),
        "clicking the library while it shows leaves it shown"
    );
    let escape = press(egui::Key::Escape, egui::Modifiers::NONE);
    let only = Scenario {
        panes: &[Pane::Browser],
        ..BASE
    };
    let drawn = draw_events(only, size, vec![vec![escape], Vec::new()])?;
    assert_eq!(
        drawn.main,
        Some(Pane::Browser),
        "Esc keeps the last main view rather than leaving one the rail cannot name"
    );
    Ok(())
}

#[test]
fn the_controls_page_draws_the_controls_once_with_the_preview_beside_them() -> Result<()> {
    let scenario = Scenario {
        panes: CONTROLS,
        ..BASE
    };
    for size in [(1920.0, 1080.0), (1280.0, 720.0)] {
        let drawn = draw(scenario, size, 3)?;
        let rows = texts(&drawn);
        // A group's header names it once per place the group is drawn; the
        // macro strip, which also names `speed`, has no headers.
        let headers = rows.iter().filter(|(_, line, _)| line == "MOTION").count();
        assert_eq!(
            headers, 1,
            "the main view has the controls and the side panel does not repeat them at {size:?}"
        );
        assert!(
            rows.iter().any(|(_, line, _)| line == "LIVE"),
            "the side panel's preview stays in view at {size:?}"
        );
        assert!(
            rows.iter().any(|(_, line, _)| line == "FAV"),
            "and the fav slots at {size:?}"
        );
    }
    Ok(())
}
