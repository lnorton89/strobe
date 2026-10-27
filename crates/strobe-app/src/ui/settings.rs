// ============================================================================
// Module:       strobe_app::ui::settings
// Description:  The settings pane: a category list with search beside one
//               page of settings at a time (Output, Audio, Appearance,
//               Library, Remote, Projectors, Shortcuts, About).
//
// Dependencies: egui; super::{form, model, theme, widgets, audio, lan,
//               projectors, keys, credits}, crate::{commands, session, icon}
// ============================================================================

//! Settings: everything the operator sets once and rarely touches, kept out
//! of Perform (UI_DESIGN section 3: system health and configuration come
//! after the output and the immediate controls).
//!
//! ## Layout (UI review section 4, the owner's answers in section 5)
//!
//! A category list on the left, with a search field over it, and the chosen
//! page on the right in [`super::form`]'s grammar: a title, then groups of
//! rows, each row a title and a description beside its control. Narrower
//! than [`NAV_BESIDE`], the list becomes a picker over the page.
//!
//! Search matches every row's title and a few words people search for
//! (`INDEX`), plus every command on the Shortcuts page. A result opens its
//! page and lights the row (`form::reveal`).
//!
//! ## Pages, in the order a soundcheck goes
//!
//! - **Output**: show or hide the window, fullscreen, which monitor (D96);
//!   adaptive resolution and the render scale (D64).
//! - **Audio** (`super::audio`): the input device, reactivity, speed and
//!   tap tempo.
//! - **Appearance**: the theme, density, reduced motion, the title bar
//!   (D132), and named layouts (D215).
//! - **Library**: Synesthesia's own scenes, read-only (D271), and the
//!   watched folder (D240, D265). Both switches act at once; the app checks
//!   the folder as at launch and saves it only when it is accepted.
//! - **Remote** (`super::lan`): the LAN remote, pairing and paired devices
//!   (D254), and the form for controlling another Strobe (D275). Ctrl+K
//!   finds it as "Control another Strobe", which opens this page.
//! - **Projectors** (`super::projectors`): projector power over PJLink, the
//!   web page a phone opens, and the projectors' connections (D502).
//! - **Shortcuts**: the command table as data, so what is advertised is
//!   what exists, and the shortcut editor (D208). Change waits for a key; a
//!   key another command holds is named and moved only on a second click;
//!   Reset returns the default. Changes apply at once and are saved under
//!   `hotkeys` in `show-control.json`. A capture lasts only while this page
//!   is up (`shell::drop_unseen_captures`).
//! - **About**: the build and the graphics context, the source and the
//!   licence, then the credits (`super::credits`, D491): contributors, the
//!   projects Strobe is built on, Synesthesia and Shadertoy, and every
//!   crate in the program with its licence.
//!
//! The render-scale slider edits a working value and only commits when the
//! drag ends, because every change reallocates every render target.

use super::audio;
use super::form::{self, Row};
use super::icon as icons;
use super::keys::{self, HotkeyEditor};
use super::model::{Action, Model};
use super::theme::{self, RADIUS, SPACE_LG, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::commands::{Bindings, Chord, CommandId};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::session::RENDER_SCALE;
use crate::workspace::Pane;
use egui::{Rect, Sense, Ui, UiBuilder, Vec2};

/// A page of Settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    /// The output window and rendering.
    #[default]
    Output,
    /// The audio input and how scenes respond to it.
    Audio,
    /// Theme, density, motion, the title bar and layouts.
    Appearance,
    /// Synesthesia's scenes and the watched folder.
    Library,
    /// The LAN remote and controlling another Strobe.
    Remote,
    /// Projector power (D502).
    Projectors,
    /// Keyboard shortcuts.
    Shortcuts,
    /// The build and the graphics context.
    About,
}

impl SettingsPage {
    /// Every page, in the order the list shows them.
    pub const ALL: [Self; 8] = [
        Self::Output,
        Self::Audio,
        Self::Appearance,
        Self::Library,
        Self::Remote,
        Self::Projectors,
        Self::Shortcuts,
        Self::About,
    ];

    /// The page's name, in the list and as its title.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Output => "Output",
            Self::Audio => "Audio",
            Self::Appearance => "Appearance",
            Self::Library => "Library",
            Self::Remote => "Remote",
            Self::Projectors => "Projectors",
            Self::Shortcuts => "Shortcuts",
            Self::About => "About",
        }
    }

    /// The page named `key`, its title in lower case (`--open-pane
    /// settings:audio`).
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|page| page.title().eq_ignore_ascii_case(key))
    }

    /// The page's icon in the list.
    const fn icon(self) -> Icon {
        match self {
            Self::Output => Icon::Output,
            Self::Audio => Icon::Audio,
            Self::Appearance => Icon::Star,
            Self::Library => Icon::Library,
            Self::Remote => Icon::Mapping,
            Self::Projectors => Icon::Blackout,
            Self::Shortcuts => Icon::Controls,
            Self::About => Icon::Info,
        }
    }

    /// The line under the page's title.
    const fn lede(self) -> &'static str {
        match self {
            Self::Output => "The window the audience sees, and how hard the GPU works to draw it.",
            Self::Audio => "What Strobe listens to, and how strongly scenes answer it.",
            Self::Appearance => "How this control window looks. The output is never affected.",
            Self::Library => "Where scenes come from besides your own library folder.",
            Self::Remote => {
                "Run the show from another device, or run another Strobe from this one."
            }
            Self::Projectors => {
                "Switch the venue's projectors on and off, from here or from a phone."
            }
            Self::Shortcuts => {
                "Every command with a key. Shortcuts are off while a text field has focus."
            }
            Self::About => {
                "Which build this is, what it is drawing with, and who and what it is made from."
            }
        }
    }
}

/// The Settings pane's presentation state. Not saved.
#[derive(Debug, Default)]
pub struct SettingsUi {
    /// The page on the right.
    pub page: SettingsPage,
    /// The search field.
    pub query: String,
}

/// The pane narrower than this puts the page picker over the page instead
/// of the list beside it.
pub const NAV_BESIDE: f32 = 620.0;

/// The width of the category list.
const NAV_WIDTH: f32 = 200.0;

// Row titles. Each is drawn on its page and listed in `INDEX`, from one
// constant, so a search result always names a row that exists.
const OUTPUT_WINDOW: &str = "Output window";
const MONITOR: &str = "Monitor";
const ADAPTIVE: &str = "Adaptive resolution";
const SCALE: &str = "Render scale";
const THEME: &str = "Theme";
const DENSITY: &str = "Density";
const REDUCE_MOTION: &str = "Reduce motion";
const TITLE_BAR: &str = "Strobe title bar";
const SAVE_LAYOUT: &str = "Save the current layout";
/// The label of the switch that lists Synesthesia's scenes (D271).
pub const SHOW_SYNESTHESIA: &str = "Show Synesthesia's scenes";
const WATCH: &str = "Watch a folder for new scenes";
const FOLDER: &str = "Folder";
const VERSION: &str = "Version";
const GRAPHICS: &str = "Graphics";

/// One searchable row: its page, its title, and words a person might
/// search for that the title does not contain.
struct Entry {
    page: SettingsPage,
    title: &'static str,
    words: &'static str,
}

/// Every row search finds, in page order. The Shortcuts page's rows are
/// the commands, matched from [`CommandId::ALL`] instead.
const INDEX: [Entry; 32] = [
    Entry {
        page: SettingsPage::Output,
        title: OUTPUT_WINDOW,
        words: "show hide fullscreen projector",
    },
    Entry {
        page: SettingsPage::Output,
        title: MONITOR,
        words: "display screen projector",
    },
    Entry {
        page: SettingsPage::Output,
        title: ADAPTIVE,
        words: "performance gpu frame budget",
    },
    Entry {
        page: SettingsPage::Output,
        title: SCALE,
        words: "resolution quality performance",
    },
    Entry {
        page: SettingsPage::Audio,
        title: audio::INPUT,
        words: "device microphone loopback sound",
    },
    Entry {
        page: SettingsPage::Audio,
        title: audio::REACTIVITY,
        words: "gain sensitivity",
    },
    Entry {
        page: SettingsPage::Audio,
        title: audio::SPEED,
        words: "time",
    },
    Entry {
        page: SettingsPage::Audio,
        title: audio::TEMPO,
        words: "bpm tap beat",
    },
    Entry {
        page: SettingsPage::Appearance,
        title: THEME,
        words: "colour color dark light",
    },
    Entry {
        page: SettingsPage::Appearance,
        title: DENSITY,
        words: "compact comfortable size",
    },
    Entry {
        page: SettingsPage::Appearance,
        title: REDUCE_MOTION,
        words: "animation accessibility",
    },
    Entry {
        page: SettingsPage::Appearance,
        title: TITLE_BAR,
        words: "window chrome",
    },
    Entry {
        page: SettingsPage::Appearance,
        title: SAVE_LAYOUT,
        words: "panes workspace layouts",
    },
    Entry {
        page: SettingsPage::Library,
        title: SHOW_SYNESTHESIA,
        words: "scenes install",
    },
    Entry {
        page: SettingsPage::Library,
        title: WATCH,
        words: "inbox drop import",
    },
    Entry {
        page: SettingsPage::Library,
        title: FOLDER,
        words: "watched path",
    },
    Entry {
        page: SettingsPage::Remote,
        title: super::lan::REMOTE,
        words: "remote phone tablet network wifi lan advertise",
    },
    Entry {
        page: SettingsPage::Remote,
        title: super::lan::PAIR,
        words: "phone tablet pair approve browser url",
    },
    Entry {
        page: SettingsPage::Remote,
        title: super::lan::DESKTOP,
        words: "full control",
    },
    Entry {
        page: SettingsPage::Remote,
        title: super::lan::REVOKE_ALL,
        words: "paired devices",
    },
    Entry {
        page: SettingsPage::Remote,
        title: super::lan::HOST,
        words: "control another strobe connect host address desktop",
    },
    Entry {
        page: SettingsPage::Remote,
        title: super::lan::USE_TRUST,
        words: "venue trust file",
    },
    Entry {
        page: SettingsPage::Projectors,
        title: super::projectors::CONTROL,
        words: "projector power pjlink panasonic sony beamer on off",
    },
    Entry {
        page: SettingsPage::Projectors,
        title: super::projectors::WEB,
        words: "projector phone browser api token tailscale",
    },
    Entry {
        page: SettingsPage::Projectors,
        title: super::projectors::ADD,
        words: "projector address ip password ntcontrol connection",
    },
    Entry {
        page: SettingsPage::Projectors,
        title: super::projectors::LISTEN,
        words: "projector port bind wifi tailscale",
    },
    Entry {
        page: SettingsPage::Shortcuts,
        title: SHORTCUTS_GROUP,
        words: "keyboard hotkeys keys",
    },
    Entry {
        page: SettingsPage::About,
        title: VERSION,
        words: "build commit",
    },
    Entry {
        page: SettingsPage::About,
        title: GRAPHICS,
        words: "opengl gpu driver renderer",
    },
    Entry {
        page: SettingsPage::About,
        title: super::credits::SOURCE,
        words: "github repository code bug issue",
    },
    Entry {
        page: SettingsPage::About,
        title: super::credits::LICENCE,
        words: "license mit commons clause legal",
    },
    Entry {
        page: SettingsPage::About,
        title: super::credits::CRATES,
        words: "credits contributors third party dependencies notices open source thanks",
    },
];

/// The Shortcuts page's group heading, which is also its search entry.
const SHORTCUTS_GROUP: &str = "Keyboard shortcuts";

/// Whether `haystack` contains `needle`, ignoring ASCII case, without
/// allocating.
fn contains_ignoring_case(haystack: &str, needle: &str) -> bool {
    let (haystack, needle) = (haystack.as_bytes(), needle.as_bytes());
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| window.eq_ignore_ascii_case(needle))
}

/// Whether every word of `query` is in one of `fields`.
fn matches(query: &str, fields: &[&str]) -> bool {
    query.split_whitespace().all(|word| {
        fields
            .iter()
            .any(|field| contains_ignoring_case(field, word))
    })
}

/// A search result: the row's page and title.
type Hit = (SettingsPage, &'static str);

/// Every row `query` matches, in page order.
fn search(query: &str) -> impl Iterator<Item = Hit> + '_ {
    let rows = INDEX
        .iter()
        .filter(move |entry| matches(query, &[entry.title, entry.words, entry.page.title()]))
        .map(|entry| (entry.page, entry.title));
    let commands = CommandId::ALL
        .into_iter()
        .filter(move |command| matches(query, &[command.name(), "shortcut key"]))
        .map(|command| (SettingsPage::Shortcuts, command.name()));
    rows.chain(commands)
}

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let closed = widgets::pane_header(ui, theme, Icon::Settings, Pane::Settings.title(), |_| {});
    if closed {
        model.ui.panes.close(Pane::Settings);
    }
    // The palette's "Control another Strobe", or a failed connection, asked
    // for the connect form: it is on the Remote page.
    if model.lan.controller.reveal_pending() {
        model.ui.settings.page = SettingsPage::Remote;
        model.ui.settings.query.clear();
    }
    let area = ui.available_rect_before_wrap();
    if area.width() >= NAV_BESIDE {
        let nav = Rect::from_min_size(area.min, Vec2::new(NAV_WIDTH, area.height()));
        let content = Rect::from_min_max(egui::pos2(nav.right() + SPACE_LG, area.top()), area.max);
        ui.scope_builder(UiBuilder::new().max_rect(nav), |ui| {
            search_field(ui, theme, &mut model.ui.settings.query);
            ui.add_space(SPACE_SM);
            egui::ScrollArea::vertical()
                .id_salt("settings-nav")
                .auto_shrink([false, false])
                .show(ui, |ui| nav_list(ui, theme, &mut model.ui.settings));
        });
        ui.scope_builder(UiBuilder::new().max_rect(content), |ui| {
            page_scroll(ui, model, theme, actions);
        });
    } else {
        ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
            ui.horizontal(|ui| {
                page_picker(ui, &mut model.ui.settings);
                search_field(ui, theme, &mut model.ui.settings.query);
            });
            ui.add_space(SPACE_SM);
            if model.ui.settings.query.trim().is_empty() {
                page_scroll(ui, model, theme, actions);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("settings-results")
                    .auto_shrink([false, false])
                    .show(ui, |ui| nav_list(ui, theme, &mut model.ui.settings));
            }
        });
    }
    ui.allocate_rect(area, Sense::hover());
}

/// The search field, filling the width it is given.
fn search_field(ui: &mut Ui, theme: &Palette, query: &mut String) {
    let width = ui.available_width();
    form::field(ui, theme, query, "Search settings", width);
}

/// The narrow layout's page picker.
fn page_picker(ui: &mut Ui, settings: &mut SettingsUi) {
    egui::ComboBox::from_id_salt("settings-page")
        .selected_text(settings.page.title())
        .show_ui(ui, |ui| {
            for page in SettingsPage::ALL {
                if ui
                    .selectable_label(settings.page == page, page.title())
                    .clicked()
                {
                    settings.page = page;
                    settings.query.clear();
                }
            }
        });
}

/// The category list, or with a query, the rows it matches.
fn nav_list(ui: &mut Ui, theme: &Palette, settings: &mut SettingsUi) {
    let query = settings.query.trim();
    if query.is_empty() {
        for page in SettingsPage::ALL {
            if nav_item(
                ui,
                theme,
                page.icon(),
                page.title(),
                None,
                settings.page == page,
            )
            .clicked()
            {
                settings.page = page;
            }
        }
        return;
    }
    let mut chosen = None;
    let mut any = false;
    for (page, title) in search(query) {
        any = true;
        if nav_item(ui, theme, page.icon(), title, Some(page.title()), false).clicked() {
            chosen = Some((page, title));
        }
    }
    if !any {
        ui.label(
            egui::RichText::new("Nothing matches.")
                .font(theme::description_font())
                .color(theme::rgb(theme.text_muted)),
        );
    }
    if let Some((page, title)) = chosen {
        settings.page = page;
        settings.query.clear();
        form::reveal(title);
    }
}

/// One entry of the list: an icon, a label, an optional second line, and
/// the selection bar when it is the page on screen.
fn nav_item(
    ui: &mut Ui,
    theme: &Palette,
    icon: Icon,
    label: &str,
    detail: Option<&str>,
    selected: bool,
) -> egui::Response {
    let height = match (theme::density(), detail.is_some()) {
        (theme::Density::Comfortable, false) => 34.0,
        (theme::Density::Compact, false) => 28.0,
        (theme::Density::Comfortable, true) => 46.0,
        (theme::Density::Compact, true) => 38.0,
    };
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let id = egui::Id::unique(("settings-nav", label));
    let fill = widgets::selection_fill(ui, theme, id, selected, response.hovered(), theme.panel);
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(RADIUS), fill);
    if selected {
        widgets::accent_bar(ui, painter, id, rect, widgets::Bar::Inset, theme.accent);
    }
    let ink = if selected {
        theme.selection_text
    } else {
        theme.text
    };
    let left = rect.left() + SPACE_SM * 1.5;
    icons::paint(
        painter,
        Rect::from_center_size(
            egui::pos2(left + icons::INLINE / 2.0, rect.center().y),
            Vec2::splat(icons::INLINE),
        ),
        icon,
        theme::rgb(if selected {
            theme.accent
        } else {
            theme.text_muted
        }),
    );
    let text_left = left + icons::INLINE + SPACE_SM;
    let text_width = (rect.right() - text_left - SPACE_XS).max(0.0);
    let body = egui::TextStyle::Body.resolve(ui.style());
    let title = widgets::truncated(ui, label, body, ink, text_width);
    match detail {
        None => {
            painter.galley(
                egui::pos2(text_left, rect.center().y - title.size().y / 2.0),
                title,
                theme::rgb(ink),
            );
        }
        Some(detail) => {
            let small = widgets::truncated(
                ui,
                detail,
                theme::description_font(),
                theme.text_muted,
                text_width,
            );
            let total = title.size().y + small.size().y;
            let top = rect.center().y - total / 2.0;
            let below = top + title.size().y;
            painter.galley(egui::pos2(text_left, top), title, theme::rgb(ink));
            painter.galley(
                egui::pos2(text_left, below),
                small,
                theme::rgb(theme.text_muted),
            );
        }
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect,
            egui::CornerRadius::same(RADIUS),
            egui::Stroke::new(2.0, theme::rgb(theme.accent)),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The page on screen, scrolled.
fn page_scroll(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let page = model.ui.settings.page;
    egui::ScrollArea::vertical()
        .id_salt(("settings-scroll", page))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            form::page(ui, theme, page.title(), Some(page.lede()), |ui| {
                match page {
                    SettingsPage::Output => output(ui, model, theme, actions),
                    SettingsPage::Audio => audio::show(ui, theme, &mut model.audio, actions),
                    SettingsPage::Appearance => {
                        appearance(ui, model, theme, actions);
                        layouts(ui, theme, model.ui);
                    }
                    SettingsPage::Library => {
                        synesthesia(ui, theme, &model.ui.synesthesia, actions);
                        watch_folder(ui, theme, &mut model.ui.watch, model.picking, actions);
                    }
                    SettingsPage::Remote => {
                        super::lan::show(ui, theme, &mut model.lan, actions);
                        super::lan::control(ui, theme, &mut model.lan, model.picking, actions);
                        super::lan::network(ui, theme, &mut model.lan);
                    }
                    SettingsPage::Projectors => {
                        super::projectors::show(ui, theme, &mut model.projectors, actions);
                    }
                    SettingsPage::Shortcuts => keyboard(
                        ui,
                        theme,
                        model.remote.bindings,
                        &mut model.ui.hotkeys,
                        actions,
                    ),
                    SettingsPage::About => about(ui, theme, model.gl_summary),
                }
                ui.add_space(SPACE_LG);
            });
        });
}

fn output(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    form::group(ui, theme, "Display", |group| {
        let visible = model.output_visible;
        let (icon, word, colour) = if visible {
            (Icon::Check, "Shown", theme.success)
        } else {
            (Icon::Info, "Hidden", theme.text_muted)
        };
        group.row(
            Row::new(OUTPUT_WINDOW)
                .status(icon, word, colour)
                .description(
                    "It starts hidden. The scene keeps rendering while it is, and the preview \
                     shows it.",
                ),
            |ui| {
                let fullscreen = if model.fullscreen && visible {
                    "Exit fullscreen"
                } else {
                    "Fullscreen"
                };
                if widgets::button(ui, theme, Some(Icon::Fullscreen), fullscreen)
                    .on_hover_text(
                        "Fullscreen on the chosen monitor (F11); shows the output if it is \
                         hidden.",
                    )
                    .clicked()
                {
                    actions.push(Action::ToggleFullscreen);
                }
                let label = if visible { "Hide" } else { "Show" };
                if widgets::button(ui, theme, Some(Icon::Output), label).clicked() {
                    actions.push(Action::SetOutputVisible(!visible));
                }
            },
        );
        group.row(
            Row::new(MONITOR).description("The display the output goes fullscreen on."),
            |ui| monitor_picker(ui, model, actions),
        );
    });
    form::group(ui, theme, "Rendering", |group| {
        let timed = model.perf.has_timer;
        let mut adaptive = model.perf.adaptive;
        let row = Row::new(ADAPTIVE).enabled(timed).description(if timed {
            "Lowers the render scale when the GPU misses its frame budget, and raises it again \
             when there is room."
        } else {
            "Not available: this graphics context cannot time the GPU."
        });
        if group.toggle(row, &mut adaptive).changed() {
            actions.push(Action::SetAdaptive(adaptive));
        }
        let description = if model.perf.adaptive && timed {
            "The most adaptive resolution renders at, as a share of the output's size. It \
             applies when you let go."
        } else {
            "The share of the output's size scenes render at. It applies when you let go."
        };
        group.row(Row::new(SCALE).description(description), |ui| {
            let slider = form::slider(ui, model.render_scale, RENDER_SCALE, Some(0.05), |v| {
                format!("{:.0}%", v * 100.0)
            });
            if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
                actions.push(Action::SetRenderScale(*model.render_scale));
            }
        });
    });
}

/// The output-monitor combo box.
pub fn monitor_picker(ui: &mut Ui, model: &Model<'_>, actions: &mut Vec<Action>) {
    let current = model
        .output_monitor
        .and_then(|index| model.monitors.get(index))
        .map_or("Output monitor", String::as_str);
    egui::ComboBox::from_id_salt("output-monitor")
        .selected_text(current)
        .show_ui(ui, |ui| {
            if model.monitors.is_empty() {
                ui.label("No monitors reported");
            }
            for (index, label) in model.monitors.iter().enumerate() {
                if ui
                    .selectable_label(model.output_monitor == Some(index), label)
                    .clicked()
                {
                    actions.push(Action::MoveToMonitor(index));
                }
            }
        });
}

fn appearance(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    form::group(ui, theme, "Look", |group| {
        let problems: Vec<String> = model
            .ui
            .catalog()
            .problems()
            .iter()
            .map(|(source, reason)| format!("Theme file {source} not used: {reason}"))
            .collect();
        let problems = problems.join(" ");
        let mut row = Row::new(THEME).description("Colours for this window, light or dark.");
        if !problems.is_empty() {
            row = row.note(&problems, theme.warning);
        }
        let current = model.ui.palette().name.clone();
        let mut chosen = None;
        group.row(row, |ui| {
            egui::ComboBox::from_id_salt("theme")
                .width(ui.available_width().min(200.0))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for palette in model.ui.catalog().themes() {
                        let label = format!(
                            "{}{}",
                            palette.name,
                            match palette.mode {
                                crate::palette::Mode::Dark => "",
                                crate::palette::Mode::Light => " (light)",
                            }
                        );
                        if ui
                            .selectable_label(palette.id == model.ui.theme_id(), label)
                            .clicked()
                        {
                            chosen = Some(palette.id.clone());
                        }
                    }
                });
        });
        if let Some(id) = chosen {
            model.ui.set_theme(&id);
        }
        let current = model.ui.density();
        let mut picked = None;
        group.row(
            Row::new(DENSITY).description(
                "Compact fits more on a small screen with several panes open: smaller text and \
                 tighter rows.",
            ),
            |ui| {
                // Right to left: the last is added first.
                for density in theme::Density::ALL.into_iter().rev() {
                    if ui
                        .selectable_label(current == density, density.label())
                        .clicked()
                    {
                        picked = Some(density);
                    }
                }
            },
        );
        if let Some(density) = picked {
            model.ui.set_density(density);
        }
        let mut reduced = model.ui.reduced_motion();
        if group
            .toggle(
                Row::new(REDUCE_MOTION).description(
                    "No transitions and no overshoot. STROBE_REDUCED_MOTION=1 forces it on.",
                ),
                &mut reduced,
            )
            .changed()
        {
            model.ui.set_reduced_motion(reduced);
        }
        let mut custom = model.ui.custom_title_bar;
        if group
            .toggle(
                Row::new(TITLE_BAR).description(
                    "Strobe's own title bar, with the health readouts in it. Turn it off to use \
                     the system's if dragging or snapping misbehaves on this machine.",
                ),
                &mut custom,
            )
            .changed()
        {
            actions.push(Action::SetCustomTitleBar(custom));
        }
    });
}

/// Synesthesia's own scenes in the library (D271): one switch, applied by
/// the app at once and saved, and the app's line saying what is listed or
/// why nothing is.
fn synesthesia(
    ui: &mut Ui,
    theme: &Palette,
    state: &super::state::SynesthesiaState,
    actions: &mut Vec<Action>,
) {
    form::group(ui, theme, "Synesthesia", |group| {
        let mut show = state.show;
        let row = Row::new(SHOW_SYNESTHESIA)
            .description(
                "Where Synesthesia is installed, list its scenes after your library's, \
                 read-only. Strobe only reads your install; copy a scene to your library to \
                 change it.",
            )
            .note(&state.note, theme.text_muted);
        if group.toggle(row, &mut show).changed() {
            actions.push(Action::SetShowSynesthesia(show));
        }
    });
}

/// The watched drop folder (D265): a switch, then the path and Apply, with
/// the app's line saying what is watched or why not. Nothing here touches
/// the disk: the app checks the folder when the action arrives.
fn watch_folder(
    ui: &mut Ui,
    theme: &Palette,
    watch: &mut super::state::WatchState,
    picking: bool,
    actions: &mut Vec<Action>,
) {
    form::group(ui, theme, "Watched folder", |group| {
        let mut on = watch.watching;
        let row = Row::new(WATCH).description(
            "A scene folder, or a folder of scenes, copied into it joins the library, a \
                 scene a frame, and nothing plays. What is in it when watching starts is left \
                 alone.",
        );
        // The app's line, unless it only repeats what the switch shows.
        let row = if watch.failed {
            row.note(&watch.note, theme.danger)
        } else if watch.watching {
            row.note(&watch.note, theme.text_muted)
        } else {
            row
        };
        if group.toggle(row, &mut on).changed() {
            actions.push(Action::SetWatchFolder {
                folder: watch.draft.trim().to_string(),
                enabled: on,
            });
        }
        let ready = !watch.draft.trim().is_empty();
        group.wide_row(
            Row::new(FOLDER).description("A folder beside the library, not the library itself."),
            |ui| {
                let apply = ui
                    .add_enabled_ui(ready, |ui| {
                        widgets::button(ui, theme, None, "Apply")
                            .on_hover_text("Watch this folder now, and from the next launch")
                    })
                    .inner;
                if widgets::browse_button(ui, theme, picking) {
                    actions.push(Action::Browse(crate::picker::Purpose::WatchFolder));
                }
                let width = ui.available_size_before_wrap().x;
                let field = form::field(ui, theme, &mut watch.draft, "D:\\VJ\\inbox", width);
                let entered =
                    field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if ready && (apply.clicked() || entered) {
                    actions.push(Action::SetWatchFolder {
                        folder: watch.draft.trim().to_string(),
                        enabled: true,
                    });
                }
            },
        );
    });
}

/// Named layouts: save the panes as they are, apply or delete one (D215).
fn layouts(ui: &mut Ui, theme: &Palette, state: &mut super::state::UiState) {
    form::group(ui, theme, "Layouts", |group| {
        let name = state.layout_name.trim().to_string();
        let replacing = state.layouts.contains_key(&name);
        let mut save = false;
        group.row(
            Row::new(SAVE_LAYOUT)
                .description("The open panes, their order and widths, under a name."),
            |ui| {
                let label = if replacing { "Replace" } else { "Save" };
                save = ui
                    .add_enabled_ui(!name.is_empty(), |ui| {
                        widgets::button(ui, theme, None, label).on_hover_text(if replacing {
                            "Replace this layout with the panes as they are now"
                        } else {
                            "Save the open panes, their order and widths under this name"
                        })
                    })
                    .inner
                    .clicked();
                let width = ui.available_size_before_wrap().x;
                form::field(
                    ui,
                    theme,
                    &mut state.layout_name,
                    "Prep, Show, Debug...",
                    width,
                );
            },
        );
        if save && state.save_layout(&name) {
            state.layout_name.clear();
        }
        let names: Vec<String> = state.layouts.keys().cloned().collect();
        let mut apply = None;
        let mut delete = None;
        for name in &names {
            group.row(
                Row::new(name).description("Opens these panes, in this order, at these widths."),
                |ui| {
                    if widgets::button(ui, theme, None, "Delete").clicked() {
                        delete = Some(name.clone());
                    }
                    if widgets::button(ui, theme, None, "Apply").clicked() {
                        apply = Some(name.clone());
                    }
                },
            );
        }
        if let Some(name) = apply {
            state.apply_layout(&name);
        }
        if let Some(name) = delete {
            state.delete_layout(&name);
        }
    });
}

fn keyboard(
    ui: &mut Ui,
    theme: &Palette,
    bindings: &Bindings,
    editor: &mut HotkeyEditor,
    actions: &mut Vec<Action>,
) {
    if let Some(command) = editor.capturing {
        match keys::captured(ui.ctx()) {
            keys::Capture::Chord(chord) => {
                editor.capturing = None;
                actions.push(Action::SetHotkey {
                    command,
                    chord: Some(chord),
                    steal: false,
                });
            }
            keys::Capture::Waiting => {}
            keys::Capture::Cancel => editor.capturing = None,
        }
    }
    form::group(ui, theme, SHORTCUTS_GROUP, |group| {
        for command in CommandId::ALL {
            let capturing = editor.capturing == Some(command);
            let custom = bindings.chord(command) != command.chord();
            let clash = editor
                .clash
                .filter(|(wanted, _, _)| *wanted == command)
                .map(|(_, chord, holder)| {
                    (
                        format!("{} is {}.", chord.label(), holder.name()),
                        chord,
                        holder,
                    )
                });
            let mut row = Row::new(command.name());
            if custom {
                row = row.description("Changed from the default.");
            }
            if let Some((text, _, _)) = &clash {
                row = row.note(text, theme.warning);
            }
            group.row(row, |ui| {
                if capturing {
                    if widgets::button(ui, theme, None, "Cancel").clicked() {
                        editor.capturing = None;
                    }
                } else {
                    if widgets::button(ui, theme, None, "Change")
                        .on_hover_text("Then press the new key; Esc cancels")
                        .clicked()
                    {
                        editor.capturing = Some(command);
                        editor.clash = None;
                    }
                    if bindings.chord(command).is_some()
                        && widgets::button(ui, theme, None, "Clear")
                            .on_hover_text("No shortcut for this command")
                            .clicked()
                    {
                        actions.push(Action::SetHotkey {
                            command,
                            chord: None,
                            steal: false,
                        });
                    }
                    if custom
                        && widgets::button(ui, theme, Some(Icon::Reset), "Reset")
                            .on_hover_text(command.chord().map_or_else(
                                || "Back to no shortcut".to_owned(),
                                |c| format!("Back to {}", c.label()),
                            ))
                            .clicked()
                    {
                        actions.push(Action::ResetHotkey(command));
                    }
                }
                let label = if capturing {
                    "press a key".to_owned()
                } else {
                    bindings
                        .chord(command)
                        .map_or_else(|| "none".to_owned(), Chord::label)
                };
                let colour = if capturing {
                    theme.warning
                } else if custom {
                    theme.info
                } else {
                    theme.accent
                };
                widgets::keycap(ui, theme, &label, colour);
            });
            if let Some((_, chord, holder)) = clash {
                let mut done = false;
                group.block(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        if widgets::primary_button(ui, theme, "Move it here").clicked() {
                            actions.push(Action::SetHotkey {
                                command,
                                chord: Some(chord),
                                steal: true,
                            });
                            done = true;
                        }
                        ui.label(
                            egui::RichText::new(format!("{} will have no shortcut", holder.name()))
                                .font(theme::description_font())
                                .color(theme::rgb(theme.text_muted)),
                        );
                        if widgets::button(ui, theme, None, "Keep it there").clicked() {
                            done = true;
                        }
                    });
                });
                if done {
                    editor.clash = None;
                }
            }
        }
    });
    ui.add_space(SPACE_SM);
    ui.label(
        egui::RichText::new("Changes apply at once and are saved to show-control.json.")
            .font(theme::description_font())
            .color(theme::rgb(theme.text_faint)),
    );
}

/// The build line, made once: it does not change while the app runs.
fn build() -> &'static str {
    static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILD.get_or_init(strobe_diag::buildinfo::summary)
}

fn about(ui: &mut Ui, theme: &Palette, gl_summary: &str) {
    form::group(ui, theme, "Strobe", |group| {
        group.row(Row::new(VERSION).description(build()), |_| {});
        group.row(Row::new(GRAPHICS).description(gl_summary), |_| {});
        super::credits::project(group, theme);
    });
    super::credits::show(ui, theme);
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use anyhow::Result;
    use egui::{Pos2, Vec2};

    /// Every row search can find by its index entry, as (page, title): the
    /// shell's tests check each one is drawn on its page.
    pub(in crate::ui) fn indexed() -> impl Iterator<Item = Hit> {
        INDEX.iter().map(|entry| (entry.page, entry.title))
    }

    /// One pass of the watched-folder group; returns where each text was
    /// drawn.
    fn draw(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        watch: &mut super::super::state::WatchState,
        actions: &mut Vec<Action>,
    ) -> Vec<(String, egui::Rect)> {
        let theme = crate::palette::fallback().clone();
        let pass = crate::ui::testing::run(
            ctx,
            crate::ui::testing::input(Vec2::new(640.0, 480.0), events),
            |ui| watch_folder(ui, &theme, watch, false, actions),
        );
        texts(&pass)
    }

    fn texts(pass: &crate::ui::testing::Pass) -> Vec<(String, egui::Rect)> {
        pass.shapes
            .iter()
            .filter_map(|clipped| {
                let egui::Shape::Text(text) = &clipped.shape else {
                    return None;
                };
                Some((
                    text.galley.text().to_string(),
                    clipped.shape.visual_bounding_rect(),
                ))
            })
            .collect()
    }

    fn click(
        ctx: &egui::Context,
        at: Pos2,
        watch: &mut super::super::state::WatchState,
        actions: &mut Vec<Action>,
    ) {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = draw(
            ctx,
            vec![egui::Event::PointerMoved(at), button(true)],
            watch,
            actions,
        );
        let _ = draw(ctx, vec![button(false)], watch, actions);
    }

    fn find(drawn: &[(String, egui::Rect)], text: &str) -> Result<Pos2> {
        drawn
            .iter()
            .find(|(drawn, _)| drawn == text)
            .map(|(_, rect)| rect.center())
            .ok_or_else(|| anyhow::anyhow!("no {text:?} in {drawn:?}"))
    }

    #[test]
    fn apply_and_the_switch_ask_the_app_and_change_nothing_themselves() -> Result<()> {
        let ctx = egui::Context::default();
        let mut watch = super::super::state::WatchState {
            draft: "  D:/VJ/inbox ".to_string(),
            note: "Off".to_string(),
            ..super::super::state::WatchState::default()
        };
        let mut actions = Vec::new();
        let drawn = draw(&ctx, Vec::new(), &mut watch, &mut actions);
        click(&ctx, find(&drawn, "Apply")?, &mut watch, &mut actions);
        assert_eq!(
            actions,
            vec![Action::SetWatchFolder {
                folder: "D:/VJ/inbox".to_string(),
                enabled: true,
            }],
            "Apply asks to watch the typed folder, trimmed"
        );
        assert!(
            !watch.watching && !watch.enabled && watch.folder.is_empty(),
            "the pane changes nothing itself: the app checks and saves: {watch:?}"
        );
        actions.clear();
        watch.watching = true;
        let drawn = draw(&ctx, Vec::new(), &mut watch, &mut actions);
        click(&ctx, find(&drawn, WATCH)?, &mut watch, &mut actions);
        assert_eq!(
            actions,
            vec![Action::SetWatchFolder {
                folder: "D:/VJ/inbox".to_string(),
                enabled: false,
            }],
            "unticking a watch that runs asks to stop it"
        );
        Ok(())
    }

    #[test]
    fn the_synesthesia_switch_asks_the_app_and_changes_nothing_itself() -> Result<()> {
        let ctx = egui::Context::default();
        let theme = crate::palette::fallback().clone();
        let state = super::super::state::SynesthesiaState {
            show: true,
            note: "88 scenes from C:\\Program Files (x86)\\Synesthesia".to_string(),
        };
        let mut actions = Vec::new();
        let pass = |events: Vec<egui::Event>, actions: &mut Vec<Action>| {
            crate::ui::testing::run(
                &ctx,
                crate::ui::testing::input(Vec2::new(640.0, 480.0), events),
                |ui| synesthesia(ui, &theme, &state, actions),
            )
        };
        let drawn = texts(&pass(Vec::new(), &mut actions));
        let at = find(&drawn, SHOW_SYNESTHESIA)?;
        assert!(
            drawn.iter().any(|(text, _)| text.starts_with("88 scenes")),
            "the app's line shows: {drawn:?}"
        );
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let _ = pass(
            vec![egui::Event::PointerMoved(at), button(true)],
            &mut actions,
        );
        let _ = pass(vec![button(false)], &mut actions);
        assert_eq!(
            actions,
            vec![Action::SetShowSynesthesia(false)],
            "unticking asks the app to stop listing them"
        );
        assert!(state.show, "the pane changes nothing itself");
        Ok(())
    }

    #[test]
    fn search_finds_rows_by_title_by_word_and_by_command() {
        let hits = |query: &str| search(query).collect::<Vec<_>>();
        assert!(
            hits("reactiv").contains(&(SettingsPage::Audio, audio::REACTIVITY)),
            "a title prefix finds its row: {:?}",
            hits("reactiv")
        );
        assert!(
            hits("display")
                .iter()
                .all(|(page, _)| *page == SettingsPage::Output)
                && !hits("display").is_empty(),
            "a word the title lacks finds the right page: {:?}",
            hits("display")
        );
        assert!(
            hits("projector").iter().all(|(page, _)| {
                matches!(page, SettingsPage::Output | SettingsPage::Projectors)
            }) && hits("projector")
                .contains(&(SettingsPage::Projectors, super::super::projectors::CONTROL)),
            "projector finds the output's rows and projector power: {:?}",
            hits("projector")
        );
        assert!(
            hits("tap tempo")
                .iter()
                .any(|(page, _)| *page == SettingsPage::Shortcuts),
            "a command finds its shortcut row: {:?}",
            hits("tap tempo")
        );
        assert!(hits("zzzz").is_empty(), "nonsense finds nothing");
        assert!(
            hits("LAN").contains(&(SettingsPage::Remote, super::super::lan::REMOTE)),
            "case does not matter: {:?}",
            hits("LAN")
        );
    }

    #[test]
    fn a_page_is_found_by_its_name_in_any_case() {
        assert_eq!(
            SettingsPage::from_key("audio"),
            Some(SettingsPage::Audio),
            "lower case"
        );
        assert_eq!(
            SettingsPage::from_key("Remote"),
            Some(SettingsPage::Remote),
            "as titled"
        );
        assert_eq!(SettingsPage::from_key("nope"), None, "an unknown page");
    }

    #[test]
    fn every_page_has_a_title_icon_and_line() {
        for page in SettingsPage::ALL {
            assert!(!page.title().is_empty(), "{page:?} has a title");
            assert!(!page.lede().is_empty(), "{page:?} has a line under it");
        }
    }
}
