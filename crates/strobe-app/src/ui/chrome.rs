// ============================================================================
// Module:       strobe_app::ui::chrome
// Description:  The window's own frame: the title bar with its health
//               readouts and window controls, the invisible resize handles,
//               and the navigation rail.
//
// Dependencies: egui (ViewportCommand for the window controls); super::{theme,
//               widgets, icon, model}, crate::{workspace, icon}.
//               Ported and adapted from rustaman's `gui::ui::chrome` (D5, D130).
// ============================================================================

//! The window's frame.
//!
//! ## Why the title bar is drawn rather than asked for
//!
//! Windows 10 gives an app a system caption that is a light grey bar with the
//! system's own fonts and buttons in it, and nothing an app does themes it. On
//! GNOME's Wayland, with winit's client-side bars removed (D122), there is no
//! caption at all. So the window can be opened undecorated and the bar drawn
//! here, which also buys room to put the health readouts *in* it (UI_DESIGN
//! section 6) rather than in a second row.
//!
//! The cost is that everything the system caption did has to be rebuilt, and
//! the parts people notice when they are missing are:
//!
//! - **Dragging**, including snap. `ViewportCommand::StartDrag` is not a "move
//!   the window" command: it hands the drag to the window manager, which is
//!   what keeps Aero Snap, snap layouts and drag-to-another-monitor working.
//!   A hand-rolled "move by the pointer delta" loses all three, silently.
//! - **Double-click to maximise**, which is muscle memory.
//! - **Resizing**, since an undecorated window has no border to grab:
//!   [`resize_handles`] adds eight invisible ones.
//!
//! It is a *setting* (`custom_title_bar`, D132), on by default on Windows and
//! Linux. Turning it off calls `set_decorations(true)` and this module draws
//! only the readouts, so a platform where the custom bar misbehaves is one
//! click from the system's.
//!
//! ## The readouts
//!
//! The health strip (fps, GPU time against budget, render scale, backend,
//! audio, output) is the Level 1 of the diagnostics model
//! (UI_IMPLEMENTATION_PLAN section 14): always visible, never a dialog. Their
//! text is formatted at 4 Hz ([`Readouts`]), not 30, because a number that
//! changes thirty times a second cannot be read, and because formatting
//! allocates. Each is a button that opens the pane that explains it.

use super::icon as icons;
use super::model::{Action, Model};
use super::theme::{self, PAD, RAIL_WIDTH, SPACE_MD, SPACE_SM, SPACE_XS, TITLE_BAR_HEIGHT};
use super::widgets;
use crate::icon::Icon;
use crate::palette::Palette;
use crate::script::ScriptState;
use crate::workspace::{Arrangement, Pane};
use egui::{
    Align, CornerRadius, Layout, Rect, ResizeDirection, Sense, TextStyle, Ui, Vec2, ViewportCommand,
};

/// How thick the invisible resize handles are: wide enough to hit without
/// aiming, narrow enough that it does not steal clicks from a control near
/// the edge. The system's own border is about four, and an undecorated window
/// has none at all.
const RESIZE_GRIP: f32 = 6.0;

/// Corners need a larger target than straight edges: six by six makes
/// diagonal resizing technically present but practically unreachable at
/// normal pointer speeds and high-DPI scaling.
const RESIZE_CORNER: f32 = 18.0;

/// How often the readouts' text is re-formatted, in seconds.
const READOUT_PERIOD: f64 = 0.25;

/// Below this window width the readouts shrink to icons with tooltips.
const COMPACT_WIDTH: f32 = 1180.0;

/// Below this width the breadcrumb goes and every readout is an icon, the
/// LAN and audio ones included (U-09: at 150% a 1024-pixel window is 683
/// points, and its readouts ran over the title).
const NARROW_WIDTH: f32 = 760.0;

/// Below this width the wordmark goes too, leaving the mark.
const TINY_WIDTH: f32 = 420.0;

/// The GPU meter's room, beside its readout.
const METER_ROOM: f32 = 40.0;

// Relations between constants, so checked when the crate is compiled.
const _: () = {
    assert!(
        RESIZE_GRIP >= 4.0,
        "an undecorated window has no border at all, so the grip must be at \
         least as wide as the system's own ~4 points"
    );
    assert!(
        RESIZE_GRIP <= 10.0,
        "a grip this wide steals clicks from controls at the window edge"
    );
    assert!(RESIZE_CORNER > RESIZE_GRIP);
};

/// The formatted text of the health readouts, refreshed at 4 Hz.
#[derive(Debug, Default)]
pub struct Readouts {
    updated: Option<f64>,
    fps: String,
    /// The tempo, for the side panel's status strip.
    bpm: String,
    gpu: String,
    gpu_fraction: f32,
    scale: String,
    backend: String,
    audio: String,
    output: String,
    /// Empty while the scene's script is fine or absent: a script readout
    /// appears only when there is something to say.
    script: String,
    script_level: Level,
    /// Empty while the LAN remote is off.
    remote: String,
}

/// How a readout is coloured: the state it reports, besides its words.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum Level {
    #[default]
    Quiet,
    Good,
    Warn,
    Bad,
}

impl Readouts {
    /// The frame-rate text, for the side panel's status strip.
    pub fn fps(&self) -> &str {
        &self.fps
    }

    /// The tempo text, for the side panel's status strip.
    pub fn bpm(&self) -> &str {
        &self.bpm
    }

    /// Re-formats the text if it is more than [`READOUT_PERIOD`] old.
    fn refresh(&mut self, model: &Model<'_>, now: f64) {
        if self.updated.is_some_and(|at| now - at < READOUT_PERIOD) {
            return;
        }
        self.updated = Some(now);
        self.fps = format!("{:.0} fps", model.stats.fps());
        self.bpm.clear();
        if model.audio.bpm > 0.0 {
            self.bpm = format!("{:.0} BPM", model.audio.bpm);
        } else {
            self.bpm.push_str("-- BPM");
        }
        let perf = &model.perf;
        self.gpu_fraction = 0.0;
        self.gpu = match (perf.has_timer, perf.gpu_ms) {
            (true, Some(ms)) => {
                self.gpu_fraction = ms / perf.budget_ms.max(0.1);
                format!("GPU {ms:.1}/{:.1} ms", perf.budget_ms)
            }
            (true, None) => "GPU measuring".to_string(),
            (false, _) => "GPU timer n/a".to_string(),
        };
        self.scale = format!(
            "{:.0}%{}",
            perf.scale * 100.0,
            if perf.adaptive && perf.has_timer {
                " adaptive"
            } else {
                ""
            }
        );
        self.backend = model
            .gl_summary
            .split(" | ")
            .next()
            .unwrap_or(model.gl_summary)
            .to_string();
        self.audio = audio_text(model);
        self.output = output_text(model);
        self.remote.clear();
        let lan = model.lan.view;
        if lan.running {
            self.remote = format!(
                "Remote on  {} connected{}",
                lan.connections,
                if lan.requests.is_empty() {
                    ""
                } else {
                    "  pairing request"
                }
            );
        }
        self.script.clear();
        match model.script {
            Some(ScriptState::Faulted { .. }) => {
                self.script.push_str("Script fault");
                self.script_level = Level::Warn;
            }
            Some(ScriptState::Disabled { .. }) => {
                self.script.push_str("Script disabled");
                self.script_level = Level::Bad;
            }
            Some(ScriptState::Running) | None => {}
        }
    }
}

/// The audio readout: what the person needs to know about whether the show
/// is hearing anything.
fn audio_text(model: &Model<'_>) -> String {
    let audio = &model.audio;
    if audio.live {
        format!("Audio live  {:.0} BPM", audio.bpm)
    } else if audio.can_capture {
        "No audio input".to_string()
    } else {
        "Synthetic audio".to_string()
    }
}

/// How the audio readout is levelled: missing input is a warning, because
/// the visuals continue without reacting.
fn audio_level(model: &Model<'_>) -> Level {
    if model.audio.live {
        Level::Good
    } else if model.audio.can_capture {
        Level::Warn
    } else {
        Level::Quiet
    }
}

fn output_text(model: &Model<'_>) -> String {
    if !model.output_visible {
        "Output hidden".to_string()
    } else if model.fullscreen {
        match model
            .output_monitor
            .and_then(|index| model.monitors.get(index))
        {
            Some(monitor) => format!("Output fullscreen  {}", short_monitor(monitor)),
            None => "Output fullscreen".to_string(),
        }
    } else {
        "Output windowed".to_string()
    }
}

/// `2: DELL U2415 (1920x1200)` as `2: DELL U2415`.
fn short_monitor(label: &str) -> &str {
    label.split(" (").next().unwrap_or(label)
}

/// The title bar's crumb: the main view, then the scene.
fn crumb(model: &Model<'_>, main: Option<Pane>) -> String {
    let view = main.map_or("PERFORM", Pane::title).to_uppercase();
    match (model.loading, model.playing, model.scene_title) {
        (Some(name), _, _) => format!("{view}  /  loading {name}"),
        (None, Some(_), title) if !title.is_empty() => format!("{view}  /  {title}"),
        (None, Some(_), _) | (None, None, _) => view,
    }
}

/// Draws the title bar. With the custom bar on it carries the drag area and
/// the window buttons; either way it carries the brand, the crumb and the
/// health readouts.
pub fn title_bar(
    ui: &mut Ui,
    model: &mut Model<'_>,
    readouts: &mut Readouts,
    theme: &Palette,
    arrangement: &Arrangement,
    actions: &mut Vec<Action>,
) {
    let custom = model.ui.custom_title_bar;
    let width = ui.ctx().content_rect().width();
    let compact = width < COMPACT_WIDTH;
    let narrow = width < NARROW_WIDTH;
    readouts.refresh(model, ui.input(|i| i.time));

    egui::Panel::top("title-bar")
        .exact_size(TITLE_BAR_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .inner_margin(theme::margin_xy(SPACE_SM, 0.0)),
        )
        .show(ui, |ui| {
            // The drag surface is registered first, under everything, so any
            // widget drawn over it wins the pointer and the empty parts of
            // the bar drag the window.
            if custom {
                drag_surface(ui);
            }
            ui.horizontal_centered(|ui| {
                paint_mark(ui, theme);
                ui.add_space(SPACE_XS);
                if width >= TINY_WIDTH {
                    ui.label(
                        egui::RichText::new("STROBE")
                            .strong()
                            .color(theme::rgb(theme.text)),
                    );
                }
                if !narrow {
                    ui.add_space(SPACE_SM);
                    ui.label(
                        egui::RichText::new(crumb(model, arrangement.main))
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    );
                }

                // Right to left: the window controls against the edge, then
                // the readouts. Laid out before the middle so the middle is
                // whatever width is left.
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if custom {
                        window_buttons(ui, model, theme, actions);
                        ui.add_space(SPACE_SM);
                    }
                    health(ui, model, readouts, theme, (compact, narrow), actions);
                });
            });
        });
}

/// Registers the whole bar as a drag surface and double-click-to-maximise.
fn drag_surface(ui: &mut Ui) {
    let rect = ui.max_rect();
    let response = ui.interact(rect, ui.id().with("drag-surface"), Sense::click_and_drag());
    if response.drag_started() {
        // Hands the drag to the window manager. See the module docs.
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if response.double_clicked() {
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!maximized));
    }
}

fn window_buttons(ui: &mut Ui, model: &Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    if widgets::window_button(ui, theme, Icon::Close, "Quit Strobe", true).clicked() {
        actions.push(Action::Quit);
    }
    let (icon, tooltip) = if model.maximized {
        (Icon::WindowRestore, "Restore")
    } else {
        (Icon::WindowMaximise, "Maximise")
    };
    if widgets::window_button(ui, theme, icon, tooltip, false).clicked() {
        ui.ctx()
            .send_viewport_cmd(ViewportCommand::Maximized(!model.maximized));
    }
    if widgets::window_button(ui, theme, Icon::WindowMinimise, "Minimise", false).clicked() {
        ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

/// The brand mark (`docs/branding.md` section 2): the Parametric Flow
/// ribbon, simplified as for the 32 px icon (no contours, innermost turn
/// dropped), in the theme's accent. That is the guide's monochrome variant,
/// so the mark follows the theme like every other colour the UI paints.
///
/// The curve is the six cubic segments of `assets/brand/icon-small.svg`, in
/// that file's 512-unit space, flattened to a polyline here. Painting it is
/// a few dozen line segments per UI pass and no allocation.
fn paint_mark(ui: &mut Ui, theme: &Palette) {
    const SIZE: f32 = 18.0;
    /// Where the ribbon's bounding box sits in the SVG, and its larger side.
    const CENTRE: [f32; 2] = [238.0, 243.0];
    const SPAN: f32 = 300.0;
    /// Start point, then each segment's two control points and end point.
    const START: [f32; 2] = [117.0, 269.0];
    const SEGMENTS: [[[f32; 2]; 3]; 6] = [
        [[93.0, 175.0], [166.0, 99.0], [252.0, 119.0]],
        [[318.0, 135.0], [381.0, 205.0], [360.0, 273.0]],
        [[339.0, 334.0], [260.0, 386.0], [192.0, 355.0]],
        [[136.0, 330.0], [115.0, 273.0], [157.0, 226.0]],
        [[202.0, 176.0], [279.0, 180.0], [322.0, 223.0]],
        [[355.0, 258.0], [333.0, 304.0], [290.0, 322.0]],
    ];
    const STEPS: u8 = 8;

    let (rect, _) = ui.allocate_exact_size(Vec2::splat(SIZE), Sense::hover());
    let painter = ui.painter();
    let centre = rect.center();
    let scale = SIZE / SPAN;
    let to_screen =
        |[x, y]: [f32; 2]| centre + Vec2::new((x - CENTRE[0]) * scale, (y - CENTRE[1]) * scale);
    let stroke = egui::Stroke::new(1.6, theme::rgb(theme.accent));
    let mut from = START;
    for [c1, c2, to] in SEGMENTS {
        let mut previous = to_screen(from);
        for step in 1..=STEPS {
            let t = f32::from(step) / f32::from(STEPS);
            let point = to_screen(cubic(from, c1, c2, to, t));
            painter.line_segment([previous, point], stroke);
            previous = point;
        }
        from = to;
    }
}

/// A point on a cubic Bézier at `t`.
fn cubic(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], t: f32) -> [f32; 2] {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [
        a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0],
        a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1],
    ]
}

/// One readout: an icon and text, as a button that opens `opens`.
fn readout(
    ui: &mut Ui,
    theme: &Palette,
    icon: Icon,
    text: &str,
    level: Level,
    tooltip: &str,
    show_text: bool,
) -> Option<egui::Response> {
    let colour = match level {
        Level::Quiet => theme.text_muted,
        Level::Good => theme.success,
        Level::Warn => theme.warning,
        Level::Bad => theme.danger,
    };
    let font = TextStyle::Small.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, theme::rgb(theme.text_muted));
    let glyph = icons::INLINE - 1.0;
    let icon_only = SPACE_XS * 2.0 + glyph;
    // Laid out right to left against the title: a readout that would run
    // into it drops its text, and with no room even for its icon it is
    // left out (U-09). The window controls and the LAN kill switch come
    // before every readout, so they always have their room.
    let room = ui.available_width();
    let show_text = show_text && icon_only + galley.size().x + SPACE_XS <= room;
    if icon_only > room {
        return None;
    }
    let text_width = if show_text {
        galley.size().x + SPACE_XS
    } else {
        0.0
    };
    let size = Vec2::new(icon_only + text_width, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let fill = widgets::hover_fill(ui, response.id, response.hovered(), theme.app, theme.hover);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    icons::paint(
        painter,
        Rect::from_center_size(
            egui::pos2(rect.left() + SPACE_XS + glyph / 2.0, rect.center().y),
            Vec2::splat(glyph),
        ),
        icon,
        theme::rgb(colour),
    );
    if show_text {
        painter.galley(
            egui::pos2(
                rect.left() + SPACE_XS + glyph + SPACE_XS,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::rgb(theme.text_muted),
        );
    }
    Some(response.on_hover_text(tooltip))
}

/// The health readouts, drawn right to left.
fn health(
    ui: &mut Ui,
    model: &mut Model<'_>,
    readouts: &Readouts,
    theme: &Palette,
    (compact, narrow): (bool, bool),
    actions: &mut Vec<Action>,
) {
    let text = !compact;
    // The readouts that keep their text in a compact bar lose it too in a
    // narrow one.
    let always = !narrow;

    // The LAN remote: always visible while it listens, with its kill
    // switch beside it (L-08, D254). Drawn first, so it sits nearest the
    // window controls.
    if !readouts.remote.is_empty() {
        let stop = if compact {
            widgets::icon_button(ui, theme, Icon::Close, "Stop the LAN remote")
        } else {
            widgets::button(ui, theme, Some(Icon::Close), "Stop remote")
                .on_hover_text("Stops listening and drops every phone at once.")
        };
        if stop.clicked() {
            actions.push(Action::SetLanRemote(false));
        }
        let waiting = !model.lan.view.requests.is_empty();
        if readout(
            ui,
            theme,
            if waiting {
                Icon::Warning
            } else {
                Icon::Mapping
            },
            &readouts.remote,
            if waiting { Level::Warn } else { Level::Good },
            "The LAN remote is listening. Click for its settings, pairing and paired devices.",
            always,
        )
        .is_some_and(|r| r.clicked())
        {
            model.ui.panes.open(Pane::Settings);
        }
    }

    let output_level = if model.output_visible {
        Level::Good
    } else {
        Level::Quiet
    };
    if readout(
        ui,
        theme,
        Icon::Output,
        &readouts.output,
        output_level,
        "Output window. Click to open Settings.",
        text,
    )
    .is_some_and(|r| r.clicked())
    {
        model.ui.panes.open(Pane::Settings);
    }

    let audio_tip = if model.audio.live {
        format!("{}. Click for audio settings.", model.audio.source)
    } else if model.audio.can_capture {
        format!(
            "{}. The visuals keep running without audio reaction. Click to choose a device.",
            model.audio.source
        )
    } else {
        format!("{}. Click for audio settings.", model.audio.source)
    };
    if readout(
        ui,
        theme,
        Icon::Audio,
        &readouts.audio,
        audio_level(model),
        &audio_tip,
        always,
    )
    .is_some_and(|r| r.clicked())
    {
        model.ui.panes.open(Pane::Settings);
    }

    if !readouts.script.is_empty()
        && readout(
            ui,
            theme,
            Icon::Warning,
            &readouts.script,
            readouts.script_level,
            "The scene's script.js is not running cleanly. Open Controls for the message.",
            always,
        )
        .is_some_and(|r| r.clicked())
    {
        model.ui.panes.open(Pane::Inspector);
    }

    if !compact
        && readout(
            ui,
            theme,
            Icon::Controls,
            &readouts.backend,
            Level::Quiet,
            model.gl_summary,
            true,
        )
        .is_some_and(|r| r.clicked())
    {
        model.ui.panes.open(Pane::Diagnostics);
    }

    if readout(
        ui,
        theme,
        Icon::Fullscreen,
        &readouts.scale,
        Level::Quiet,
        "Render scale. Click to change it in Settings.",
        text,
    )
    .is_some_and(|r| r.clicked())
    {
        model.ui.panes.open(Pane::Settings);
    }

    // GPU time against its budget, tinted along the heat scale.
    let ratio = readouts.gpu_fraction;
    let heat = theme.heat(ratio.clamp(0.0, 1.0));
    let level = if ratio >= 1.0 {
        Level::Bad
    } else if ratio >= 0.85 {
        Level::Warn
    } else {
        Level::Quiet
    };
    if readout(
        ui,
        theme,
        Icon::Diagnostics,
        &readouts.gpu,
        level,
        "GPU time against the frame budget. Click for diagnostics.",
        text,
    )
    .is_some_and(|r| r.clicked())
    {
        model.ui.panes.open(Pane::Diagnostics);
    }
    if text && model.perf.has_timer && ui.available_width() >= METER_ROOM {
        widgets::meter(ui, theme, ratio, heat, 36.0);
    }

    if readout(
        ui,
        theme,
        Icon::Play,
        &readouts.fps,
        Level::Quiet,
        "Output frame rate. Click for diagnostics.",
        always,
    )
    .is_some_and(|r| r.clicked())
    {
        model.ui.panes.open(Pane::Diagnostics);
    }
}

/// Adds the eight invisible resize handles an undecorated window needs.
///
/// Drawn last, over everything, so they win the hit test against a control
/// that happens to sit against the window edge.
pub fn resize_handles(ui: &mut Ui) {
    let rect = ui.ctx().content_rect();
    resize_handles_in(ui, rect);
}

fn resize_handles_in(ui: &mut Ui, rect: Rect) {
    for (index, (area, direction, cursor)) in resize_regions(rect).into_iter().enumerate() {
        let id = egui::Id::new("resize-handle").with(index);
        // A foreground Area gives the native border its own interaction
        // layer. Registering the rectangles on the root Ui left them
        // underneath panel widgets in egui's hit-test order, so a corner
        // could be stolen by the title bar beneath it.
        egui::Area::new(id)
            .order(egui::Order::Foreground)
            .constrain(false)
            .fixed_pos(area.min)
            .show(ui.ctx(), |ui| {
                let (_, response) = ui.allocate_exact_size(area.size(), Sense::click_and_drag());
                // The eight rectangles are disjoint, so geometric hit testing
                // is unambiguous and more reliable than asking egui which
                // widget owns the pointer.
                let contains_pointer = ui.input(|input| {
                    input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| response.rect.contains(pos))
                });
                if contains_pointer {
                    ui.ctx().set_cursor_icon(cursor);
                }
                // Hand the gesture to the window manager on the press frame:
                // waiting for egui's drag threshold misses the button-down
                // that the native resize call requires.
                if contains_pointer && ui.input(|input| input.pointer.primary_pressed()) {
                    ui.ctx()
                        .send_viewport_cmd(ViewportCommand::BeginResize(direction));
                }
            });
    }
}

/// The eight disjoint hit regions around an undecorated viewport. Edge bands
/// stop at the corner squares: egui resolves overlapping interactions by
/// layer, so a full-width edge over a corner would turn an intended diagonal
/// resize into a vertical one.
fn resize_regions(rect: Rect) -> [(Rect, ResizeDirection, egui::CursorIcon); 8] {
    let horizontal = (rect.width() - RESIZE_CORNER * 2.0).max(0.0);
    let vertical = (rect.height() - RESIZE_CORNER * 2.0).max(0.0);
    [
        (
            Rect::from_min_size(rect.left_top(), Vec2::splat(RESIZE_CORNER)),
            ResizeDirection::NorthWest,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(
                rect.right_top() - Vec2::new(RESIZE_CORNER, 0.0),
                Vec2::splat(RESIZE_CORNER),
            ),
            ResizeDirection::NorthEast,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(
                rect.left_bottom() - Vec2::new(0.0, RESIZE_CORNER),
                Vec2::splat(RESIZE_CORNER),
            ),
            ResizeDirection::SouthWest,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            Rect::from_min_size(
                rect.right_bottom() - Vec2::splat(RESIZE_CORNER),
                Vec2::splat(RESIZE_CORNER),
            ),
            ResizeDirection::SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            Rect::from_min_size(
                rect.left_top() + Vec2::new(RESIZE_CORNER, 0.0),
                Vec2::new(horizontal, RESIZE_GRIP),
            ),
            ResizeDirection::North,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_size(
                rect.left_bottom() + Vec2::new(RESIZE_CORNER, -RESIZE_GRIP),
                Vec2::new(horizontal, RESIZE_GRIP),
            ),
            ResizeDirection::South,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            Rect::from_min_size(
                rect.left_top() + Vec2::new(0.0, RESIZE_CORNER),
                Vec2::new(RESIZE_GRIP, vertical),
            ),
            ResizeDirection::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            Rect::from_min_size(
                rect.right_top() + Vec2::new(-RESIZE_GRIP, RESIZE_CORNER),
                Vec2::new(RESIZE_GRIP, vertical),
            ),
            ResizeDirection::East,
            egui::CursorIcon::ResizeHorizontal,
        ),
    ]
}

/// The rail's destinations: each switches the main view (D272). LIST is the
/// Playlist (D157), MAP the Mappings (D177), EDIT the Editor (D204), GET the
/// Shadertoy import.
const RAIL: [(Icon, &str, Option<Pane>, &str); 8] = [
    (
        Icon::Perform,
        "PLAY",
        None,
        "Perform: the live preview at full size",
    ),
    (
        Icon::Library,
        "LIB",
        Some(Pane::Browser),
        "Scene library: search, preview and load scenes",
    ),
    (
        Icon::Import,
        "GET",
        Some(Pane::Import),
        "Get scenes: import a Shadertoy shader",
    ),
    (
        Icon::Playlist,
        "LIST",
        Some(Pane::Playlist),
        "Playlist: load a file, the timeline, cue commands and Generate",
    ),
    (
        Icon::Editor,
        "EDIT",
        Some(Pane::Editor),
        "Editor: the scene's shader, scene.json and script, with save and reload",
    ),
    (
        Icon::Mapping,
        "MAP",
        Some(Pane::Map),
        "MIDI and OSC: devices, mappings with learn, macros and the activity monitor",
    ),
    (
        Icon::Diagnostics,
        "DIAG",
        Some(Pane::Diagnostics),
        "Diagnostics and console",
    ),
    (
        Icon::Settings,
        "SET",
        Some(Pane::Settings),
        "Output, audio, performance and appearance",
    ),
];

/// Draws the navigation rail. The rail switches the main view; the side
/// panel stays whatever it was.
pub fn nav_rail(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, arrangement: &Arrangement) {
    let shown = |pane: Pane| arrangement.shows(pane);
    let nothing_shown = arrangement.main.is_none();
    egui::Panel::left("nav-rail")
        .exact_size(RAIL_WIDTH)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .inner_margin(theme::margin_xy(SPACE_XS, SPACE_SM)),
        )
        .show(ui, |ui| {
            let last = RAIL.len() - 1;
            // A short window (T480 Task 38: 1024x640 at 150%) has no room for
            // every entry: labels go first, then the upper entries scroll, so
            // none is ever drawn under the transport strip or under SET at
            // the bottom.
            let room = ui.available_height();
            let fits =
                |labels: bool| RAIL.len() as f32 * (rail_item_height(labels) + SPACE_XS) <= room;
            let labels = arrangement.rail_labels && fits(true);
            let upper_room = (room - rail_item_height(labels) - SPACE_XS).max(0.0);
            egui::ScrollArea::vertical()
                .max_height(upper_room)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    for (index, (icon, label, pane, tooltip)) in RAIL.iter().enumerate() {
                        if index == last {
                            continue;
                        }
                        rail_entry(
                            ui,
                            model,
                            theme,
                            *icon,
                            label,
                            *pane,
                            tooltip,
                            labels,
                            &shown,
                            nothing_shown,
                        );
                        ui.add_space(SPACE_XS);
                    }
                });
            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                if let Some((icon, label, pane, tooltip)) = RAIL.get(last) {
                    rail_entry(
                        ui,
                        model,
                        theme,
                        *icon,
                        label,
                        *pane,
                        tooltip,
                        labels,
                        &shown,
                        nothing_shown,
                    );
                }
            });
        });
}

/// The height of one rail entry, as `widgets::nav_item` draws it.
fn rail_item_height(labels: bool) -> f32 {
    widgets::nav_item_height(labels)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site per rail entry; a struct would only rename the arguments"
)]
fn rail_entry(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    icon: Icon,
    label: &str,
    pane: Option<Pane>,
    tooltip: &str,
    labels: bool,
    shown: &dyn Fn(Pane) -> bool,
    nothing_shown: bool,
) {
    // PLAY has no pane: it is the large preview, the main view when no pane
    // is open. The not-built destinations have no pane either and are told
    // apart by their tooltip.
    let is_play = icon == Icon::Perform;
    let built = is_play || pane.is_some();
    let active = match pane {
        Some(pane) => shown(pane),
        None => is_play && nothing_shown,
    };
    let response = widgets::nav_item(ui, theme, icon, label, active, built, labels);
    let response = super::help::hovered(response, tooltip);
    if built && response.clicked() {
        match pane {
            Some(pane) => model.ui.panes.toggle_drawn(pane, shown(pane)),
            None => model.ui.panes.close_main(),
        }
    }
}

/// The inset the work surface keeps from the window edge.
pub const SURFACE_PAD: f32 = PAD;

/// The gap between the surface's rows.
pub const ROW_GAP: f32 = SPACE_MD;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_resize_grip_has_one_unambiguous_direction() {
        let window = Rect::from_min_size(egui::pos2(10.0, 20.0), Vec2::new(780.0, 620.0));
        let regions = resize_regions(window);
        let probes = [
            (
                window.left_top() + Vec2::splat(RESIZE_GRIP / 2.0),
                ResizeDirection::NorthWest,
            ),
            (
                window.right_top() + Vec2::new(-RESIZE_GRIP / 2.0, RESIZE_GRIP / 2.0),
                ResizeDirection::NorthEast,
            ),
            (
                window.left_bottom() + Vec2::new(RESIZE_GRIP / 2.0, -RESIZE_GRIP / 2.0),
                ResizeDirection::SouthWest,
            ),
            (
                window.right_bottom() - Vec2::splat(RESIZE_GRIP / 2.0),
                ResizeDirection::SouthEast,
            ),
            (
                egui::pos2(window.center().x, window.top() + 1.0),
                ResizeDirection::North,
            ),
            (
                egui::pos2(window.center().x, window.bottom() - 1.0),
                ResizeDirection::South,
            ),
            (
                egui::pos2(window.left() + 1.0, window.center().y),
                ResizeDirection::West,
            ),
            (
                egui::pos2(window.right() - 1.0, window.center().y),
                ResizeDirection::East,
            ),
        ];
        for (point, expected) in probes {
            let hits: Vec<ResizeDirection> = regions
                .iter()
                .filter(|(region, _, _)| region.contains(point))
                .map(|(_, direction, _)| *direction)
                .collect();
            assert_eq!(
                hits,
                vec![expected],
                "{point:?} must hit exactly one region"
            );
        }
    }

    #[test]
    fn diagonal_resize_targets_are_large_enough_to_hit() {
        let window = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(780.0, 620.0));
        for (region, direction, _) in resize_regions(window).into_iter().take(4) {
            assert!(matches!(
                direction,
                ResizeDirection::NorthWest
                    | ResizeDirection::NorthEast
                    | ResizeDirection::SouthWest
                    | ResizeDirection::SouthEast
            ));
            assert_eq!(region.size(), Vec2::splat(RESIZE_CORNER));
            assert!(
                region.width() >= 16.0,
                "{direction:?} is still only a precision-sized target"
            );
        }
    }

    #[test]
    fn pressing_a_corner_immediately_requests_a_diagonal_native_resize() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let window = Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(780.0, 620.0));
        let run = |events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(window),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ui| resize_handles_in(ui, window),
            )
        };

        let start = window.right_bottom() - Vec2::splat(3.0);
        // Areas participate in hit testing from the frame after their first
        // layout, as they do in the live app after its first painted frame.
        let mut warmup = run(Vec::new());
        warmup.textures_delta.clear();
        let mut output = run(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        ]);
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .ok_or_else(|| anyhow::anyhow!("egui produced no root viewport output"))?
            .commands;
        let requested =
            commands.contains(&ViewportCommand::BeginResize(ResizeDirection::SouthEast));
        let commands = format!("{commands:?}");
        output.textures_delta.clear();
        assert!(
            requested,
            "corner press produced {commands} instead of a diagonal resize"
        );
        Ok(())
    }

    #[test]
    fn the_rail_lists_every_workspace_of_the_mockup_in_order() {
        let labels: Vec<&str> = RAIL.iter().map(|(_, label, _, _)| *label).collect();
        assert_eq!(
            labels,
            ["PLAY", "LIB", "GET", "LIST", "EDIT", "MAP", "DIAG", "SET"],
            "UI_DESIGN section 5: the main views in the rail's order (D272)"
        );
    }

    #[test]
    fn the_unbuilt_workspaces_say_so_in_their_tooltips() {
        for (_, label, pane, tooltip) in RAIL {
            if pane.is_none() && label != "PLAY" {
                assert!(
                    tooltip.contains("not built"),
                    "{label} is disabled but does not say why: {tooltip}"
                );
            }
        }
    }

    #[test]
    fn a_monitor_label_is_shortened_for_the_strip() {
        assert_eq!(short_monitor("2: DELL U2415 (1920x1200)"), "2: DELL U2415");
        assert_eq!(short_monitor("Display"), "Display");
    }
}
