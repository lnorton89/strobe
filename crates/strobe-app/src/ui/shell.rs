// ============================================================================
// Module:       strobe_app::ui::shell
// Description:  Composes the control window: title bar and readouts, rail,
//               contextual panes, the Perform surface and the transport
//               strip, in a fixed order.
//
// Dependencies: egui; super::{chrome, perform, transport, browser, inspector,
//               diagnostics, settings, import, modal, keys, model, theme},
//               crate::workspace
// ============================================================================

//! The shell (UI_IMPLEMENTATION_PLAN section 5).
//!
//! The shell owns navigation, the title and readout strip, the panes' dock
//! positions and the modal. It does not own scene controls: a pane receives
//! a rectangle and draws inside it. Perform is the same surface at every
//! window size; what changes is how many panes the width allows
//! ([`crate::workspace::arrange`]).
//!
//! ```text
//! +------------------------------------------------------------------+
//! | STROBE  PERFORM / scene          readouts ............  _ [] x   |
//! +----+----------+------------------------------+------------------+
//! | P  | browser  | toolbar                      | inspector /      |
//! | L  | (opt.)   | banners                      | diagnostics /    |
//! | ...|          | PREVIEW                      | settings (opt.)  |
//! +----+----------+------------------------------+------------------+
//! | prev hold next  NOW .. NEXT ..  time  bpm        [BLACKOUT]      |
//! | macro  macro  macro  macro  macro  macro                         |
//! +------------------------------------------------------------------+
//! ```
//!
//! ## Order matters
//!
//! egui panels claim space in the order they are added, outermost first: the
//! title bar, the transport strip, the rail, the docked panes, and last the
//! central surface. Splitters and the resize handles are added after
//! everything so they win the hit test against the panel edge they sit on.
//!
//! ## Pane widths are ours, not egui's
//!
//! Docked panes are fixed-size panels whose width comes from
//! [`Panes`](crate::workspace::Panes); the splitter on each pane's inner edge
//! writes the new width back. egui's own panel resizing would keep the width
//! in its widget memory, out of reach of the preferences file and of tests.

use super::chrome::{self, Readouts};
use super::model::{Action, Model};
use super::theme::{self, PAD, RAIL_WIDTH, SPACE_SM, SPACE_XS};
use super::{
    browser, diagnostics, import, inspector, keys, map, modal, perform, playlist, settings,
    transport,
};
use crate::palette::Palette;
use crate::workspace::{self, Dock, Pane};
use egui::{CornerRadius, Rect, Sense, Stroke, Ui, Vec2};

/// The drawing state the shell keeps between frames: only the readouts'
/// formatted text. Everything the operator owns is in `UiState`.
#[derive(Debug, Default)]
pub struct ShellState {
    readouts: Readouts,
}

/// A splitter to add once everything else is laid out.
#[derive(Clone, Copy)]
struct Splitter {
    pane: Pane,
    dock: Dock,
    /// The drawn width of the pane in the other dock, if there is one.
    other: Option<f32>,
    window_width: f32,
    /// The pane panel's rectangle.
    rect: Rect,
}

/// Draws the whole control window.
pub fn show(ui: &mut Ui, shell: &mut ShellState, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    let theme = theme::palette();
    let modal_question = import::replace_question(model);
    keys::handle(ui.ctx(), model, modal_question.is_some(), actions);

    let width = ui.ctx().content_rect().width();
    let arrangement = workspace::arrange(&model.ui.panes, width, RAIL_WIDTH);

    chrome::title_bar(ui, model, &mut shell.readouts, &theme, actions);
    transport::show(ui, model, &theme, actions);
    chrome::nav_rail(ui, model, &theme, &arrangement);

    let mut splitters: [Option<Splitter>; 2] = [None, None];
    if !arrangement.overlay {
        if let Some((pane, w)) = arrangement.left {
            let rect = dock_panel(ui, Dock::Left, pane, w, model, &theme, actions);
            splitters[0] = Some(Splitter {
                pane,
                dock: Dock::Left,
                other: arrangement.right.map(|(_, w)| w),
                window_width: width,
                rect,
            });
        }
        if let Some((pane, w)) = arrangement.right {
            let rect = dock_panel(ui, Dock::Right, pane, w, model, &theme, actions);
            splitters[1] = Some(Splitter {
                pane,
                dock: Dock::Right,
                other: arrangement.left.map(|(_, w)| w),
                window_width: width,
                rect,
            });
        }
    }

    let central = egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .inner_margin(theme::margin_xy(SPACE_SM, SPACE_SM)),
        )
        .show(ui, |ui| {
            perform::toolbar(ui, model, &theme, &arrangement, actions);
            ui.add_space(SPACE_XS);
            perform::banners(ui, model, &theme, actions);
            perform::preview(ui, model, &theme, actions);
        });

    if arrangement.overlay {
        let pane = arrangement.right.or(arrangement.left);
        if let Some((pane, w)) = pane {
            drawer(ui, pane, w, central.response.rect, model, &theme, actions);
        }
    }

    for splitter in splitters.into_iter().flatten() {
        splitter_handle(ui, splitter, model, &theme);
    }

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

/// Draws one pane's contents.
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
        Pane::Inspector => inspector::show(ui, model, theme, actions),
        Pane::Diagnostics => diagnostics::show(ui, model, theme, actions),
        Pane::Settings => settings::show(ui, model, theme, actions),
        Pane::Playlist => playlist::show(ui, model, theme, actions),
        Pane::Map => map::show(ui, model, theme, actions),
    }
}

/// A docked pane as a fixed-width panel; returns its rectangle.
fn dock_panel(
    ui: &mut Ui,
    dock: Dock,
    pane: Pane,
    width: f32,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
) -> Rect {
    let panel = match dock {
        Dock::Left => egui::Panel::left("dock-left"),
        Dock::Right => egui::Panel::right("dock-right"),
    };
    let frame = egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .inner_margin(theme::margin_xy(PAD, SPACE_SM));
    panel
        .exact_size(width)
        .resizable(false)
        .show_separator_line(false)
        .frame(frame)
        .show(ui, |ui| {
            pane_body(pane, ui, model, theme, actions);
        })
        .response
        .rect
}

/// A pane drawn over the preview, for a window too narrow to dock it.
fn drawer(
    ui: &mut Ui,
    pane: Pane,
    width: f32,
    surface: Rect,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let height = surface.height();
    egui::Area::new(egui::Id::new("drawer"))
        .order(egui::Order::Foreground)
        .fixed_pos(surface.right_top() - Vec2::new(width, 0.0))
        .constrain(false)
        .show(ui.ctx(), |ui| {
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
                    ui.set_height(height - SPACE_SM * 2.0);
                    pane_body(pane, ui, model, theme, actions);
                });
        });
}

/// The draggable edge on a docked pane's inner side. It paints a line that
/// brightens under the pointer and writes the dragged width back to the pane
/// model.
fn splitter_handle(ui: &mut Ui, splitter: Splitter, model: &mut Model<'_>, theme: &Palette) {
    const GRAB: f32 = 8.0;
    let x = match splitter.dock {
        Dock::Left => splitter.rect.right(),
        Dock::Right => splitter.rect.left(),
    };
    let rect = Rect::from_center_size(
        egui::pos2(x, splitter.rect.center().y),
        Vec2::new(GRAB, splitter.rect.height()),
    );
    let id = egui::Id::new(("splitter", splitter.pane.key()));
    let response = ui
        .interact(rect, id, Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let width = match splitter.dock {
            Dock::Left => pos.x - splitter.rect.left(),
            Dock::Right => splitter.rect.right() - pos.x,
        };
        // Dragging a pane wider must not push its neighbour out of the
        // window: stop at the limit instead.
        let limit = workspace::drag_limit(
            splitter.pane,
            splitter.window_width,
            RAIL_WIDTH,
            splitter.other,
        );
        model.ui.panes.set_width(splitter.pane, width.min(limit));
    }
    let lit = response.hovered() || response.dragged();
    let colour = if lit { theme.accent } else { theme.border };
    ui.painter().vline(
        x,
        splitter.rect.y_range(),
        Stroke::new(if lit { 2.0 } else { 1.0 }, theme::rgb(colour)),
    );
}
