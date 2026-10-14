// ============================================================================
// Module:       strobe_app::ui::shell
// Description:  Composes the control window: title bar and readouts, rail,
//               the main view, the side panel (docked, a drawer, or a handle)
//               and the transport strip, in a fixed order.
//
// Dependencies: egui; super::{chrome, perform, side, transport, browser,
//               diagnostics, settings, import, modal, keys, model, theme},
//               crate::workspace
// ============================================================================

//! The shell (UI_IMPLEMENTATION_PLAN section 5, D272).
//!
//! The shell owns navigation, the title and readout strip, the main view,
//! the side panel's place and the modal. It does not own scene controls: a
//! pane receives a rectangle and draws inside it. The topology is
//! Synesthesia's, three columns under the title bar, with Strobe's transport
//! strip along the bottom:
//!
//! ```text
//! +------------------------------------------------------------------+
//! | STROBE  LIBRARY / scene          readouts ............  _ [] x   |
//! +----+-------------------------------------------+-----------------+
//! | P  | banners                                   | PREVIEW         |
//! | L  |                                           | fps audio  [][] |
//! | G  | main view: the library grid, or the pane  | FAV 1..8        |
//! | L  | the rail chose (LIST, EDIT, MAP, DIAG,    | CONTROLS|LIST|.. |
//! | E  | SET, GET), or the large preview (PLAY)    | (tab body)      |
//! | ...|                                           |                 |
//! +----+-------------------------------------------+-----------------+
//! | prev hold next  NOW .. NEXT ..  time  bpm        [BLACKOUT]      |
//! | macro  macro  macro  macro  macro  macro                         |
//! | help bar: what the control under the pointer does                |
//! +------------------------------------------------------------------+
//! ```
//!
//! ## Order matters
//!
//! egui panels claim space in the order they are added, outermost first: the
//! title bar, the help bar, the transport strip, the rail, the side panel (or
//! its handle), and last the main view. The splitter and the resize handles are added
//! after everything so they win the hit test against the panel edge they
//! sit on.
//!
//! ## The side panel's three states
//!
//! [`crate::workspace::arrange`] decides: *docked* beside the main view
//! while the main view keeps [`crate::workspace::MAIN_MIN`]; a *drawer*
//! over the main view below that; and when the panel is closed, a
//! [`HANDLE`]-wide strip at the right edge with one button that opens it,
//! so the preview and the controls are never more than one click away.
//!
//! ## Banners
//!
//! Failures and notices sit above whatever the main view is. They take at
//! most [`BANNER_SHARE`] of its height and scroll beyond that, so a long
//! notice in a small window can neither take the main view nor grow the
//! surface over the transport strip and BLACKOUT (D276).
//!
//! ## Pane widths are ours, not egui's
//!
//! The side panel's width comes from [`Panes`](crate::workspace::Panes);
//! the splitter on its inner edge writes the new width back. egui's own
//! panel resizing would keep the width in its widget memory, out of reach
//! of the preferences file and of tests.

use super::chrome::{self, Readouts};
use super::model::{Action, Model};
use super::theme::{self, PAD, RAIL_WIDTH, SPACE_SM, SPACE_XS};
use super::{
    browser, command_palette, diagnostics, editor, help, import, keys, map, modal, perform,
    playlist, settings, side, transport,
};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::workspace::{self, Arrangement, Pane};
use egui::{CornerRadius, Rect, Sense, Stroke, Ui, Vec2};

/// The drawing state the shell keeps between frames: only the readouts'
/// formatted text. Everything the operator owns is in `UiState`.
#[derive(Debug, Default)]
pub struct ShellState {
    readouts: Readouts,
}

/// The width of the strip that stands in for the closed side panel.
pub const HANDLE: f32 = 28.0;

/// The most of the main surface's height the banners take before they
/// scroll.
const BANNER_SHARE: f32 = 0.5;

/// Draws the whole control window.
pub fn show(ui: &mut Ui, shell: &mut ShellState, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    let theme = theme::palette();
    let modal_question = import::replace_question(model);
    if modal_question.is_some() {
        // The question outranks the palette; it is the one modal.
        model.ui.command_palette.close();
    }
    // The palette owns the keyboard while it is open, as the modal does.
    let palette_open = model.ui.command_palette.open;
    // Capturing a new shortcut owns the keyboard too (D208).
    let capturing = model.ui.hotkeys.capturing.is_some() || model.remote.ui.key_capture.is_some();
    keys::handle(
        ui.ctx(),
        model,
        modal_question.is_some() || palette_open || capturing,
        actions,
    );

    let width = ui.ctx().content_rect().width();
    let arrangement = workspace::arrange(&model.ui.panes, width, RAIL_WIDTH);

    chrome::title_bar(
        ui,
        model,
        &mut shell.readouts,
        &theme,
        &arrangement,
        actions,
    );
    // The help bar is the outermost of the bottom panels; the transport
    // strip sits above it.
    help::show(ui, &theme, arrangement.main);
    transport::show(ui, model, &theme, actions);
    chrome::nav_rail(ui, model, &theme, &arrangement);

    let mut splitter = None;
    match arrangement.side {
        Some(w) if !arrangement.overlay => {
            let rect = side_panel(ui, w, model, &shell.readouts, &theme, actions);
            splitter = Some((rect, width));
        }
        Some(_) | None => handle_strip(ui, model, &theme, &arrangement),
    }

    let mut below_banners = f32::NAN;
    let central = egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .inner_margin(theme::margin_xy(SPACE_SM, SPACE_SM)),
        )
        .show(ui, |ui| {
            // The banners sit above whatever the main view is, so a scene
            // that failed is seen from the library as well as from PLAY.
            // They share the surface: past half of it they scroll, so a
            // long notice in a small window never takes the main view, and
            // never grows the surface over the transport strip and BLACKOUT
            // (U-09).
            let cap = (ui.available_height() * BANNER_SHARE).max(0.0);
            egui::ScrollArea::vertical()
                .id_salt("banners")
                .max_height(cap)
                .auto_shrink([false, true])
                .show(ui, |ui| perform::banners(ui, model, &theme, actions));
            // A drawer opens below the banners, so it never covers them
            // (U-09).
            below_banners = ui.cursor().top();
            match arrangement.main {
                None => {
                    perform::toolbar(ui, model, &theme, actions);
                    ui.add_space(SPACE_XS);
                    perform::preview(ui, model, &theme, actions);
                }
                Some(pane) => main_view(ui, pane, model, &theme, actions),
            }
        });

    if arrangement.overlay
        && let Some(w) = arrangement.side
    {
        let mut surface = central.response.rect;
        if below_banners.is_finite() {
            surface.min.y = below_banners.clamp(surface.min.y, surface.max.y);
        }
        drawer(ui, w, surface, model, &shell.readouts, &theme, actions);
    }

    if let Some((rect, window_width)) = splitter {
        splitter_handle(ui, rect, window_width, model, &theme);
    }

    command_palette::draw(ui.ctx(), &theme, model, actions);
    browser::drag_label(ui.ctx(), &theme);

    if let Some(question) = modal_question
        && let Some(answer) = modal::draw(ui.ctx(), &theme, &question)
    {
        actions.push(if answer {
            Action::ConfirmReplace
        } else {
            Action::CancelReplace
        });
    }

    if model.ui.custom_title_bar && !model.maximized {
        chrome::resize_handles(ui);
    }
}

/// Draws one main pane's contents.
fn pane_body(
    pane: Pane,
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    match pane {
        Pane::Browser => browser::show(ui, model, theme, actions),
        Pane::Import => import::show(ui, model, theme, actions),
        Pane::Diagnostics => diagnostics::show(ui, model, theme, actions),
        Pane::Settings => settings::show(ui, model, theme, actions),
        Pane::Playlist => playlist::show(ui, model, theme, actions),
        Pane::Map => map::show(ui, model, theme, actions),
        Pane::Editor => editor::show(ui, model, theme, actions),
        // The side panel; never a main view. Drawn here only if the pane
        // model were to misfile it, and then as the panel's contents.
        Pane::Inspector => {
            super::inspector::show(ui, model, theme, actions, super::inspector::Part::Controls);
        }
    }
}

/// The main view: a pane in a panel-coloured card that fills the surface,
/// drawn in a child that cannot widen it.
fn main_view(
    ui: &mut Ui,
    pane: Pane,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let area = ui.available_rect_before_wrap();
    ui.painter().rect_filled(
        area,
        CornerRadius::same(theme::RADIUS),
        theme::rgb(theme.panel),
    );
    let inner = area.shrink2(Vec2::new(PAD, SPACE_SM));
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    body.set_clip_rect(inner.intersect(ui.clip_rect()));
    pane_body(pane, &mut body, model, theme, actions);
    ui.advance_cursor_after_rect(area);
}

/// The docked side panel as a fixed-width panel; returns its rectangle.
fn side_panel(
    ui: &mut Ui,
    width: f32,
    model: &mut Model<'_>,
    readouts: &Readouts,
    theme: &Palette,
    actions: &mut Vec<Action>,
) -> Rect {
    let frame = egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .inner_margin(theme::margin_xy(PAD, SPACE_SM));
    egui::Panel::right("side-panel")
        .exact_size(width)
        .resizable(false)
        .show_separator_line(false)
        .frame(frame)
        .show(ui, |ui| {
            // The panel draws in a child that cannot widen it. A right
            // panel whose content overflows its exact width moves its
            // recorded left edge right by the overflow, and the main view
            // then drew over it (D260).
            let rect = ui.max_rect();
            let mut body = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            body.set_clip_rect(rect.intersect(ui.clip_rect()));
            side::show(&mut body, model, readouts, theme, actions);
            ui.advance_cursor_after_rect(rect);
        })
        .response
        .rect
}

/// The strip that stands in for the side panel while it is closed, or
/// while it is an open drawer: one button that opens or closes it.
fn handle_strip(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, arrangement: &Arrangement) {
    let open = arrangement.side.is_some();
    egui::Panel::right("side-handle")
        .exact_size(HANDLE)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.panel))
                .inner_margin(theme::margin_xy(0.0, SPACE_SM)),
        )
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                let (icon, tip) = if open {
                    (Icon::Close, "Close the panel (Esc)")
                } else {
                    (
                        Icon::Controls,
                        "Open the panel: preview, controls, playlist and presets",
                    )
                };
                if handle_button(ui, theme, icon, tip).clicked() {
                    model.ui.panes.toggle_drawn(Pane::Inspector, open);
                }
                if !open && model.transport.blackout {
                    // The output is dark: say so where the panel's badge
                    // would have.
                    let (dot, response) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                    ui.painter()
                        .circle_filled(dot.center(), 4.0, theme::rgb(theme.danger));
                    response.on_hover_text("BLACKOUT: the output is dark");
                }
            });
        });
}

/// The handle strip's one button: an icon button that fits the strip.
fn handle_button(ui: &mut Ui, theme: &Palette, icon: Icon, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(HANDLE - 4.0, 28.0), Sense::click());
    let fill = super::widgets::hover_fill(
        ui,
        response.id,
        response.hovered(),
        theme.panel,
        theme.hover,
    );
    ui.painter()
        .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    super::icon::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(super::icon::INLINE)),
        icon,
        theme::rgb(theme.accent),
    );
    response.on_hover_text(tooltip)
}

/// The side panel drawn over the main view, for a window too narrow to
/// dock it.
fn drawer(
    ui: &mut Ui,
    width: f32,
    surface: Rect,
    model: &mut Model<'_>,
    readouts: &Readouts,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let height = surface.height();
    egui::Area::new(egui::Id::new("drawer"))
        .order(egui::Order::Foreground)
        .fixed_pos(surface.right_top() - Vec2::new(width, 0.0))
        .constrain(false)
        .show(ui.ctx(), |ui| {
            // Never past the main view: whatever the panel cannot fit is
            // cut off here, not drawn over the transport strip and BLACKOUT
            // (U-09, T480 Task 35). The tabs scroll inside.
            ui.set_clip_rect(surface.intersect(ui.clip_rect()));
            egui::Frame::new()
                .fill(theme::rgb(theme.panel))
                .stroke(Stroke::new(1.0, theme::rgb(theme.border)))
                .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(theme::margin_xy(PAD, SPACE_SM))
                .shadow(egui::epaint::Shadow {
                    offset: [-4, 0],
                    blur: 16,
                    spread: 0,
                    // Alpha-only: a shadow is lighting, not a theme colour.
                    color: egui::Color32::from_black_alpha(96),
                })
                .show(ui, |ui| {
                    ui.set_width(width - PAD * 2.0);
                    // A tall banner can leave the drawer no room at all.
                    ui.set_height((height - SPACE_SM * 2.0).max(0.0));
                    side::show(ui, model, readouts, theme, actions);
                });
        });
}

/// The draggable edge on the docked side panel's inner side. It paints a
/// line that brightens under the pointer and writes the dragged width back
/// to the pane model.
fn splitter_handle(
    ui: &mut Ui,
    panel: Rect,
    window_width: f32,
    model: &mut Model<'_>,
    theme: &Palette,
) {
    const GRAB: f32 = 8.0;
    let x = panel.left();
    let rect = Rect::from_center_size(
        egui::pos2(x, panel.center().y),
        Vec2::new(GRAB, panel.height()),
    );
    let id = egui::Id::new(("splitter", Pane::Inspector.key()));
    let response = ui
        .interact(rect, id, Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let width = panel.right() - pos.x;
        // Dragging the panel wider must not squeeze the main view below
        // its minimum: stop at the limit instead.
        let limit = workspace::drag_limit(window_width, RAIL_WIDTH);
        model.ui.panes.set_width(Pane::Inspector, width.min(limit));
    }
    let lit = response.hovered() || response.dragged();
    let colour = if lit { theme.accent } else { theme.border };
    ui.painter().vline(
        x,
        panel.y_range(),
        Stroke::new(if lit { 2.0 } else { 1.0 }, theme::rgb(colour)),
    );
}
