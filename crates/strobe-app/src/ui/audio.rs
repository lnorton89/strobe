// ============================================================================
// Module:       strobe_app::ui::audio
// Description:  Settings' Audio page: the active source, the input device
//               picker, reactivity and speed, and tap tempo.
//
// Dependencies: egui; strobe-audio (device types); super::{form, model,
//               widgets}; crate::audio
// ============================================================================

//! The Audio page of Settings, in `super::form`'s grammar.
//!
//! It shows what the title bar's audio readout also shows (the active
//! source), because on stage the first question about a dead-looking show is
//! "is it hearing anything?". Missing audio is a state, not an error: the
//! visuals keep running on the fixed synthetic track, and this section says
//! so in the input row's status and note, and offers the two things that
//! fix it: the rescan button and choosing a device.
//!
//! The widgets only edit the model's values and push actions; the app owns
//! the settings, so they survive a scene change.

use super::form::{self, Row};
use super::model::{Action, AudioPanel};
use super::widgets;
use crate::audio::GAIN;
use crate::icon::Icon;
use crate::palette::Palette;
use strobe_audio::capture::{DeviceChoice, DeviceInfo, DeviceKind};

/// The widest the device picker gets, in points.
const PICKER_WIDTH: f32 = 240.0;

/// The input row's title, also its entry in Settings' search.
pub const INPUT: &str = "Audio input";
/// The reactivity row's title.
pub const REACTIVITY: &str = "Reactivity";
/// The audio-speed row's title.
pub const SPEED: &str = "Audio speed";
/// The tempo row's title.
pub const TEMPO: &str = "Tempo";

/// Draws the page's groups.
pub fn show(
    ui: &mut egui::Ui,
    theme: &Palette,
    audio: &mut AudioPanel<'_>,
    actions: &mut Vec<Action>,
) {
    form::group(ui, theme, "Input", |group| {
        let (icon, word, colour) = if audio.live {
            (Icon::Check, "Listening", theme.success)
        } else if audio.can_capture {
            (Icon::Warning, "No input", theme.warning)
        } else {
            (Icon::Info, "Synthetic", theme.text_muted)
        };
        let mut row = Row::new(INPUT)
            .status(icon, word, colour)
            .description(audio.source);
        if audio.can_capture && !audio.live {
            row = row.note(
                "The visuals keep running on the built-in 120 BPM track until audio arrives.",
                theme.warning,
            );
        }
        group.row(row, |ui| {
            ui.add_enabled_ui(audio.can_capture, |ui| {
                if widgets::icon_button(ui, theme, Icon::Refresh, "List the audio devices again")
                    .clicked()
                {
                    actions.push(Action::RefreshAudioDevices);
                }
                egui::ComboBox::from_id_salt("audio-input")
                    .width(ui.available_width().min(PICKER_WIDTH))
                    .selected_text(choice_label(audio.choice, audio.devices))
                    .show_ui(ui, |ui| input_choices(ui, audio, actions));
            });
        });
    });
    form::group(ui, theme, "Response", |group| {
        group.row(
            Row::new(REACTIVITY)
                .description("How strongly scenes answer the input level. 1.00 is as heard."),
            |ui| {
                if form::slider(ui, audio.reactivity, GAIN, None, |v| format!("{v:.2}")).changed() {
                    actions.push(Action::SetReactivity(*audio.reactivity));
                }
            },
        );
        group.row(
            Row::new(SPEED).description("How fast the audio-driven clocks run. 1.00 is normal."),
            |ui| {
                if form::slider(ui, audio.speed, GAIN, None, |v| format!("{v:.2}")).changed() {
                    actions.push(Action::SetAudioSpeed(*audio.speed));
                }
            },
        );
        let tempo = format!("{:.1} BPM", audio.bpm);
        group.row(
            Row::new(TEMPO)
                .status(Icon::Audio, &tempo, theme.text_muted)
                .description(if audio.live {
                    "Tap along with the beat, or press T."
                } else {
                    "Tap tempo needs a live audio input."
                }),
            |ui| {
                let tap = ui
                    .add_enabled_ui(audio.live, |ui| widgets::button(ui, theme, None, "Tap"))
                    .inner;
                if tap.clicked() {
                    actions.push(Action::TapTempo);
                }
            },
        );
    });
}

/// The entries of the picker: the defaults, then every device found.
fn input_choices(ui: &mut egui::Ui, audio: &AudioPanel<'_>, actions: &mut Vec<Action>) {
    let mut entry = |ui: &mut egui::Ui, label: &str, choice: DeviceChoice| {
        if ui
            .selectable_label(*audio.choice == choice, label)
            .clicked()
        {
            actions.push(Action::SelectAudioInput(choice));
        }
    };
    entry(ui, "Default input", DeviceChoice::DefaultInput);
    if audio
        .devices
        .iter()
        .any(|device| device.kind == DeviceKind::Loopback)
    {
        entry(
            ui,
            "Default output (loopback)",
            DeviceChoice::DefaultLoopback,
        );
    }
    for device in audio.devices {
        entry(ui, &device.name, DeviceChoice::ById(device.id.clone()));
    }
}

/// The picker's closed text: what is chosen, by name where known.
fn choice_label(choice: &DeviceChoice, devices: &[DeviceInfo]) -> String {
    match choice {
        DeviceChoice::DefaultInput => "Default input".to_string(),
        DeviceChoice::DefaultLoopback => "Default output (loopback)".to_string(),
        DeviceChoice::ById(id) => devices
            .iter()
            .find(|device| device.id == *id)
            .map_or_else(|| id.clone(), |device| device.name.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn device(id: &str, name: &str) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: name.to_string(),
            kind: DeviceKind::Input,
            is_default: false,
        }
    }

    #[test]
    fn the_picker_names_the_chosen_device() -> Result<()> {
        let devices = [device("a", "Line In"), device("b", "Mic")];
        assert_eq!(
            choice_label(&DeviceChoice::ById("b".to_string()), &devices),
            "Mic",
            "a known id shows its name"
        );
        assert_eq!(
            choice_label(&DeviceChoice::ById("gone".to_string()), &devices),
            "gone",
            "an unknown id shows itself"
        );
        assert_eq!(
            choice_label(&DeviceChoice::DefaultInput, &devices),
            "Default input",
            "default"
        );
        Ok(())
    }
}
