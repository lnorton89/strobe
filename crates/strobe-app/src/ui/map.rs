// ============================================================================
// Module:       strobe_app::ui::map
// Description:  The MAP pane: OSC and MIDI connections, the Performance Macro
//               set, the mapping list with learn, and the activity monitor.
//
// Dependencies: egui; strobe-io (Engine, Mapping, Target, State); super::{
//               model, theme, widgets, icon}, crate::{remote, commands}
// ============================================================================

//! The Mappings pane (UI_DESIGN section 15, D177).
//!
//! Learn first, then adjust:
//!
//! ```text
//! MAPPINGS                                                     x
//! CONNECTIONS
//!   [x] OSC   Listening on 127.0.0.1:9000     port [9000]
//!       [ ] All network interfaces   [x] Built-in /strobe/... addresses
//!   [x] MIDI input                        [Refresh]
//!       [x] nanoKONTROL2                  Connected
//! (a banner while learn is armed, or when a source is already mapped)
//! ACTIVITY                                                   Clear
//!   12.3s  CC 21 = 64  ch 1  [nanoKONTROL2]    applied   x37
//! MACROS
//!   [pin] Speed                                       [Learn] [x]
//!         Resolved   SCENE speed
//!         CC 21 ch 1
//! MAPPINGS                          [search]          [Add...]
//!   CC 21 ch 1 [nanoKONTROL2]               [on] [Learn] [x]
//!         Resolved   -> SCENE speed
//! ```
//!
//! Every row says its state in a word and an icon, never colour alone
//! (resolved, unavailable, broken, disabled, learning, conflict). A mapping
//! to a control the playing scene does not have is **unavailable**, kept,
//! muted, and says which control it is waiting for.
//!
//! Learn is the primary path: choose the target (*Add...*, or *Learn* on an
//! existing row), then move a fader, press a pad or send an OSC message. The
//! pane edits mappings and macros in place (they are the operator's data and
//! the mapper's own); everything that touches a worker, a file or the
//! scene goes back to the app as an [`Action`].

use super::icon as icons;
use super::model::{Action, AddKind, MapUi, Model};
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::color::Rgb;
use crate::commands::CommandId;
use crate::icon::Icon;
use crate::palette::Palette;
use crate::remote::{Evaluated, OscState, Remote};
use crate::workspace::Pane;
use egui::Ui;
use strobe_io::engine::Armed;
use strobe_io::target::{Resolution, State};
use strobe_io::{ButtonMode, Curve, Macro, Mapping, OscSettings, Source, Takeover, Target};
use strobe_scene::control::{ControlBank, ControlKind};

/// Width a row keeps for its buttons (Learn, delete, a checkbox), so the
/// name takes only what is left and the row never outgrows the pane.
const ROW_BUTTONS: f32 = 190.0;

/// Activity lines drawn (the log keeps more).
const ACTIVITY_LINES: usize = 8;

/// The icon and colour that say a state.
pub fn state_look(theme: &Palette, state: State) -> (Icon, Rgb) {
    match state {
        State::Resolved => (Icon::Check, theme.success),
        State::Unavailable => (Icon::Info, theme.text_muted),
        State::Broken => (Icon::Error, theme.danger),
        State::Disabled => (Icon::Close, theme.text_faint),
        State::Learning => (Icon::Refresh, theme.accent),
        State::Conflict => (Icon::Warning, theme.warning),
    }
}

/// A chip that says a state in words and an icon.
pub fn state_chip(ui: &mut Ui, theme: &Palette, resolution: &Resolution) {
    let (icon, colour) = state_look(theme, resolution.state);
    let response = widgets::status_chip(ui, icon, resolution.state.label(), colour);
    if !resolution.reason.is_empty() {
        response.on_hover_text(&resolution.reason);
    }
}

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let Model {
        remote: panel,
        controls,
        meta,
        ui: ui_state,
        ..
    } = model;
    if widgets::pane_header(ui, theme, Icon::Mapping, Pane::Map.title(), |_| {}) {
        ui_state.panes.close(Pane::Map);
    }
    let scene = controls.as_deref();
    let meta = meta.as_deref();
    let eval = panel.remote.evaluate(scene, meta, panel.presets);
    let presets = panel.presets;
    let remote = &mut *panel.remote;
    let map = &mut *panel.ui;
    egui::ScrollArea::vertical()
        .id_salt("map-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            learn_banner(ui, theme, remote);
            widgets::section(ui, theme, "Connections");
            osc_block(ui, theme, remote, map, actions);
            ui.add_space(SPACE_SM);
            midi_block(ui, theme, remote, actions);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Activity");
            activity_block(ui, theme, remote);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Macros");
            macros_block(ui, theme, remote, map, &eval);
            ui.add_space(SPACE_SM);
            widgets::section(ui, theme, "Mappings");
            let targets = Targets {
                scene,
                meta,
                presets,
            };
            add_form(ui, theme, remote, map, &targets);
            mappings_block(ui, theme, remote, map, &eval);
        });
}

/// What the add form can pick from.
struct Targets<'a> {
    scene: Option<&'a ControlBank>,
    meta: Option<&'a ControlBank>,
    presets: &'a [String],
}

// ---------------------------------------------------------------- learn

/// The banner shown while learn is armed, or a learned source is taken.
fn learn_banner(ui: &mut Ui, theme: &Palette, remote: &mut Remote) {
    let engine = &mut remote.engine;
    if let Some(conflict) = engine.conflict().cloned() {
        let holder = engine
            .mappings
            .get(conflict.existing)
            .map_or_else(String::new, |m| m.target.label());
        let message = format!("{} is already mapped to {holder}.", conflict.source.label());
        let mut answer = None;
        widgets::banner(ui, theme, Tone::Warning, &message, |ui| {
            if widgets::button(ui, theme, None, "Cancel").clicked() {
                answer = Some(false);
            }
            if widgets::button(ui, theme, None, "Replace")
                .on_hover_text("Remove the old mapping and bind this source to the new target")
                .clicked()
            {
                answer = Some(true);
            }
        });
        ui.add_space(SPACE_SM);
        if let Some(replace) = answer {
            remote.learned = engine.resolve_conflict(replace);
        }
        return;
    }
    let Some(armed) = engine.armed().cloned() else {
        return;
    };
    let what = match &armed {
        Armed::New(target) => target.label(),
        Armed::Relearn(index) => engine
            .mappings
            .get(*index)
            .map_or_else(String::new, |m| m.target.label()),
    };
    let mut cancel = false;
    widgets::banner(
        ui,
        theme,
        Tone::Info,
        &format!("Learning {what}: move a fader, press a pad or send an OSC message."),
        |ui| {
            cancel = widgets::button(ui, theme, None, "Cancel").clicked();
        },
    );
    ui.add_space(SPACE_SM);
    if cancel {
        engine.cancel_learn();
    }
}

// ---------------------------------------------------------- connections

fn osc_block(
    ui: &mut Ui,
    theme: &Palette,
    remote: &Remote,
    map: &mut MapUi,
    actions: &mut Vec<Action>,
) {
    let mut settings: OscSettings = remote.osc.clone();
    let mut changed = false;
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut settings.enabled, "OSC")
            .on_hover_text(
                "Listen for OSC messages over UDP. Off until you switch it on. \
                 Only this computer can send to it unless you allow every network interface.",
            )
            .changed()
        {
            changed = true;
        }
        match &remote.osc_state {
            OscState::Listening(addr) => {
                widgets::status_chip(
                    ui,
                    Icon::Check,
                    &format!("Listening on {addr}"),
                    theme.success,
                );
            }
            OscState::Off => {
                let word = if settings.enabled { "Starting" } else { "Off" };
                widgets::status_chip(ui, Icon::Info, word, theme.text_muted);
            }
            OscState::Error(_) => {
                widgets::status_chip(ui, Icon::Error, "Cannot listen", theme.danger);
            }
        }
    });
    if let OscState::Error(why) = &remote.osc_state {
        ui.label(
            egui::RichText::new(why)
                .small()
                .color(theme::rgb(theme.danger)),
        );
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Port")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        let edit = ui.add(
            egui::TextEdit::singleline(&mut map.port_text)
                .desired_width(56.0)
                .hint_text("9000"),
        );
        if !edit.has_focus() {
            map.port_text = settings.port.to_string();
        }
        if edit.lost_focus()
            && let Ok(port) = map.port_text.trim().parse::<u16>()
            && port != settings.port
        {
            settings.port = port;
            changed = true;
        }
        if ui
            .checkbox(&mut settings.all_interfaces, "All network interfaces")
            .on_hover_text(
                "Accept OSC from other computers on the network. OSC has no password: anyone \
                 who can reach this port can change scenes, move controls and black out the \
                 projector. Leave this off unless a phone or another machine must control the show.",
            )
            .changed()
        {
            changed = true;
        }
    });
    if remote.osc.all_interfaces && remote.osc.enabled {
        ui.label(
            egui::RichText::new("Anyone on this network can send commands to the show.")
                .small()
                .color(theme::rgb(theme.warning)),
        );
    }
    if ui
        .checkbox(&mut settings.builtin, "Built-in /strobe/... addresses")
        .on_hover_text(
            "/strobe/control/<name>, /strobe/meta/<name>, /strobe/cmd/<id>, /strobe/cue/<n>, \
             /strobe/fav/<n>, /strobe/preset/<name> and /strobe/macro/<name> work with no mapping. \
             A mapping for an address always wins.",
        )
        .changed()
    {
        changed = true;
    }
    feedback_row(ui, theme, remote, map, &mut settings, &mut changed);
    if remote.osc != remote.osc_saved {
        ui.label(
            egui::RichText::new("Running with command-line settings; they are not saved.")
                .small()
                .color(theme::rgb(theme.text_faint)),
        );
    }
    if changed {
        actions.push(Action::SetOsc(settings));
    }
}

/// OSC feedback out (D209): on or off, host and port, and where it is going.
fn feedback_row(
    ui: &mut Ui,
    theme: &Palette,
    remote: &Remote,
    map: &mut MapUi,
    settings: &mut OscSettings,
    changed: &mut bool,
) {
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut settings.feedback, "Send feedback")
            .on_hover_text(
                "Send the controls, meta, blackout and cue back out on the same /strobe/... \
                 addresses, so a TouchOSC layout or motorised faders follow the show. \
                 Only changes are sent, at most 30 times a second.",
            )
            .changed()
        {
            *changed = true;
        }
        let host = ui.add(
            egui::TextEdit::singleline(&mut map.feedback_host_text)
                .desired_width(110.0)
                .hint_text("127.0.0.1"),
        );
        if !host.has_focus() {
            map.feedback_host_text.clone_from(&settings.feedback_host);
        }
        if host.lost_focus() && map.feedback_host_text.trim() != settings.feedback_host {
            settings.feedback_host = map.feedback_host_text.trim().to_string();
            *changed = true;
        }
        ui.label(
            egui::RichText::new(":")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        let port = ui.add(
            egui::TextEdit::singleline(&mut map.feedback_port_text)
                .desired_width(56.0)
                .hint_text("9001"),
        );
        if !port.has_focus() {
            map.feedback_port_text = settings.feedback_port.to_string();
        }
        if port.lost_focus()
            && let Ok(number) = map.feedback_port_text.trim().parse::<u16>()
            && number != settings.feedback_port
        {
            settings.feedback_port = number;
            *changed = true;
        }
    });
    if let Some(why) = &remote.feedback.note {
        ui.label(
            egui::RichText::new(why)
                .small()
                .color(theme::rgb(theme.danger)),
        );
    } else if let Some(to) = remote.feedback.target() {
        let dropped = remote.feedback.dropped();
        let text = if dropped > 0 {
            format!(
                "Sending to {to} ({dropped} values dropped: the network is slower than the show)"
            )
        } else {
            format!("Sending to {to}")
        };
        ui.label(
            egui::RichText::new(text)
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
    }
}

fn midi_block(ui: &mut Ui, theme: &Palette, remote: &Remote, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let mut enabled = remote.midi.enabled;
        if ui
            .checkbox(&mut enabled, "MIDI input")
            .on_hover_text(
                "Open the MIDI input devices that are plugged in. Plugging one in later works too.",
            )
            .changed()
        {
            actions.push(Action::SetMidiEnabled(enabled));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, theme, Some(Icon::Refresh), "Refresh")
                .on_hover_text(
                    "Look at the MIDI ports again and retry a device that failed to open",
                )
                .clicked()
            {
                actions.push(Action::RescanMidi);
            }
        });
    });
    if !remote.workers_running() {
        ui.label(
            egui::RichText::new("Device workers are not running in this mode.")
                .small()
                .color(theme::rgb(theme.text_faint)),
        );
        return;
    }
    if let Some(why) = &remote.midi_note {
        ui.label(
            egui::RichText::new(format!("MIDI is not available here: {why}"))
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
    }
    if remote.midi.enabled && remote.devices.is_empty() && remote.midi_note.is_none() {
        ui.label(
            egui::RichText::new("No MIDI input devices found.")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
    }
    for device in &remote.devices {
        ui.horizontal(|ui| {
            let mut used = !device.disabled;
            if ui
                .checkbox(&mut used, &device.key)
                .on_hover_text(&device.name)
                .changed()
            {
                actions.push(Action::SetMidiDevice(device.key.clone(), used));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if device.connected {
                    widgets::status_chip(ui, Icon::Check, "Connected", theme.success);
                } else if device.disabled {
                    widgets::status_chip(ui, Icon::Close, "Off", theme.text_faint);
                } else if let Some(error) = &device.error {
                    widgets::status_chip(ui, Icon::Error, "Could not open", theme.danger)
                        .on_hover_text(error);
                } else {
                    widgets::status_chip(ui, Icon::Info, "Opening", theme.text_muted);
                }
            });
        });
    }
}

// -------------------------------------------------------------- macros

fn macros_block(
    ui: &mut Ui,
    theme: &Palette,
    remote: &mut Remote,
    map: &mut MapUi,
    eval: &Evaluated,
) {
    if remote.engine.macros.is_empty() {
        ui.label(
            egui::RichText::new(
                "No macros yet. Pin a control from the Controls pane, or use Add below. \
                 Until then the strip under the preview shows the scene's first controls.",
            )
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
        return;
    }
    let mut remove = None;
    let mut learn = None;
    let ids: Vec<u32> = remote.engine.macros.all().iter().map(|m| m.id).collect();
    for (position, id) in ids.iter().copied().enumerate() {
        let Some(found) = remote.engine.macros.get(id).cloned() else {
            continue;
        };
        let resolution = eval.macros.get(position).cloned().unwrap_or(Resolution {
            state: State::Unavailable,
            reason: String::new(),
        });
        let sources: Vec<String> = remote
            .mappings_for_macro(id)
            .map(|m| m.source.label())
            .collect();
        let selected = map.selected_macro == Some(id);
        ui.push_id(("macro-row", id), |ui| {
            theme::card(theme).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut pinned = found.pinned;
                    if ui
                        .checkbox(&mut pinned, "")
                        .on_hover_text("Pinned macros are on the strip under the preview")
                        .changed()
                    {
                        remote.engine.macros.set_pinned(id, pinned);
                    }
                    let colour = if resolution.state == State::Resolved {
                        theme.text
                    } else {
                        theme.text_muted
                    };
                    let room = (ui.available_width() - ROW_BUTTONS).max(40.0);
                    let galley = widgets::truncated(
                        ui,
                        &found.name,
                        egui::TextStyle::Body.resolve(ui.style()),
                        colour,
                        room,
                    );
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(galley.size().x, galley.size().y),
                        egui::Sense::click(),
                    );
                    ui.painter().galley(rect.min, galley, theme::rgb(colour));
                    if response.on_hover_text("Click for options").clicked() {
                        map.selected_macro = if selected { None } else { Some(id) };
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, theme, Icon::Close, "Delete this macro")
                            .clicked()
                        {
                            remove = Some(id);
                        }
                        let learning = remote.engine.is_learning_target(&Target::Macro { id });
                        if widgets::tool_button(
                            ui,
                            theme,
                            Icon::Mapping,
                            "Learn",
                            learning,
                            "Then move a fader, press a pad or send an OSC message",
                        )
                        .clicked()
                        {
                            learn = Some(id);
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    state_chip(ui, theme, &resolution);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  {}",
                            found.target.badge(),
                            found.target.label()
                        ))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                    );
                });
                let source_text = if sources.is_empty() {
                    "no MIDI or OSC source".to_string()
                } else {
                    sources.join(", ")
                };
                ui.label(
                    egui::RichText::new(source_text)
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
                if !resolution.reason.is_empty() && resolution.state != State::Resolved {
                    let sentence = if resolution.state == State::Unavailable {
                        format!("unavailable in current scene: {}", resolution.reason)
                    } else {
                        resolution.reason.clone()
                    };
                    ui.label(
                        egui::RichText::new(sentence)
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
                }
                if selected {
                    macro_options(ui, theme, remote, id);
                }
            });
        });
    }
    if let Some(id) = remove {
        remote.engine.macros.remove(id);
        if map.selected_macro == Some(id) {
            map.selected_macro = None;
        }
    }
    if let Some(id) = learn {
        let target = Target::Macro { id };
        if remote.engine.is_learning_target(&target) {
            remote.engine.cancel_learn();
        } else {
            remote.engine.arm_new(target);
        }
    }
}

fn macro_options(ui: &mut Ui, theme: &Palette, remote: &mut Remote, id: u32) {
    let Some(mut found) = remote.engine.macros.get(id).cloned() else {
        return;
    };
    let before = found.clone();
    widgets::section_rule(ui, theme);
    ui.horizontal(|ui| {
        if widgets::button(ui, theme, None, "Move earlier").clicked() {
            remote.engine.macros.move_by(id, -1);
        }
        if widgets::button(ui, theme, None, "Move later").clicked() {
            remote.engine.macros.move_by(id, 1);
        }
    });
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Name")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        ui.add(egui::TextEdit::singleline(&mut found.name).desired_width(160.0));
        ui.checkbox(&mut found.enabled, "On");
    });
    let mut aliases = found.aliases.join(", ");
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Also try")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        let edit = ui
            .add(
                egui::TextEdit::singleline(&mut aliases)
                    .desired_width(220.0)
                    .hint_text("other control names, comma separated"),
            )
            .on_hover_text(
                "If a scene has no control with this macro's name, these names are tried in \
                 order. Nothing is ever matched by guessing.",
            );
        if edit.changed() {
            found.aliases = aliases
                .split(',')
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(str::to_string)
                .collect();
        }
    });
    // Reach: the part of the control's range a fader or knob covers (D210).
    let [mut low, mut high] = found.range.unwrap_or([0.0, 1.0]);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Reach")
                .small()
                .color(theme::rgb(theme.text_muted)),
        )
        .on_hover_text(
            "The part of the control's range a mapped fader or knob covers: a fader's bottom \
             is the low end and its top the high end.",
        );
        ui.add(percent(&mut low));
        ui.label(
            egui::RichText::new("to")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        ui.add(percent(&mut high));
        if found.range.is_some() && ui.small_button("Full").clicked() {
            low = 0.0;
            high = 1.0;
        }
    });
    let range = Some([low, high]);
    // Colour: an identity for the card, from the theme's series.
    let mut colour = found.color;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Colour")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        if ui
            .selectable_label(colour.is_none(), "none")
            .on_hover_text("The theme's accent")
            .clicked()
        {
            colour = None;
        }
        for index in 0..strobe_io::macros::COLOURS {
            let fill = super::macros::macro_colour(theme, index);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::click());
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(4), theme::rgb(fill));
            if colour == Some(index) {
                ui.painter().rect_stroke(
                    rect.expand(2.0),
                    egui::CornerRadius::same(5),
                    egui::Stroke::new(1.5, theme::rgb(theme.text)),
                    egui::StrokeKind::Outside,
                );
            }
            if response.clicked() {
                colour = Some(index);
            }
        }
    });
    if found != before {
        remote.engine.macros.rename(id, &found.name);
        remote.engine.macros.set_enabled(id, found.enabled);
        remote.engine.macros.set_aliases(id, found.aliases);
    }
    if range != Some(before.range.unwrap_or([0.0, 1.0])) {
        remote.engine.macros.set_range(id, range);
    }
    if colour != before.color {
        remote.engine.macros.set_color(id, colour);
    }
}

/// A fraction edited as a percentage.
fn percent(value: &mut f32) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0.0..=1.0)
        .speed(0.005)
        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
        .custom_parser(|text| {
            text.trim()
                .trim_end_matches('%')
                .trim()
                .parse::<f64>()
                .ok()
                .map(|v| v / 100.0)
        })
}

// ------------------------------------------------------------ add form

fn add_form(
    ui: &mut Ui,
    theme: &Palette,
    remote: &mut Remote,
    map: &mut MapUi,
    targets: &Targets<'_>,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut map.search)
                .hint_text("Search mappings")
                .desired_width(ui.available_width() - 90.0),
        );
        if widgets::button(ui, theme, Some(Icon::Mapping), "Add...")
            .on_hover_text("Choose what to control, then learn a MIDI or OSC source for it")
            .clicked()
        {
            map.add.open = !map.add.open;
        }
    });
    if !map.add.open {
        ui.add_space(SPACE_XS);
        return;
    }
    theme::card(theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let form = &mut map.add;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Control")
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
            egui::ComboBox::from_id_salt("add-kind")
                .selected_text(kind_label(form.kind))
                .show_ui(ui, |ui| {
                    for kind in [
                        AddKind::SceneControl,
                        AddKind::Meta,
                        AddKind::Command,
                        AddKind::Cue,
                        AddKind::Fav,
                        AddKind::Preset,
                        AddKind::Macro,
                    ] {
                        if ui
                            .selectable_label(form.kind == kind, kind_label(kind))
                            .clicked()
                        {
                            form.kind = kind;
                            form.text.clear();
                        }
                    }
                });
        });
        match form.kind {
            AddKind::SceneControl => control_picker(ui, form, targets.scene, "control name or group/name"),
            AddKind::Meta => control_picker(ui, form, targets.meta, "meta control name"),
            AddKind::Command => {
                egui::ComboBox::from_id_salt("add-command")
                    .selected_text(if form.text.is_empty() {
                        "choose a command"
                    } else {
                        form.text.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for command in CommandId::ALL.into_iter().filter(|c| c.mappable()) {
                            if ui
                                .selectable_label(form.text == command.id(), command.name())
                                .clicked()
                            {
                                form.text = command.id().to_string();
                            }
                        }
                    });
            }
            AddKind::Cue => {
                ui.add(
                    egui::TextEdit::singleline(&mut form.text)
                        .hint_text("cue number, from 1")
                        .desired_width(160.0),
                );
            }
            AddKind::Fav => {
                egui::ComboBox::from_id_salt("add-fav")
                    .selected_text(if form.text.is_empty() {
                        "choose a slot"
                    } else {
                        form.text.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for slot in 1..=strobe_scene::preset::FAV_SLOTS {
                            let text = slot.to_string();
                            if ui.selectable_label(form.text == text, &text).clicked() {
                                form.text = text;
                            }
                        }
                    });
            }
            AddKind::Preset => {
                ui.add(
                    egui::TextEdit::singleline(&mut form.text)
                        .hint_text("preset name")
                        .desired_width(200.0),
                );
                if !targets.presets.is_empty() {
                    egui::ComboBox::from_id_salt("add-preset")
                        .selected_text("or pick one")
                        .show_ui(ui, |ui| {
                            for name in targets.presets {
                                if ui.selectable_label(form.text == *name, name).clicked() {
                                    form.text.clone_from(name);
                                }
                            }
                        });
                }
            }
            AddKind::Macro => {
                egui::ComboBox::from_id_salt("add-macro")
                    .selected_text(if form.text.is_empty() {
                        "choose a macro"
                    } else {
                        form.text.as_str()
                    })
                    .show_ui(ui, |ui| {
                        for found in remote.engine.macros.all() {
                            let label = format!("{} (#{})", found.name, found.id);
                            if ui.selectable_label(form.text == found.id.to_string(), label).clicked() {
                                form.text = found.id.to_string();
                            }
                        }
                    });
            }
        }
        let target = form_target(form, targets);
        let can_macro = matches!(
            form.kind,
            AddKind::SceneControl | AddKind::Meta | AddKind::Command | AddKind::Preset
        );
        if can_macro {
            ui.checkbox(&mut form.as_macro, "Pin it as a macro too")
                .on_hover_text("A macro is a card on the strip under the preview; it stays there when the scene changes");
        }
        ui.horizontal(|ui| {
            let ready = target.is_some();
            let learn = ui
                .add_enabled_ui(ready, |ui| {
                    widgets::tool_button(
                        ui,
                        theme,
                        Icon::Mapping,
                        "Learn a source",
                        false,
                        "Then move a fader, press a pad or send an OSC message",
                    )
                })
                .inner;
            if learn.clicked()
                && let Some(target) = target.clone()
            {
                let mut armed = target.clone();
                if can_macro && form.as_macro {
                    let name = default_macro_name(&target);
                    let id = remote.engine.macros.pin(&name, target);
                    armed = Target::Macro { id };
                }
                remote.engine.arm_new(armed);
                form.open = false;
                form.text.clear();
            }
            if can_macro && form.as_macro {
                let pin = ui
                    .add_enabled_ui(ready, |ui| widgets::button(ui, theme, None, "Pin without a source"))
                    .inner;
                if pin.clicked()
                    && let Some(target) = target
                {
                    let name = default_macro_name(&target);
                    remote.engine.macros.pin(&name, target);
                    form.open = false;
                    form.text.clear();
                }
            }
            if widgets::button(ui, theme, None, "Close").clicked() {
                form.open = false;
            }
        });
    });
    ui.add_space(SPACE_SM);
}

fn kind_label(kind: AddKind) -> &'static str {
    match kind {
        AddKind::SceneControl => "Scene control",
        AddKind::Meta => "Meta control",
        AddKind::Command => "Command",
        AddKind::Cue => "Playlist cue",
        AddKind::Fav => "Fav slot",
        AddKind::Preset => "Preset",
        AddKind::Macro => "Macro",
    }
}

/// A text field for a control name, with a picker over the controls of the
/// bank that is playing now. Typing a name the scene lacks is allowed: the
/// mapping waits as unavailable until a scene that has it plays.
fn control_picker(
    ui: &mut Ui,
    form: &mut super::model::AddForm,
    bank: Option<&ControlBank>,
    hint: &str,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut form.text)
                .hint_text(hint)
                .desired_width(180.0),
        );
        if let Some(bank) = bank
            && !bank.defs().is_empty()
        {
            egui::ComboBox::from_id_salt("add-control")
                .selected_text("or pick one")
                .show_ui(ui, |ui| {
                    for def in bank.defs() {
                        if ui
                            .selectable_label(form.text == def.name, &def.name)
                            .clicked()
                        {
                            form.text.clone_from(&def.name);
                            form.component = 0;
                        }
                    }
                });
        }
    });
    let multi = bank.and_then(|bank| {
        let index = strobe_io::target::find_control(bank, form.text.trim(), &[])?;
        let def = bank.defs().get(index)?;
        matches!(def.kind, ControlKind::Xy(_) | ControlKind::Color { .. }).then_some(def.dims())
    });
    if let Some(dims) = multi {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Component")
                    .small()
                    .color(theme::rgb(theme::palette().text_muted)),
            );
            for component in 0..dims {
                let name = ["x or red", "y or green", "z or blue"][component.min(2)];
                if ui
                    .selectable_label(usize::from(form.component) == component, name)
                    .clicked()
                {
                    form.component = component as u8;
                }
            }
        });
    }
}

/// The target the form describes, if it is complete.
fn form_target(form: &super::model::AddForm, targets: &Targets<'_>) -> Option<Target> {
    let text = form.text.trim();
    if text.is_empty() {
        return None;
    }
    let component = |bank: Option<&ControlBank>| {
        let bank = bank?;
        let index = strobe_io::target::find_control(bank, text, &[])?;
        let def = bank.defs().get(index)?;
        matches!(def.kind, ControlKind::Xy(_) | ControlKind::Color { .. }).then_some(form.component)
    };
    match form.kind {
        AddKind::SceneControl => Some(Target::Control {
            name: text.to_string(),
            component: component(targets.scene),
        }),
        AddKind::Meta => Some(Target::Meta {
            name: text.to_string(),
            component: component(targets.meta),
        }),
        AddKind::Command => Some(Target::command(text)),
        AddKind::Cue => text
            .parse()
            .ok()
            .filter(|n| *n >= 1)
            .map(|number| Target::Cue { number }),
        AddKind::Fav => text.parse().ok().map(|slot| Target::FavSlot { slot }),
        AddKind::Preset => Some(Target::Preset {
            name: text.to_string(),
        }),
        AddKind::Macro => text.parse().ok().map(|id| Target::Macro { id }),
    }
}

fn default_macro_name(target: &Target) -> String {
    match target {
        Target::Control { name, .. } | Target::Meta { name, .. } | Target::Preset { name } => {
            name.clone()
        }
        Target::Command { id } => {
            CommandId::from_id(id).map_or_else(|| id.clone(), |c| c.name().to_string())
        }
        Target::Cue { number } => format!("Cue {number}"),
        Target::FavSlot { slot } => format!("Fav {slot}"),
        Target::Macro { id } => format!("Macro {id}"),
    }
}

// ------------------------------------------------------------ mappings

fn mappings_block(
    ui: &mut Ui,
    theme: &Palette,
    remote: &mut Remote,
    map: &mut MapUi,
    eval: &Evaluated,
) {
    if remote.engine.mappings.is_empty() {
        widgets::empty_state(
            ui,
            theme,
            Icon::Mapping,
            "No mappings yet",
            "Press Add..., choose what to control, then move a fader or press a pad. \
             OSC works without a mapping too: send /strobe/control/<name> a number from 0 to 1.",
        );
        return;
    }
    let needle = map.search.trim().to_lowercase();
    let known_devices: Vec<String> = remote.devices.iter().map(|d| d.key.clone()).collect();
    let mut remove = None;
    let mut learn = None;
    for index in 0..remote.engine.mappings.len() {
        let Some(row) = remote.engine.mappings.get(index) else {
            continue;
        };
        if !needle.is_empty() && !haystack(row).contains(&needle) {
            continue;
        }
        let resolution = eval.mappings.get(index).cloned().unwrap_or(Resolution {
            state: State::Unavailable,
            reason: String::new(),
        });
        let selected = map.selected == Some(index);
        let relearning = remote.engine.is_relearning(index);
        let just_learned = remote.learned == Some(index);
        ui.push_id(("mapping-row", index), |ui| {
            theme::card(theme).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let mapping = &mut remote.engine.mappings[index];
                ui.horizontal(|ui| {
                    let title = source_title(&mapping.source);
                    let colour = if mapping.enabled {
                        theme.text
                    } else {
                        theme.text_faint
                    };
                    let room = (ui.available_width() - ROW_BUTTONS).max(40.0);
                    let galley = widgets::truncated(
                        ui,
                        &title,
                        egui::TextStyle::Body.resolve(ui.style()),
                        colour,
                        room,
                    );
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(galley.size().x, galley.size().y),
                        egui::Sense::click(),
                    );
                    ui.painter().galley(rect.min, galley, theme::rgb(colour));
                    if response.on_hover_text(&title).clicked() {
                        map.selected = if selected { None } else { Some(index) };
                    }
                    if just_learned {
                        widgets::chip(ui, "NEW", theme.accent_soft, theme.accent);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button(ui, theme, Icon::Close, "Delete this mapping")
                            .clicked()
                        {
                            remove = Some(index);
                        }
                        if widgets::tool_button(
                            ui,
                            theme,
                            Icon::Mapping,
                            "Learn",
                            relearning,
                            "Bind a different MIDI or OSC source to this target",
                        )
                        .clicked()
                        {
                            learn = Some(index);
                        }
                        ui.checkbox(&mut mapping.enabled, "")
                            .on_hover_text("Switch this mapping off without deleting it");
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    state_chip(ui, theme, &resolution);
                    ui.label(
                        egui::RichText::new(format!(
                            "-> {}  {}",
                            mapping.target.badge(),
                            mapping.target.label()
                        ))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                    );
                });
                if matches!(resolution.state, State::Unavailable | State::Broken)
                    && !resolution.reason.is_empty()
                {
                    ui.label(
                        egui::RichText::new(&resolution.reason)
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
                }
                if selected {
                    mapping_options(ui, theme, mapping, &known_devices);
                }
            });
        });
    }
    if let Some(index) = remove {
        remote.engine.mappings.remove(index);
        remote.engine.cancel_learn();
        map.selected = None;
    }
    if let Some(index) = learn {
        if remote.engine.is_relearning(index) {
            remote.engine.cancel_learn();
        } else {
            remote.engine.arm_relearn(index);
        }
    }
}

/// What search looks through.
fn haystack(mapping: &Mapping) -> String {
    format!(
        "{} {} {} {}",
        mapping.source.label(),
        mapping.source.device().unwrap_or(""),
        mapping.target.label(),
        mapping.name
    )
    .to_lowercase()
}

/// `CC 21 ch 1 [nanoKONTROL2]`.
fn source_title(source: &Source) -> String {
    match source.device() {
        Some(device) => format!("{}  [{device}]", source.label()),
        None if source.is_osc() => source.label(),
        None => format!("{}  [any device]", source.label()),
    }
}

fn mapping_options(ui: &mut Ui, theme: &Palette, mapping: &mut Mapping, devices: &[String]) {
    widgets::section_rule(ui, theme);
    ui.add_space(SPACE_XS);
    if !mapping.source.is_osc() {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Device")
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
            let current = mapping.source.device().unwrap_or("Any device").to_string();
            egui::ComboBox::from_id_salt("mapping-device")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(mapping.source.device().is_none(), "Any device")
                        .clicked()
                    {
                        mapping.source = mapping.source.for_any_device();
                    }
                    let own = mapping.source.device().map(str::to_string);
                    let mut keys: Vec<String> = devices.to_vec();
                    if let Some(own) = own
                        && !keys.contains(&own)
                    {
                        keys.push(own);
                    }
                    for key in keys {
                        let current = mapping.source.device() == Some(key.as_str());
                        if ui.selectable_label(current, &key).clicked() {
                            mapping.source = mapping.source.for_device(&key);
                        }
                    }
                });
        });
    }
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Range")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        ui.add(
            egui::DragValue::new(&mut mapping.range[0])
                .range(0.0..=1.0)
                .speed(0.005)
                .fixed_decimals(2),
        )
        .on_hover_text("The lowest part of the control's range the source reaches");
        ui.label("to");
        ui.add(
            egui::DragValue::new(&mut mapping.range[1])
                .range(0.0..=1.0)
                .speed(0.005)
                .fixed_decimals(2),
        )
        .on_hover_text("The highest part. Higher low than high reverses it.");
        ui.checkbox(&mut mapping.invert, "Invert");
    });
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Curve")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        egui::ComboBox::from_id_salt("mapping-curve")
            .selected_text(mapping.curve.label())
            .show_ui(ui, |ui| {
                for curve in Curve::ALL {
                    if ui
                        .selectable_label(mapping.curve == curve, curve.label())
                        .clicked()
                    {
                        mapping.curve = curve;
                    }
                }
            });
        ui.label(
            egui::RichText::new("Fader")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        egui::ComboBox::from_id_salt("mapping-takeover")
            .selected_text(mapping.takeover.label())
            .show_ui(ui, |ui| {
                for mode in [Takeover::Pickup, Takeover::Jump] {
                    if ui
                        .selectable_label(mapping.takeover == mode, mode.label())
                        .on_hover_text(match mode {
                            Takeover::Pickup => {
                                "Ignore the fader until it reaches the control's value, so nothing jumps"
                            }
                            Takeover::Jump => "Follow the fader at once, even if that jumps",
                        })
                        .clicked()
                    {
                        mapping.takeover = mode;
                    }
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Button")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        egui::ComboBox::from_id_salt("mapping-button")
            .selected_text(mapping.button.label())
            .show_ui(ui, |ui| {
                for mode in [ButtonMode::Toggle, ButtonMode::Momentary] {
                    if ui
                        .selectable_label(mapping.button == mode, mode.label())
                        .on_hover_text(match mode {
                            ButtonMode::Toggle => "Each press flips the control",
                            ButtonMode::Momentary => {
                                "The control follows the button while it is held"
                            }
                        })
                        .clicked()
                    {
                        mapping.button = mode;
                    }
                }
            });
        ui.label(
            egui::RichText::new("Press at")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        ui.add(
            egui::DragValue::new(&mut mapping.threshold)
                .range(0.05..=0.95)
                .speed(0.005)
                .fixed_decimals(2),
        )
        .on_hover_text(
            "Where a fader counts as pressed when it drives a button, a command or a toggle",
        );
    });
    ui.add_space(SPACE_XS);
}

// ------------------------------------------------------------- activity

fn activity_block(ui: &mut Ui, theme: &Palette, remote: &mut Remote) {
    if remote.activity.is_empty() {
        ui.label(
            egui::RichText::new(
                "Nothing heard yet. Move a fader, press a pad, or send an OSC message \
                 and it appears here with what it did.",
            )
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
        return;
    }
    let mut clear = false;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{} dropped", remote.dropped))
                .small()
                .color(theme::rgb(theme.text_faint)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            clear = widgets::button(ui, theme, None, "Clear").clicked();
        });
    });
    for line in remote.activity.lines().rev().take(ACTIVITY_LINES) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_MD;
            ui.label(
                egui::RichText::new(format!("{:7.1}s", line.at))
                    .monospace()
                    .size(10.5)
                    .color(theme::rgb(theme.text_faint)),
            );
            ui.label(
                egui::RichText::new(&line.text)
                    .monospace()
                    .size(10.5)
                    .color(theme::rgb(theme.text)),
            );
            if !line.result.is_empty() {
                let colour = if line.result.starts_with("unmapped")
                    || line.result.starts_with("unavailable")
                {
                    theme.text_faint
                } else {
                    theme.success
                };
                ui.label(
                    egui::RichText::new(&line.result)
                        .size(10.5)
                        .color(theme::rgb(colour)),
                );
            }
            if line.count > 1 {
                ui.label(
                    egui::RichText::new(format!("x{}", line.count))
                        .size(10.5)
                        .color(theme::rgb(theme.text_muted)),
                );
            }
        });
    }
    if clear {
        remote.activity.clear();
    }
    let _ = icons::INLINE;
}

/// A macro with a default name, for tests and the add form.
pub fn macro_for(target: Target) -> Macro {
    Macro::new(&default_macro_name(&target), target)
}
