// ============================================================================
// Module:       strobe_app::ui::inspector
// Description:  The Controls and Edit tabs of the side panel: the playing
//               scene's controls, grouped by UI_GROUP, with per-group default
//               and random, then the output-wide meta group; and its tags,
//               picture, media and Generate controls.
//
// Dependencies: egui; strobe-scene (ControlBank); super::{controls, model,
//               theme, widgets, state}, crate::script
// ============================================================================

//! The control inspector (UI_DESIGN sections 9 to 11, UI_IMPLEMENTATION_PLAN
//! section 10).
//!
//! The inspector is *generated* from the scene's `ControlDef`s: nothing here
//! knows what a control is called or what range it has, and there is no
//! second taxonomy. Groups come from `UI_GROUP`, in the order a scene
//! declares them, ungrouped controls first.
//!
//! Each group has a header with a disclosure chevron (collapse state is the
//! operator's and is saved), and two small actions that act on the group
//! only: *default* (every control in the group eases back to its default,
//! and a bang is left alone) and *random* (every control in the group moves
//! to a random point in its range). Descriptions stay in tooltips until
//! asked for; there is no four-button toolbar beside every control.
//!
//! States: no scene ("Pick a scene"), a scene with no controls, a script
//! that is running, faulted or disabled (visible here without opening the
//! console).
//!
//! It draws two tabs of the side panel (D283), chosen by [`Part`]: the
//! panel draws the preview, the status strip and the tab bar above it, and
//! this draws from the scene's title down. *Controls* is the script's state
//! and the groups; *Edit* is the scene's tags and picture, its media,
//! Generate controls, and the button to the Editor.
//!
//! It also draws the library view's **Controls page** (D291), [`Part::Page`]:
//! the same groups, the same widgets and the same writes as the Controls
//! tab, laid out for the main view. The groups flow into as many columns as
//! [`PAGE_COLUMN`] fits (at most [`PAGE_COLUMNS_MAX`]), each to the
//! shortest column so far, and the rows are drawn at
//! [`controls::Scale::Touch`].

use super::controls;
use super::icon as icons;
use super::model::{Action, GeneratePanel, MediaFrom, MediaPanel, Model};
use super::tags;
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::generate::{Kept, Outcome, Review, Summary};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::script::ScriptState;
use egui::{Sense, Ui, Vec2};
use strobe_scene::control::{ControlBank, ControlKind};

/// Which half of the playing scene's panel is drawn (D283).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The Controls tab: the script's state, the scene's controls by
    /// group, then the output-wide meta group.
    Controls,
    /// The Edit tab: the scene's tags and picture, its media, Generate
    /// controls, and the way to the Editor.
    Edit,
    /// The library view's Controls page (D291): what the Controls tab
    /// shows, in columns and at touch size.
    Page,
}

/// The width one column of the Controls page aims for.
pub const PAGE_COLUMN: f32 = 340.0;

/// The most columns the Controls page uses.
pub const PAGE_COLUMNS_MAX: usize = 3;

/// How many columns of groups the Controls page has at `width`.
pub fn page_columns(width: f32) -> usize {
    if !width.is_finite() || width <= 0.0 {
        return 1;
    }
    ((width / PAGE_COLUMN).floor() as usize).clamp(1, PAGE_COLUMNS_MAX)
}

/// Draws the tab's contents.
pub fn show(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
    part: Part,
) {
    let has_scene = model.controls.is_some();
    let page = part == Part::Page;
    let scale = if page {
        controls::Scale::Touch
    } else {
        controls::Scale::Panel
    };

    // Everything scrolls as one, so a short window (a drawer at 150% on a
    // small laptop, U-09) still reaches every control rather than losing
    // them under the scene's tags.
    let mut chosen_pins = (None, None);
    egui::ScrollArea::vertical()
        .id_salt(if page {
            "controls-page-scroll"
        } else {
            "inspector-scroll"
        })
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if has_scene {
                ui.horizontal(|ui| {
                    let title = if model.scene_title.is_empty() {
                        model
                            .playing
                            .and_then(|i| model.library.entries.get(i))
                            .map_or("", |e| e.name.as_str())
                    } else {
                        model.scene_title
                    };
                    // The page's title is as large as a card's: the live
                    // scene is what the page is about.
                    let mut font = egui::TextStyle::Body.resolve(ui.style());
                    if page {
                        font.size += 3.0;
                    }
                    let galley = widgets::truncated(
                        ui,
                        title,
                        font,
                        theme.text,
                        (ui.available_width() - 36.0).max(40.0),
                    );
                    let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                    ui.painter()
                        .galley(rect.min, galley, theme::rgb(theme.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(
                            ui,
                            theme,
                            Icon::Refresh,
                            "Reload the scene from disk. Control values are kept.",
                        )
                        .clicked()
                        {
                            actions.push(Action::Reload);
                        }
                    });
                });
            }
            if part == Part::Edit
                && let Some(entry) = model.playing.and_then(|i| model.library.entries.get(i))
            {
                let capture = |ui: &mut Ui| {
                    if ui
                        .small_button("Use frame as picture")
                        .on_hover_text(
                            "Save what is on screen now as this scene's picture in the library \
                             (thumbnail.jpg in its folder)",
                        )
                        .clicked()
                    {
                        actions.push(Action::CaptureThumbnail);
                    }
                };
                let mut asked = Vec::new();
                tags::row(
                    ui,
                    theme,
                    tags::Place {
                        scene: &entry.name,
                        named: false,
                        read_only: entry.read_only().then_some(entry.dir.as_path()),
                    },
                    &entry.tags,
                    &mut model.ui.tag_draft,
                    capture,
                    &mut asked,
                );
                actions.append(&mut asked);
            }
            if part == Part::Edit {
                if let Some(media) = model.media {
                    media_row(ui, theme, media, &mut model.ui.media_draft, actions);
                }
                ui.add_space(SPACE_XS);
                if has_scene {
                    generate_strip(
                        ui,
                        theme,
                        model.generate,
                        &mut model.ui.generate_review,
                        actions,
                    );
                    if widgets::button(ui, theme, Some(Icon::Editor), "Open in Editor")
                        .on_hover_text(
                            "The scene's files, with Save and Save and reload (the EDIT view)",
                        )
                        .clicked()
                    {
                        actions.push(Action::OpenEditor(None));
                    }
                } else {
                    widgets::empty_state(
                        ui,
                        theme,
                        Icon::Editor,
                        "No scene playing",
                        "Tags, picture, media and the Editor belong to the playing scene. Click \
                         one in the library to make it live.",
                    );
                }
                return;
            }
            if let Some(state) = model.script {
                script_status(ui, theme, state);
            }
            ui.add_space(SPACE_XS);

            // Which controls already have a pinned macro, for the rows' pin
            // menu (D210).
            let (pinned_scene, pinned_meta) = pinned_names(&model.remote.remote.engine.macros);
            let mut scene_pins = controls::Pins {
                pinned: &pinned_scene,
                chosen: None,
            };
            let mut meta_pins = controls::Pins {
                pinned: &pinned_meta,
                chosen: None,
            };
            let ui_state = &mut *model.ui;
            let meta = model.meta.as_deref_mut();
            match model.controls.as_deref_mut() {
                Some(bank) if !bank.defs().is_empty() => {
                    groups(
                        ui,
                        theme,
                        bank,
                        meta,
                        ui_state,
                        &mut scene_pins,
                        &mut meta_pins,
                        scale,
                    );
                }
                Some(_) => {
                    widgets::empty_state(
                        ui,
                        theme,
                        Icon::Controls,
                        "This scene has no controls",
                        "It plays as authored. Generate controls makes a copy with some.",
                    );
                    if let Some(meta) = meta {
                        meta_section(ui, theme, meta, ui_state, &mut meta_pins, scale);
                    }
                }
                None => widgets::empty_state(
                    ui,
                    theme,
                    Icon::Library,
                    "No scene playing",
                    if page {
                        "Pick one in Scenes, above: a click makes it live, and its controls \
                         appear here."
                    } else {
                        "Click a scene in the library to make it live; its controls appear \
                         here."
                    },
                ),
            }
            chosen_pins = (scene_pins.chosen.take(), meta_pins.chosen.take());
        });
    let (scene_chosen, meta_chosen) = chosen_pins;
    for (chosen, meta) in [(scene_chosen, false), (meta_chosen, true)] {
        if let Some((name, pin)) = chosen {
            actions.push(Action::PinMacro { name, meta, pin });
        }
    }
}

/// What the playing scene shows behind `syn_Media`, where that comes from,
/// and the field that chooses another (D217). A scene whose shaders never
/// read `syn_Media` gets no row unless media was chosen anyway.
fn media_row(
    ui: &mut Ui,
    theme: &Palette,
    media: MediaPanel<'_>,
    draft: &mut Option<String>,
    actions: &mut Vec<Action>,
) {
    if !media.read && !media.chosen && draft.is_none() {
        return;
    }
    let Some(text) = draft.as_mut() else {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Media")
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
            let (what, hover) = media_words(media);
            let mut label =
                egui::RichText::new(what)
                    .small()
                    .color(theme::rgb(if media.showing.is_some() {
                        theme.text
                    } else {
                        theme.text_faint
                    }));
            if media.loading {
                label = label.italics();
            }
            ui.label(label).on_hover_text(hover);
            if !media.read {
                ui.label(
                    egui::RichText::new("(this scene does not read it)")
                        .small()
                        .color(theme::rgb(theme.text_faint)),
                );
            }
            if ui
                .small_button("Choose")
                .on_hover_text("Show a picture, or a folder of pictures, behind syn_Media")
                .clicked()
            {
                *draft = Some(
                    media
                        .showing
                        .filter(|_| media.from == MediaFrom::Chosen)
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                );
            }
            if media.chosen
                && ui
                    .small_button("Scene's own")
                    .on_hover_text("Give each scene its own media back")
                    .clicked()
            {
                actions.push(Action::SetMedia(None));
            }
        });
        return;
    };
    let mut done = false;
    ui.horizontal(|ui| {
        let field = ui.add(
            egui::TextEdit::singleline(text)
                .hint_text("A picture, or a folder of pictures")
                .desired_width((ui.available_width() - 120.0).max(80.0)),
        );
        let entered = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let path = text.trim();
        if (ui.small_button("Use").clicked() || entered) && !path.is_empty() {
            actions.push(Action::SetMedia(Some(std::path::PathBuf::from(path))));
            done = true;
        }
        if ui.small_button("Cancel").clicked() {
            done = true;
        }
    });
    if done {
        *draft = None;
    }
}

/// The media row's words: what shows, and a hover line saying where it
/// comes from.
fn media_words(media: MediaPanel<'_>) -> (String, String) {
    let Some(path) = media.showing else {
        return (
            "None".to_string(),
            "Nothing shows behind syn_Media".to_string(),
        );
    };
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let from = match media.from {
        MediaFrom::Scene => "the scene's own",
        MediaFrom::Cue => "named by this cue",
        MediaFrom::Chosen => "chosen",
    };
    let state = if media.loading { ", loading" } else { "" };
    (
        format!("{name} ({from}{state})"),
        path.display().to_string(),
    )
}

/// The names of the scene controls and meta controls that a pinned macro
/// drives as a whole (not one component).
fn pinned_names(macros: &strobe_io::MacroSet) -> (Vec<String>, Vec<String>) {
    let mut scene = Vec::new();
    let mut meta = Vec::new();
    for found in macros.pinned() {
        match &found.target {
            strobe_io::Target::Control {
                name,
                component: None,
            } => scene.push(name.clone()),
            strobe_io::Target::Meta {
                name,
                component: None,
            } => meta.push(name.clone()),
            strobe_io::Target::Control { .. }
            | strobe_io::Target::Meta { .. }
            | strobe_io::Target::Command { .. }
            | strobe_io::Target::Cue { .. }
            | strobe_io::Target::FavSlot { .. }
            | strobe_io::Target::Preset { .. }
            | strobe_io::Target::Macro { .. }
            | strobe_io::Target::Action(_) => {}
        }
    }
    (scene, meta)
}

/// "Generate controls": the button, the worker's progress, or its result
/// for review (D202). The worker runs in its own process, so the show keeps
/// playing; the result is a new scene, and the source is never changed.
fn generate_strip(
    ui: &mut Ui,
    theme: &Palette,
    panel: GeneratePanel<'_>,
    reviews: &mut Vec<Review>,
    actions: &mut Vec<Action>,
) {
    if let Some((scene, seconds)) = panel.running {
        widgets::banner(
            ui,
            theme,
            Tone::Info,
            &format!("Measuring {scene} for controls ({seconds:.0} s). The show keeps playing."),
            |ui| {
                if widgets::button(ui, theme, None, "Cancel").clicked() {
                    actions.push(Action::CancelGenerate);
                }
            },
        );
        ui.add_space(SPACE_XS);
        return;
    }
    match panel.outcome {
        Some(Outcome::Done(summary)) => review(ui, theme, summary, reviews, actions),
        Some(Outcome::Failed { message }) => {
            widgets::banner(
                ui,
                theme,
                Tone::Error,
                &format!("No controls generated: {message}"),
                |ui| {
                    if widgets::button(ui, theme, None, "Dismiss").clicked() {
                        actions.push(Action::DismissGenerated);
                    }
                },
            );
            ui.add_space(SPACE_XS);
        }
        None => {
            let (label, tip) = if panel.authored {
                (
                    "Add generated controls",
                    "Measure the scene's literal numbers in the background and write a copy,                      <name>-auto, with the strongest ones added beside the controls it has.                      This scene is not changed.",
                )
            } else {
                (
                    "Generate controls",
                    "Measure the scene's literal numbers in the background and write a copy,                      <name>-auto, with the strongest ones as controls. This scene is not changed.",
                )
            };
            if widgets::button(ui, theme, Some(Icon::Controls), label)
                .on_hover_text(tip)
                .clicked()
            {
                actions.push(Action::GenerateControls);
            }
            ui.add_space(SPACE_XS);
        }
    }
}

/// The worker's result: what it made, how faithful the copy is, each
/// control to keep or drop and re-range (D219), and the two ways on.
fn review(
    ui: &mut Ui,
    theme: &Palette,
    summary: &Summary,
    reviews: &mut Vec<Review>,
    actions: &mut Vec<Action>,
) {
    if reviews.len() != summary.kept.len() {
        *reviews = summary.kept.iter().map(Review::of).collect();
    }
    theme::card(theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let Some(written) = summary.written.as_ref() else {
            ui.label(
                egui::RichText::new(format!(
                    "No useful controls in {}: {} literal(s) measured, none moved the picture enough.",
                    summary.scene, summary.candidates
                ))
                .color(theme::rgb(theme.text)),
            );
            if widgets::button(ui, theme, None, "Dismiss").clicked() {
                actions.push(Action::DismissGenerated);
            }
            return;
        };
        let name = written
            .file_name()
            .map(|n| n.to_string_lossy().trim_end_matches(".synScene").to_owned())
            .unwrap_or_default();
        let keeping = reviews.iter().filter(|r| r.keep).count();
        ui.label(
            egui::RichText::new(format!(
                "{keeping} of {} control(s) kept for {}, written as {name}",
                summary.kept.len(),
                summary.scene
            ))
            .strong()
            .color(theme::rgb(theme.text)),
        );
        let faithful = summary
            .fidelity
            .map_or_else(|| "not measured".to_owned(), |f| format!("{:.1}% off", f * 100.0));
        ui.label(
            egui::RichText::new(format!(
                "At their defaults the copy matches the original ({faithful}). This scene is unchanged."
            ))
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
        ui.add_space(SPACE_XS);
        let strongest = summary
            .kept
            .iter()
            .filter_map(|k| k.score)
            .fold(f32::EPSILON, f32::max);
        for (kept, edit) in summary.kept.iter().zip(reviews.iter_mut()) {
            // Wrapped: a row is wider than a pane at its narrowest, and an
            // unwrapped one pushed the dock under the toolbar (U-09).
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut edit.keep, "")
                    .on_hover_text("Keep this control; untick to leave the number as it was");
                // The bar in a fixed column, so a long colour range never
                // runs under it.
                let fraction = kept.score.unwrap_or(0.0) / strongest;
                // The bar is display only; its hover text is all the response
                // is for.
                let _ = widgets::meter(ui, theme, fraction, theme.accent, 36.0)
                    .on_hover_text("How much the picture moves with it, against the strongest");
                if edit.keep {
                    // Renamable while kept (D246); checked when applied.
                    ui.add(
                        egui::TextEdit::singleline(&mut edit.name)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(110.0),
                    )
                    .on_hover_text(format!(
                        "The control's name in the copy (proposed: {}). Letters, digits and _.",
                        kept.name
                    ));
                } else {
                    whole(
                        ui,
                        egui::RichText::new(&kept.name)
                            .monospace()
                            .color(theme::rgb(theme.text_faint)),
                    );
                }
                let slider = kept.default.len() == 1 && kept.min.len() == 1;
                if slider && edit.keep {
                    whole(
                        ui,
                        egui::RichText::new(format!("{} · {:.2}", kept.group, kept.default[0]))
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    )
                    .on_hover_text(&kept.context);
                    range_fields(ui, kept, edit);
                } else {
                    whole(
                        ui,
                        egui::RichText::new(format!("{} · {}", kept.group, range(kept)))
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    )
                    .on_hover_text(&kept.context);
                }
            });
        }
        for line in &summary.suggestions {
            ui.label(
                egui::RichText::new(format!("Audio idea: {line}"))
                    .small()
                    .color(theme::rgb(theme.text_faint)),
            );
        }
        ui.add_space(SPACE_XS);
        let edited = summary
            .kept
            .iter()
            .zip(reviews.iter())
            .any(|(kept, edit)| Review::of(kept) != *edit);
        ui.horizontal_wrapped(|ui| {
            let open = if edited {
                format!("Apply and open {name}")
            } else {
                format!("Open {name}")
            };
            if widgets::primary_button(ui, theme, &open)
                .on_hover_text(if edited {
                    "Write the copy again with the controls kept and their ranges, then open it"
                } else {
                    "Open the copy"
                })
                .clicked()
            {
                actions.push(Action::OpenGenerated);
            }
            if widgets::button(ui, theme, None, "Discard copy")
                .on_hover_text("Delete the copy. The original scene is not touched.")
                .clicked()
            {
                actions.push(Action::DiscardGenerated);
            }
        });
    });
    ui.add_space(SPACE_XS);
}

/// A label that moves to the next line whole in a wrapped row, rather
/// than breaking across two.
fn whole(ui: &mut Ui, text: egui::RichText) -> egui::Response {
    ui.add(egui::Label::new(text).wrap_mode(egui::TextWrapMode::Extend))
}

/// A kept slider's range, edited in place. The bounds stay either side of
/// the default, so the control can still reach the original value.
fn range_fields(ui: &mut Ui, kept: &Kept, edit: &mut Review) {
    let default = kept.default.first().copied().unwrap_or(0.0);
    let span = (edit.max - edit.min).abs().max(0.01);
    let speed = span / 200.0;
    ui.add(
        egui::DragValue::new(&mut edit.min)
            .speed(speed)
            .range(f32::MIN..=default)
            .max_decimals(3),
    )
    .on_hover_text("The lowest the control goes");
    ui.label("–");
    ui.add(
        egui::DragValue::new(&mut edit.max)
            .speed(speed)
            .range(default..=f32::MAX)
            .max_decimals(3),
    )
    .on_hover_text("The highest the control goes");
}

/// `0.5 (0–2)` for a slider, `(0.8, 0.3, 0.1)` for a colour.
fn range(kept: &Kept) -> String {
    let join = |values: &[f32]| {
        values
            .iter()
            .map(|v| format!("{v:.2}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (
        kept.default.as_slice(),
        kept.min.as_slice(),
        kept.max.as_slice(),
    ) {
        ([value], [min], [max]) => format!("{value:.2} ({min:.2}–{max:.2})"),
        (values, _, _) => format!("({})", join(values)),
    }
}

/// One line (or a short paragraph) on how the scene's script is doing: a
/// dead script must be visible without opening the console.
fn script_status(ui: &mut Ui, theme: &Palette, state: &ScriptState) {
    match state {
        ScriptState::Running => {
            ui.horizontal(|ui| {
                icons::show(
                    ui,
                    Icon::Check,
                    icons::DISCLOSURE,
                    theme::rgb(theme.success),
                );
                ui.label(
                    egui::RichText::new("script.js running")
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            });
        }
        ScriptState::Faulted {
            consecutive,
            message,
        } => widgets::banner(
            ui,
            theme,
            Tone::Warning,
            &format!("script.js fault ({consecutive} in a row): {message}"),
            |_| {},
        ),
        ScriptState::Disabled { message } => widgets::banner(
            ui,
            theme,
            Tone::Error,
            &format!("script.js disabled: {message}. Reload the scene to run it again."),
            |_| {},
        ),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one call site; the bank, its meta bank, the state and the pins are each needed"
)]
fn groups(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    meta: Option<&mut ControlBank>,
    state: &mut super::state::UiState,
    pins: &mut controls::Pins<'_>,
    meta_pins: &mut controls::Pins<'_>,
    scale: controls::Scale,
) {
    let groups = controls::group_order(bank.defs());
    // The pane's one scroll area is around everything below its header.
    ui.scope(|ui| {
        let columns = match scale {
            controls::Scale::Touch => page_columns(ui.available_width()),
            controls::Scale::Panel => 1,
        };
        if columns > 1 {
            // Each group goes to the column with the fewest rows so far: a
            // fixed array, so the layout allocates nothing.
            ui.columns(columns, |cols| {
                let mut load = [0_usize; PAGE_COLUMNS_MAX];
                for (name, members) in &groups {
                    let slot = (0..columns)
                        .min_by_key(|c| load.get(*c).copied().unwrap_or(usize::MAX))
                        .unwrap_or(0);
                    if let Some(rows) = load.get_mut(slot) {
                        *rows += members.len() + 1;
                    }
                    if let Some(col) = cols.get_mut(slot) {
                        group(
                            col,
                            theme,
                            bank,
                            state,
                            pins,
                            name.as_deref(),
                            members,
                            scale,
                        );
                    }
                }
            });
        } else {
            for (name, members) in &groups {
                group(
                    ui,
                    theme,
                    bank,
                    state,
                    pins,
                    name.as_deref(),
                    members,
                    scale,
                );
            }
        }
        // Bangs are left alone by default and random; say so once, near
        // the end, only if the scene has any.
        let has_bang = bank
            .defs()
            .iter()
            .any(|def| matches!(def.kind, ControlKind::Bang(_)));
        if has_bang {
            ui.label(
                egui::RichText::new("Default and random leave bangs alone.")
                    .small()
                    .color(theme::rgb(theme.text_faint)),
            );
        }
        if let Some(meta) = meta {
            meta_section(ui, theme, meta, state, meta_pins, scale);
        }
    });
}

/// One group: its header (none for the ungrouped controls, `name` `None`)
/// and, unless collapsed, its rows.
#[expect(
    clippy::too_many_arguments,
    reason = "a group's whole context; both layouts call it the same way"
)]
fn group(
    ui: &mut Ui,
    theme: &Palette,
    bank: &mut ControlBank,
    state: &mut super::state::UiState,
    pins: &mut controls::Pins<'_>,
    name: Option<&str>,
    members: &[usize],
    scale: controls::Scale,
) {
    match name {
        None => controls::rows(ui, theme, bank, members, Some(&mut *pins), scale),
        Some(name) => {
            let collapsed = state.is_collapsed(name);
            match group_header(ui, theme, name, collapsed, members.len()) {
                HeaderAction::None => {}
                HeaderAction::Toggle => state.set_collapsed(name, !collapsed),
                HeaderAction::Default => {
                    for &index in members {
                        bank.set_default(index);
                    }
                }
                HeaderAction::Random => {
                    for &index in members {
                        bank.randomize(index, || state.random_unit());
                    }
                }
            }
            if !collapsed {
                ui.indent(("group", name), |ui| {
                    controls::rows(ui, theme, bank, members, Some(&mut *pins), scale);
                });
            }
        }
    }
    ui.add_space(SPACE_SM);
}

/// The output-wide meta controls (master, colour, mirror, audio gains),
/// drawn under the scene's own. They belong to the output, not the scene:
/// they are applied after the last pass (D180), and a script reaches them
/// with a `meta/` prefix. Collapsed by the operator, saved with the layout.
fn meta_section(
    ui: &mut Ui,
    theme: &Palette,
    meta: &mut ControlBank,
    state: &mut super::state::UiState,
    pins: &mut controls::Pins<'_>,
    scale: controls::Scale,
) {
    const NAME: &str = "Output (meta)";
    let collapsed = state.is_collapsed(NAME);
    ui.separator();
    ui.horizontal(|ui| {
        // The chevron points the way the group headers' do: right while
        // closed, down while open.
        let (chevron, label) = if collapsed {
            (Icon::ChevronRight, "Output (meta): show")
        } else {
            (Icon::ChevronDown, "Output (meta): hide")
        };
        if widgets::button(ui, theme, Some(chevron), label)
            .on_hover_text("Master, colour, mirror and audio gains for the whole output")
            .clicked()
        {
            state.set_collapsed(NAME, !collapsed);
        }
        if widgets::button(ui, theme, Some(Icon::Reset), "Reset")
            .on_hover_text("Return every output control to neutral")
            .clicked()
        {
            for index in 0..meta.defs().len() {
                meta.set_default(index);
            }
        }
    });
    if collapsed {
        return;
    }
    for (group, members) in controls::group_order(meta.defs()) {
        if let Some(name) = group {
            ui.label(
                egui::RichText::new(name.to_uppercase())
                    .small()
                    .color(theme::rgb(theme.text_faint)),
            );
        }
        controls::rows(ui, theme, meta, &members, Some(&mut *pins), scale);
        ui.add_space(SPACE_XS);
    }
}

/// What a group header's click did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderAction {
    None,
    Toggle,
    Default,
    Random,
}

/// A group header: a chevron that turns, the name, the member count, and
/// the group's two actions.
fn group_header(
    ui: &mut Ui,
    theme: &Palette,
    name: &str,
    collapsed: bool,
    count: usize,
) -> HeaderAction {
    let mut action = HeaderAction::None;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    let id = ui.scope_id().with(("group-header", name));
    let fill = widgets::hover_fill(
        ui,
        response.id,
        response.hovered(),
        theme.panel,
        theme.raised,
    );
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), fill);
    // Collapsed points right; open points down: a quarter turn.
    let turn = super::motion::toggle(ui.ctx(), id, !collapsed, super::motion::QUICK);
    icons::paint_rotated(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 12.0, rect.center().y),
            Vec2::splat(icons::DISCLOSURE),
        ),
        Icon::ChevronRight,
        theme::rgb(theme.text_muted),
        turn * 0.25,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name.to_uppercase(),
        egui::FontId::new(11.0, egui::FontFamily::Proportional),
        theme::rgb(theme.text),
    );
    ui.painter().text(
        egui::pos2(
            rect.left() + 24.0 + 7.0 * name.chars().count() as f32 + 14.0,
            rect.center().y,
        ),
        egui::Align2::LEFT_CENTER,
        count.to_string(),
        egui::FontId::new(10.0, egui::FontFamily::Proportional),
        theme::rgb(theme.text_faint),
    );
    if response.clicked() {
        action = HeaderAction::Toggle;
    }
    // The two actions sit over the right end of the header and take their
    // clicks before the header does.
    let buttons = egui::Rect::from_min_max(
        egui::pos2(rect.right() - 56.0, rect.top() + 2.0),
        egui::pos2(rect.right() - 2.0, rect.bottom() - 2.0),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(buttons)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            if widgets::icon_button(
                ui,
                theme,
                Icon::Refresh,
                "Random: move every control in this group to a random value",
            )
            .clicked()
            {
                action = HeaderAction::Random;
            }
            if widgets::icon_button(
                ui,
                theme,
                Icon::Reset,
                "Default: return every control in this group to its default",
            )
            .clicked()
            {
                action = HeaderAction::Default;
            }
        },
    );
    action
}
