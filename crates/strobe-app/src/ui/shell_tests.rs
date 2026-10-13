// ============================================================================
// Module:       strobe_app::ui::shell_tests
// Description:  The whole shell drawn headless at the sizes the plan names and
//               in every state the audit lists. Test code only.
//
// Dependencies: egui (headless context); super::{shell, model, state},
//               crate::{library, console, stats, script}
// ============================================================================

//! The shell, end to end, with no GPU.
//!
//! egui lays out and tessellates without a window, so the states STATUS_AUDIT
//! P1 lists (empty and loading, failure, missing audio, script fault, output
//! hidden, a narrow window) are drawn here at 1920x1080, 1440x900, 1366x768,
//! 1280x720, 900x600 and 640x480, and each must fit its window: the root
//! UI's used rectangle may not exceed the screen. That is the "no horizontal
//! scrolling" acceptance criterion (UI_IMPLEMENTATION_PLAN section 23) as an
//! assertion rather than a screenshot. The screenshots in
//! `docs/images/perform/` are the other half: they show it looks right.

use super::shell::{ShellState, show};
use super::{
    Action, AudioPanel, CueTransport, EditorPanel, FailurePanel, GeneratePanel, ImportPanel,
    ListenerPanel, MapUi, Model, NoticePanel, PerfPanel, PlaylistPanel, PlaylistUi, Preview,
    RemotePanel, ScenePreview, TransportPanel, UiState,
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
    /// The shortcut editor waits for a key for this command.
    capturing: Option<crate::commands::CommandId>,
    /// The shortcut editor shows a clash (tap tempo wanted blackout's B).
    clash: bool,
    panes: &'static [Pane],
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
    previewing: false,
    replace: false,
    playlist: false,
    mapped: false,
    generating: false,
    generated: false,
    capturing: None,
    clash: false,
    panes: &[Pane::Inspector],
};

fn scenarios() -> Vec<Scenario> {
    vec![
        BASE,
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
    ]
}

/// Show control with the content the MAP pane and the macro strip need to
/// show every state: nothing, or one of each.
/// A scene open in the Editor, with an unsaved edit, built in memory (no
/// files are read).
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
        written: Some(PathBuf::from("scenes/01-scene-0-auto.synScene")),
        context: "headless EGL device | llvmpipe".to_string(),
        candidates: 9,
        fidelity: Some(0.0),
        kept: vec![
            Kept {
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
    crate::ui::theme::apply(&ctx, crate::palette::fallback());
    let library = Library {
        dir: PathBuf::from("scenes"),
        entries: (0..scenario.scenes)
            .map(|i| {
                let entry = Entry::new(format!("{:02}-scene-{i}", i + 1), PathBuf::from("x"));
                // The playing scene carries tags, so the Inspector's chips
                // are measured at every size.
                if i == 0 {
                    entry.with_tags(&["dark", "tunnel", "audio reactive", "3d", "raymarching"])
                } else {
                    entry
                }
            })
            .collect(),
    };
    let health: Vec<Health> = (0..scenario.scenes)
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
    let mut map_ui = MapUi::default();
    let bindings = crate::commands::Bindings::default();
    let mut remote = sample_remote(scenario.mapped)?;
    let presets = vec!["Slow burn".to_string(), "Peak".to_string()];
    let sample_show_state = scenario
        .playlist
        .then(|| sample_show(&library))
        .transpose()?;
    let generated = scenario.generated.then(sample_generated);
    let mut editor_doc = scenario.panes.contains(&Pane::Editor).then(sample_doc);
    let generate_panel = GeneratePanel {
        running: scenario.generating.then_some(("01-scene-0", 12.0)),
        outcome: generated.as_ref(),
        authored: false,
    };
    let mut shell_state = ShellState::default();
    let mut actions = Vec::new();
    let mut all_actions = Vec::new();
    let mut shapes = Vec::new();
    let screen = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(size.0, size.1));
    for events in frames {
        actions.clear();
        let mut model = Model {
            ui: &mut ui_state,
            library: &library,
            health: &health,
            library_rev: 1,
            scan_error: None,
            playing: scenario.playing.then_some(0),
            scene_title: if scenario.playing { "Aurora Field" } else { "" },
            controls: scenario.playing.then_some(&mut bank),
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
            notice: scenario.notice.then_some(NoticePanel {
                tone: Tone::Info,
                text: "The graphics context was lost and rebuilt on ANGLE in 181 ms.",
            }),
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
            },
            script: script.as_ref(),
            perf: PerfPanel {
                adaptive: true,
                has_timer: true,
                gpu_ms: Some(2.1),
                budget_ms: 16.7,
                scale: 0.85,
            },
            import: ImportPanel {
                text: &mut import_text,
                status: &status,
                replace_title: scenario.replace.then_some("fractal-tunnel"),
                busy: false,
                drop_hover: false,
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
            },
        };
        let pass = crate::ui::testing::run(
            &ctx,
            crate::ui::testing::input(screen.size(), events),
            |ui| show(ui, &mut shell_state, &mut model, &mut actions),
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
    })
}

#[test]
fn every_state_draws_at_every_size_without_overflowing_the_window() -> Result<()> {
    for scenario in scenarios() {
        for size in SIZES {
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
        vec![Pane::Inspector],
        "Escape closed the palette, not the pane behind it"
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
