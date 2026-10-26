// ============================================================================
// Module:       strobe_app::ui::map
// Description:  The MAP pane: OSC and MIDI connections, the activity monitor,
//               the Performance Macro set, and the mapping list with learn,
//               in the form grammar.
//
// Dependencies: egui; strobe-io (Engine, Mapping, Target, State); super::{
//               form, model, theme, widgets, icon}, crate::{remote, commands}
// ============================================================================

//! The Mappings pane (UI_DESIGN section 15, D177), in `super::form`'s
//! grammar (UI review step 2, D467).
//!
//! Learn first, then adjust:
//!
//! ```text
//! Mappings
//! Drive scenes from MIDI controllers and OSC ...
//! (a banner while learn is armed, or when a source is already mapped)
//! Sources      [ Add a mapping                               [Add...]   ]
//!              [ CC 21 ch 1 [nanoKONTROL2] (Resolved) [>] (on) [Learn][x]]
//! Macros       [ Speed  (Resolved)              [>] [Pin] [Learn] [x]   ]
//!              [   SCENE speed . CC 21 ch 1                              ]
//! Activity     [ 12.3s  CC 21 = 64  ch 1  [nanoKONTROL2]  applied  x37  ]
//! OSC          [ OSC input  (Listening on 127.0.0.1:9000)          (on) ]
//!              [ Port                                          [9000]   ]
//! MIDI         [ MIDI input ...                                    (on) ]
//!              [ nanoKONTROL2  (Connected)                         (on) ]
//! ```
//!
//! What a show reaches for comes first; the connections, set up once, come
//! last.
//!
//! Every row says its state in a word and an icon, never colour alone
//! (resolved, unavailable, broken, disabled, learning, conflict). A mapping
//! to a control the playing scene does not have is **unavailable**, kept,
//! and says which control it is waiting for. A macro's or a mapping's
//! options open under its row, in an inset card, from its disclosure
//! button.
//!
//! Learn is the primary path: choose the target (*Add...*, or *Learn* on an
//! existing row), then move a fader, press a pad or send an OSC message. The
//! pane edits mappings and macros in place (they are the operator's data and
//! the mapper's own); everything that touches a worker, a file or the
//! scene goes back to the app as an [`Action`].

use super::form::{self, Row};
use super::model::{Action, AddKind, MapUi, Model};
use super::theme::{self, SPACE_MD, SPACE_XS};
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

/// Activity lines drawn (the log keeps more).
const ACTIVITY_LINES: usize = 8;

/// The width of a short field: a port, a cue number.
const SHORT_FIELD: f32 = 80.0;

/// The width of a name or address field.
const FIELD: f32 = 180.0;

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

/// A row's words for a state: its title, then the state as a chip.
fn with_state<'a>(row: Row<'a>, theme: &Palette, resolution: &'a Resolution) -> Row<'a> {
    let (icon, colour) = state_look(theme, resolution.state);
    row.status(icon, resolution.state.label(), colour)
}

/// The disclosure button that opens or closes a row's options.
fn disclosure(ui: &mut Ui, theme: &Palette, open: bool) -> bool {
    let (icon, tip) = if open {
        (Icon::ChevronDown, "Hide the options")
    } else {
        (Icon::ChevronRight, "Show the options")
    };
    widgets::icon_button(ui, theme, icon, tip).clicked()
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
    let bindings = panel.bindings;
    egui::ScrollArea::vertical()
        .id_salt("map-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            form::page(
                ui,
                theme,
                Pane::Map.title(),
                Some(
                    "Drive scenes from MIDI controllers and OSC: choose what to control, then \
                     move a fader, press a pad or send a message.",
                ),
                |ui| {
                    // What a show uses first, what is set up once last.
                    learn_banner(ui, theme, remote);
                    let targets = Targets {
                        scene,
                        meta,
                        presets,
                    };
                    mappings_group(ui, theme, remote, map, &eval, &targets);
                    macros_group(ui, theme, remote, map, &eval, bindings);
                    activity_group(ui, theme, remote);
                    osc_group(ui, theme, remote, map, actions);
                    midi_group(ui, theme, remote, actions);
                    ui.add_space(theme::SPACE_LG);
                },
            );
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
        ui.add_space(SPACE_MD);
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
    ui.add_space(SPACE_MD);
    widgets::banner(
        ui,
        theme,
        Tone::Info,
        &format!("Learning {what}: move a fader, press a pad or send an OSC message."),
        |ui| {
            cancel = widgets::button(ui, theme, None, "Cancel").clicked();
        },
    );
    if cancel {
        engine.cancel_learn();
    }
}

// ---------------------------------------------------------- connections

fn osc_group(
    ui: &mut Ui,
    theme: &Palette,
    remote: &Remote,
    map: &mut MapUi,
    actions: &mut Vec<Action>,
) {
    let mut settings: OscSettings = remote.osc.clone();
    let mut changed = false;
    form::group(ui, theme, "OSC", |group| {
        let listening;
        let (icon, word, colour) = match &remote.osc_state {
            OscState::Listening(addr) => {
                listening = format!("Listening on {addr}");
                (Icon::Check, listening.as_str(), theme.success)
            }
            OscState::Off if settings.enabled => (Icon::Info, "Starting", theme.text_muted),
            OscState::Off => (Icon::Info, "Off", theme.text_muted),
            OscState::Error(_) => (Icon::Error, "Cannot listen", theme.danger),
        };
        let mut row = Row::new("OSC input")
            .status(icon, word, colour)
            .description(
                "Listen for OSC messages over UDP. Only this computer can send to it unless every \
             network interface is allowed.",
            );
        if let OscState::Error(why) = &remote.osc_state {
            row = row.note(why, theme.danger);
        }
        changed |= group.toggle(row, &mut settings.enabled).changed();
        group.row(Row::new("Port"), |ui| {
            let edit = form::field(ui, theme, &mut map.port_text, "9000", SHORT_FIELD);
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
        });
        let mut row = Row::new("All network interfaces").description(
            "Accept OSC from other computers. OSC has no password: anyone who can reach this \
             port can change scenes, move controls and black out the projector.",
        );
        if remote.osc.all_interfaces && remote.osc.enabled {
            row = row.note(
                "Anyone on this network can send commands to the show.",
                theme.warning,
            );
        }
        changed |= group.toggle(row, &mut settings.all_interfaces).changed();
        changed |= group
            .toggle(
                Row::new("Built-in /strobe/... addresses").description(
                    "/strobe/control/<name>, /strobe/meta/<name>, /strobe/cmd/<id>, \
                     /strobe/cue/<n>, /strobe/fav/<n>, /strobe/preset/<name> and \
                     /strobe/macro/<name> work with no mapping. A mapping for an address \
                     always wins.",
                ),
                &mut settings.builtin,
            )
            .changed();
        feedback_rows(group, theme, remote, map, &mut settings, &mut changed);
        if remote.osc != remote.osc_saved {
            group.block(|ui| {
                ui.label(
                    egui::RichText::new("Running with command-line settings; they are not saved.")
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                );
            });
        }
    });
    if changed {
        actions.push(Action::SetOsc(settings));
    }
}

/// OSC feedback out (D209): on or off, host and port, and where it is going.
fn feedback_rows(
    group: &mut form::Group<'_>,
    theme: &Palette,
    remote: &Remote,
    map: &mut MapUi,
    settings: &mut OscSettings,
    changed: &mut bool,
) {
    let status = if let Some(to) = remote.feedback.target() {
        let dropped = remote.feedback.dropped();
        if dropped > 0 {
            format!(
                "Sending to {to} ({dropped} values dropped: the network is slower than the show)."
            )
        } else {
            format!("Sending to {to}.")
        }
    } else {
        String::new()
    };
    let mut row = Row::new("Send feedback").description(
        "Send the controls, meta, blackout and cue back out on the same /strobe/... \
         addresses, so a TouchOSC layout or motorised faders follow the show. Only changes \
         are sent, at most 30 times a second.",
    );
    if let Some(why) = &remote.feedback.note {
        row = row.note(why, theme.danger);
    } else if !status.is_empty() {
        row = row.note(&status, theme.text_muted);
    }
    *changed |= group.toggle(row, &mut settings.feedback).changed();
    group.row(
        Row::new("Feedback address").description("Where feedback goes: a host and its port."),
        |ui| {
            // Right to left: the port first, then the colon, then the host.
            let port = form::field(ui, theme, &mut map.feedback_port_text, "9001", SHORT_FIELD);
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
            ui.label(egui::RichText::new(":").color(theme::rgb(theme.text_muted)));
            let host = form::field(ui, theme, &mut map.feedback_host_text, "127.0.0.1", FIELD);
            if !host.has_focus() {
                map.feedback_host_text.clone_from(&settings.feedback_host);
            }
            if host.lost_focus() && map.feedback_host_text.trim() != settings.feedback_host {
                settings.feedback_host = map.feedback_host_text.trim().to_string();
                *changed = true;
            }
        },
    );
}

fn midi_group(ui: &mut Ui, theme: &Palette, remote: &Remote, actions: &mut Vec<Action>) {
    form::group(ui, theme, "MIDI", |group| {
        let mut enabled = remote.midi.enabled;
        let mut row = Row::new("MIDI input").description(
            "Open the MIDI input devices that are plugged in. Plugging one in later works too.",
        );
        let unavailable;
        if !remote.workers_running() {
            row = row.note(
                "MIDI devices are not opened in this mode (a screenshot or test run).",
                theme.text_muted,
            );
        } else if let Some(why) = &remote.midi_note {
            unavailable = format!("MIDI is not available here: {why}");
            row = row.note(&unavailable, theme.warning);
        }
        if group.toggle(row, &mut enabled).changed() {
            actions.push(Action::SetMidiEnabled(enabled));
        }
        if !remote.workers_running() {
            return;
        }
        let found = match remote.devices.len() {
            0 if remote.midi.enabled && remote.midi_note.is_none() => {
                "No MIDI input devices found.".to_string()
            }
            0 => String::new(),
            1 => "1 device found.".to_string(),
            count => format!("{count} devices found."),
        };
        let mut row = Row::new("Devices");
        if !found.is_empty() {
            row = row.description(&found);
        }
        group.row(row, |ui| {
            if widgets::button(ui, theme, Some(Icon::Refresh), "Rescan")
                .on_hover_text(
                    "Look at the MIDI ports again and retry a device that failed to open",
                )
                .clicked()
            {
                actions.push(Action::RescanMidi);
            }
        });
        for device in &remote.devices {
            let (icon, word, colour) = if device.connected {
                (Icon::Check, "Connected", theme.success)
            } else if device.disabled {
                (Icon::Close, "Off", theme.text_faint)
            } else if device.error.is_some() {
                (Icon::Error, "Could not open", theme.danger)
            } else {
                (Icon::Info, "Opening", theme.text_muted)
            };
            let mut row = Row::new(&device.key).status(icon, word, colour);
            if let Some(error) = &device.error {
                row = row.note(error, theme.danger);
            } else if device.name != device.key {
                row = row.description(&device.name);
            }
            let mut used = !device.disabled;
            if group.toggle(row, &mut used).changed() {
                actions.push(Action::SetMidiDevice(device.key.clone(), used));
            }
        }
        let dropped;
        let mut feedback = remote.midi.feedback;
        let mut row = Row::new("Send MIDI feedback")
            .enabled(remote.midi.enabled)
            .description(
                "Send each mapped control back to its device, so motorised faders, LED rings \
                 and pad lights show the show. Only for mappings on a named device.",
            );
        if remote.midi_feedback.running() && remote.midi_feedback.dropped() > 0 {
            dropped = format!(
                "{} feedback message(s) dropped: the devices did not keep up.",
                remote.midi_feedback.dropped()
            );
            row = row.note(&dropped, theme.text_muted);
        }
        if group.toggle(row, &mut feedback).changed() {
            actions.push(Action::SetMidiFeedback(feedback));
        }
    });
}

// ------------------------------------------------------------- activity

fn activity_group(ui: &mut Ui, theme: &Palette, remote: &mut Remote) {
    let mut clear = false;
    form::group(ui, theme, "Activity", |group| {
        if remote.activity.is_empty() {
            group.block(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Nothing heard yet. Move a fader, press a pad, or send an OSC \
                             message and it appears here with what it did.",
                        )
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                    )
                    .wrap(),
                );
            });
            return;
        }
        let dropped = format!("{} dropped.", remote.dropped);
        let mut row = Row::new("Recent messages");
        if remote.dropped > 0 {
            row = row.description(&dropped);
        }
        group.row(row, |ui| {
            clear = widgets::button(ui, theme, None, "Clear").clicked();
        });
        group.block(|ui| {
            for line in remote.activity.lines().rev().take(ACTIVITY_LINES) {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_MD;
                    ui.label(
                        egui::RichText::new(format!("{:7.1}s", line.at))
                            .monospace()
                            .color(theme::rgb(theme.text_faint)),
                    );
                    ui.label(
                        egui::RichText::new(&line.text)
                            .monospace()
                            .color(theme::rgb(theme.text)),
                    );
                    if !line.result.is_empty() {
                        let colour = if line.result.starts_with("unmapped")
                            || line.result.starts_with("unavailable")
                        {
                            theme.text_muted
                        } else {
                            theme.success
                        };
                        ui.label(
                            egui::RichText::new(&line.result)
                                .font(theme::description_font())
                                .color(theme::rgb(colour)),
                        );
                    }
                    if line.count > 1 {
                        ui.label(
                            egui::RichText::new(format!("x{}", line.count))
                                .font(theme::description_font())
                                .color(theme::rgb(theme.text_muted)),
                        );
                    }
                });
            }
        });
    });
    if clear {
        remote.activity.clear();
    }
}

// -------------------------------------------------------------- macros

fn macros_group(
    ui: &mut Ui,
    theme: &Palette,
    remote: &mut Remote,
    map: &mut MapUi,
    eval: &Evaluated,
    bindings: &crate::commands::Bindings,
) {
    let mut remove = None;
    let mut learn = None;
    form::group(ui, theme, "Macros", |group| {
        if remote.engine.macros.is_empty() {
            group.block(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "No macros yet. Pin a control from the Controls pane, or use Add \
                             below. Until then the strip under the preview shows the scene's \
                             first controls.",
                        )
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                    )
                    .wrap(),
                );
            });
            return;
        }
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
            let sources = if sources.is_empty() {
                "no MIDI or OSC source".to_string()
            } else {
                sources.join(", ")
            };
            let what = format!(
                "{} {}  ·  {sources}",
                found.target.badge(),
                found.target.label()
            );
            let reason = (!resolution.reason.is_empty() && resolution.state != State::Resolved)
                .then(|| {
                    if resolution.state == State::Unavailable {
                        format!("Unavailable in this scene: {}", resolution.reason)
                    } else {
                        resolution.reason.clone()
                    }
                });
            let mut row = with_state(Row::new(&found.name), theme, &resolution).description(&what);
            if let Some(reason) = &reason {
                row = row.note(reason, theme.text_muted);
            }
            let selected = map.selected_macro == Some(id);
            let learning = remote.engine.is_learning_target(&Target::Macro { id });
            let mut pinned = found.pinned;
            group.row(row, |ui| {
                if widgets::icon_button(ui, theme, Icon::Close, "Delete this macro").clicked() {
                    remove = Some(id);
                }
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
                if widgets::tool_button(
                    ui,
                    theme,
                    Icon::Star,
                    if pinned { "Pinned" } else { "Pin" },
                    pinned,
                    "Pinned macros are on the strip under the preview",
                )
                .clicked()
                {
                    pinned = !pinned;
                }
                if disclosure(ui, theme, selected) {
                    map.selected_macro = if selected { None } else { Some(id) };
                }
            });
            if pinned != found.pinned {
                remote.engine.macros.set_pinned(id, pinned);
            }
            if selected {
                group.block(|ui| {
                    ui.push_id(("macro-options", id), |ui| {
                        form::inset(ui, theme, |options| {
                            macro_options(options, theme, remote, id);
                            macro_key_row(options, theme, remote, map, id, bindings);
                        });
                    });
                });
            }
        }
    });
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

/// A macro's keyboard key (D228): shown, captured from the next key
/// pressed, or cleared. A key a command holds is refused, so a shortcut
/// never stops working because a macro took it.
fn macro_key_row(
    group: &mut form::Group<'_>,
    theme: &Palette,
    remote: &mut Remote,
    map: &mut MapUi,
    id: u32,
    bindings: &crate::commands::Bindings,
) {
    use super::keys::{Capture, captured};
    use crate::commands::{Chord, CommandId};
    let current = remote.engine.macros.get(id).and_then(|m| m.key.clone());
    let capturing = map.key_capture == Some(id);
    if capturing {
        match captured(group.ui().ctx()) {
            Capture::Waiting => {}
            Capture::Cancel => map.key_capture = None,
            Capture::Chord(chord) => {
                map.key_capture = None;
                let holder = CommandId::ALL
                    .into_iter()
                    .find(|command| bindings.chord(*command) == Some(chord));
                match holder {
                    Some(command) => {
                        map.key_problem = Some(format!(
                            "{} is the shortcut for {}; choose another key, or change that \
                             shortcut in Settings.",
                            chord.label(),
                            command.name()
                        ));
                    }
                    None => {
                        map.key_problem = None;
                        remote.engine.macros.set_key(id, Some(chord.label()));
                    }
                }
            }
        }
    }
    let capturing = map.key_capture == Some(id);
    let mut row = Row::new("Key").description(
        "Held, the macro goes to the top of its reach; let go, to the bottom. Through its \
         glide, if it has one.",
    );
    let problem = map.key_problem.clone();
    if let Some(problem) = &problem {
        row = row.note(problem, theme.warning);
    }
    let mut set = false;
    let mut clear = false;
    group.row(row, |ui| {
        if !capturing {
            if current.is_some() && widgets::button(ui, theme, None, "Clear").clicked() {
                clear = true;
            }
            if widgets::button(ui, theme, None, "Set").clicked() {
                set = true;
            }
        }
        let (shown, colour) = if capturing {
            ("press a key (Esc cancels)".to_string(), theme.warning)
        } else {
            (
                current
                    .as_deref()
                    .and_then(Chord::parse)
                    .map_or_else(|| "none".to_string(), Chord::label),
                theme.accent,
            )
        };
        widgets::keycap(ui, theme, &shown, colour);
    });
    if set {
        map.key_capture = Some(id);
        map.key_problem = None;
    }
    if clear {
        remote.engine.macros.set_key(id, None);
    }
}

fn macro_options(group: &mut form::Group<'_>, theme: &Palette, remote: &mut Remote, id: u32) {
    let Some(mut found) = remote.engine.macros.get(id).cloned() else {
        return;
    };
    let before = found.clone();
    group.row(Row::new("Name"), |ui| {
        form::field(ui, theme, &mut found.name, "Macro name", FIELD);
    });
    group.toggle(
        Row::new("On").description("Off keeps the macro and its mappings, and does nothing."),
        &mut found.enabled,
    );
    let mut aliases = found.aliases.join(", ");
    group.row(
        Row::new("Also try").description(
            "If a scene has no control with this macro's name, these names are tried in \
             order. Nothing is ever matched by guessing.",
        ),
        |ui| {
            if form::field(
                ui,
                theme,
                &mut aliases,
                "other names, comma separated",
                FIELD,
            )
            .changed()
            {
                found.aliases = aliases
                    .split(',')
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .map(str::to_string)
                    .collect();
            }
        },
    );
    // Reach: the part of the control's range a fader or knob covers (D210).
    let [mut low, mut high] = found.range.unwrap_or([0.0, 1.0]);
    group.row(
        Row::new("Reach").description(
            "The part of the control's range a mapped fader or knob covers: its bottom is the \
             low end and its top the high end.",
        ),
        |ui| {
            // Right to left: Full, the high end, "to", the low end.
            if found.range.is_some() && widgets::button(ui, theme, None, "Full").clicked() {
                low = 0.0;
                high = 1.0;
            }
            ui.add(percent(&mut high));
            ui.label(egui::RichText::new("to").color(theme::rgb(theme.text_muted)));
            ui.add(percent(&mut low));
        },
    );
    let range = Some([low, high]);
    // Glide: how long the control takes to follow (D227).
    let mut glide = found.glide.unwrap_or(0.0);
    group.row(
        Row::new("Glide").description(
            "How long the control takes to follow a fader, knob or key, so a jumpy fader moves \
             the picture smoothly. 0 follows at once.",
        ),
        |ui| {
            ui.add(
                egui::DragValue::new(&mut glide)
                    .speed(0.01)
                    .range(0.0..=strobe_io::macros::MAX_GLIDE)
                    .max_decimals(2)
                    .suffix(" s"),
            );
        },
    );
    let glide = (glide > 0.0).then_some(glide);
    // Colour: an identity for the card, from the theme's series.
    let mut colour = found.color;
    group.row(
        Row::new("Colour").description("The card's colour on the strip under the preview."),
        |ui| {
            // Right to left: the swatches last first, then "none".
            for index in (0..strobe_io::macros::COLOURS).rev() {
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
            if ui
                .selectable_label(colour.is_none(), "none")
                .on_hover_text("The theme's accent")
                .clicked()
            {
                colour = None;
            }
        },
    );
    let mut step = 0;
    group.row(
        Row::new("Order").description("Where the card sits on the strip."),
        |ui| {
            if widgets::button(ui, theme, None, "Move later").clicked() {
                step = 1;
            }
            if widgets::button(ui, theme, None, "Move earlier").clicked() {
                step = -1;
            }
        },
    );
    if step != 0 {
        remote.engine.macros.move_by(id, step);
    }
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
    if glide != before.glide {
        remote.engine.macros.set_glide(id, glide);
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

// ------------------------------------------------------------ mappings

fn mappings_group(
    ui: &mut Ui,
    theme: &Palette,
    remote: &mut Remote,
    map: &mut MapUi,
    eval: &Evaluated,
    targets: &Targets<'_>,
) {
    let mut remove = None;
    let mut learn = None;
    // "Sources": the page is already titled Mappings, and each row is
    // named for the fader, pad or address it answers.
    form::group(ui, theme, "Sources", |group| {
        add_form(group, theme, remote, map, targets);
        if remote.engine.mappings.is_empty() {
            group.block(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "No mappings yet. OSC works without one too: send \
                             /strobe/control/<name> a number from 0 to 1.",
                        )
                        .font(theme::description_font())
                        .color(theme::rgb(theme.text_muted)),
                    )
                    .wrap(),
                );
            });
            return;
        }
        group.block(|ui| {
            let width = ui.available_width();
            form::field(ui, theme, &mut map.search, "Search mappings", width);
        });
        let needle = map.search.trim().to_lowercase();
        let known_devices: Vec<String> = remote.devices.iter().map(|d| d.key.clone()).collect();
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
            let mapping = &mut remote.engine.mappings[index];
            let title = source_title(&mapping.source);
            let what = format!("-> {}  {}", mapping.target.badge(), mapping.target.label());
            let mut words = with_state(Row::new(&title), theme, &resolution).description(&what);
            if just_learned {
                words = words.note("Just learned.", theme.accent);
            } else if matches!(resolution.state, State::Unavailable | State::Broken)
                && !resolution.reason.is_empty()
            {
                words = words.note(&resolution.reason, theme.text_muted);
            }
            group.row(words, |ui| {
                if widgets::icon_button(ui, theme, Icon::Close, "Delete this mapping").clicked() {
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
                form::toggle(ui, theme, &mut mapping.enabled)
                    .on_hover_text("Switch this mapping off without deleting it");
                if disclosure(ui, theme, selected) {
                    map.selected = if selected { None } else { Some(index) };
                }
            });
            if selected {
                group.block(|ui| {
                    ui.push_id(("mapping-options", index), |ui| {
                        form::inset(ui, theme, |options| {
                            mapping_options(options, theme, mapping, &known_devices);
                        });
                    });
                });
            }
        }
    });
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

fn mapping_options(
    group: &mut form::Group<'_>,
    theme: &Palette,
    mapping: &mut Mapping,
    devices: &[String],
) {
    if !mapping.source.is_osc() {
        group.row(
            Row::new("Device")
                .description("Answer this device only, or the same message from any."),
            |ui| {
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
            },
        );
    }
    group.row(
        Row::new("Range").description(
            "The part of the control's range the source reaches. A higher low than high \
             reverses it.",
        ),
        |ui| {
            // Right to left: the high end, "to", the low end.
            ui.add(
                egui::DragValue::new(&mut mapping.range[1])
                    .range(0.0..=1.0)
                    .speed(0.005)
                    .fixed_decimals(2),
            );
            ui.label(egui::RichText::new("to").color(theme::rgb(theme.text_muted)));
            ui.add(
                egui::DragValue::new(&mut mapping.range[0])
                    .range(0.0..=1.0)
                    .speed(0.005)
                    .fixed_decimals(2),
            );
        },
    );
    group.toggle(
        Row::new("Invert").description("The source's top drives the control's bottom."),
        &mut mapping.invert,
    );
    group.row(
        Row::new("Curve").description("How the source's travel maps onto the range."),
        |ui| {
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
        },
    );
    group.row(
        Row::new("Fader").description(
            "Pickup ignores the fader until it reaches the control's value, so nothing jumps. \
             Jump follows at once.",
        ),
        |ui| {
            egui::ComboBox::from_id_salt("mapping-takeover")
                .selected_text(mapping.takeover.label())
                .show_ui(ui, |ui| {
                    for mode in [Takeover::Pickup, Takeover::Jump] {
                        if ui
                            .selectable_label(mapping.takeover == mode, mode.label())
                            .clicked()
                        {
                            mapping.takeover = mode;
                        }
                    }
                });
        },
    );
    group.row(
        Row::new("Button").description(
            "Toggle flips the control on each press; momentary follows the button while it is \
             held.",
        ),
        |ui| {
            egui::ComboBox::from_id_salt("mapping-button")
                .selected_text(mapping.button.label())
                .show_ui(ui, |ui| {
                    for mode in [ButtonMode::Toggle, ButtonMode::Momentary] {
                        if ui
                            .selectable_label(mapping.button == mode, mode.label())
                            .clicked()
                        {
                            mapping.button = mode;
                        }
                    }
                });
        },
    );
    group.row(
        Row::new("Press at").description(
            "Where a fader counts as pressed when it drives a button, a command or a toggle.",
        ),
        |ui| {
            ui.add(
                egui::DragValue::new(&mut mapping.threshold)
                    .range(0.05..=0.95)
                    .speed(0.005)
                    .fixed_decimals(2),
            );
        },
    );
}

// ------------------------------------------------------------ add form

fn add_form(
    group: &mut form::Group<'_>,
    theme: &Palette,
    remote: &mut Remote,
    map: &mut MapUi,
    targets: &Targets<'_>,
) {
    let open = map.add.open;
    group.row(
        Row::new("Add a mapping").description(
            "Choose what to control, then move a fader, press a pad or send an OSC message.",
        ),
        |ui| {
            if widgets::button(ui, theme, Some(Icon::Mapping), "Add...")
                .on_hover_text("Choose what to control, then learn a MIDI or OSC source for it")
                .clicked()
            {
                map.add.open = !open;
            }
        },
    );
    if !map.add.open {
        return;
    }
    group.block(|ui| {
        form::inset(ui, theme, |rows| {
            add_rows(rows, theme, remote, &mut map.add, targets);
        });
    });
}

/// The add form's rows: what kind of target, which one, and how to bind it.
fn add_rows(
    rows: &mut form::Group<'_>,
    theme: &Palette,
    remote: &mut Remote,
    form: &mut super::model::AddForm,
    targets: &Targets<'_>,
) {
    rows.row(Row::new("Control"), |ui| {
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
    let hint = match form.kind {
        AddKind::SceneControl => {
            "A name the playing scene lacks is kept, and waits for a scene that has it."
        }
        AddKind::Meta => "One of the meta controls every scene has.",
        AddKind::Command => "A command, as the keyboard runs it.",
        AddKind::Cue => "A cue of the loaded playlist, counted from 1.",
        AddKind::Fav => "One of the fav slots under the preview.",
        AddKind::Preset => "A preset of the playing scene, by name.",
        AddKind::Macro => "One of the macros above.",
    };
    rows.row(Row::new("Target").description(hint), |ui| match form.kind {
        AddKind::SceneControl => control_picker(ui, theme, form, targets.scene, "control name"),
        AddKind::Meta => control_picker(ui, theme, form, targets.meta, "meta control name"),
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
            form::field(ui, theme, &mut form.text, "cue number", SHORT_FIELD);
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
            form::field(ui, theme, &mut form.text, "preset name", FIELD * 0.8);
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
                        if ui
                            .selectable_label(form.text == found.id.to_string(), label)
                            .clicked()
                        {
                            form.text = found.id.to_string();
                        }
                    }
                });
        }
    });
    let bank = match form.kind {
        AddKind::SceneControl => targets.scene,
        AddKind::Meta => targets.meta,
        AddKind::Command | AddKind::Cue | AddKind::Fav | AddKind::Preset | AddKind::Macro => None,
    };
    if let Some(dims) = components(bank, form) {
        rows.row(
            Row::new("Component").description("Which part of a pad or a colour the source moves."),
            |ui| {
                // Right to left: the last component first.
                for component in (0..dims).rev() {
                    let name = ["x or red", "y or green", "z or blue"][component.min(2)];
                    if ui
                        .selectable_label(usize::from(form.component) == component, name)
                        .clicked()
                    {
                        form.component = component as u8;
                    }
                }
            },
        );
    }
    let target = form_target(form, targets);
    let can_macro = matches!(
        form.kind,
        AddKind::SceneControl | AddKind::Meta | AddKind::Command | AddKind::Preset
    );
    if can_macro {
        rows.toggle(
            Row::new("Pin it as a macro too").description(
                "A macro is a card on the strip under the preview; it stays there when the \
                 scene changes.",
            ),
            &mut form.as_macro,
        );
    }
    let ready = target.is_some();
    rows.block(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                    .add_enabled_ui(ready, |ui| {
                        widgets::button(ui, theme, None, "Pin without a source")
                    })
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
    rows.ui().add_space(SPACE_XS);
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
    theme: &Palette,
    form: &mut super::model::AddForm,
    bank: Option<&ControlBank>,
    hint: &str,
) {
    // Right to left: the picker, then the field.
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
    form::field(ui, theme, &mut form.text, hint, FIELD * 0.8);
}

/// How many components the named control has, when it has more than one.
fn components(bank: Option<&ControlBank>, form: &super::model::AddForm) -> Option<usize> {
    let bank = bank?;
    let index = strobe_io::target::find_control(bank, form.text.trim(), &[])?;
    let def = bank.defs().get(index)?;
    matches!(def.kind, ControlKind::Xy(_) | ControlKind::Color { .. }).then_some(def.dims())
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
        Target::Action(_) => target.label(),
    }
}

/// A macro with a default name, for tests and the add form.
pub fn macro_for(target: Target) -> Macro {
    Macro::new(&default_macro_name(&target), target)
}
