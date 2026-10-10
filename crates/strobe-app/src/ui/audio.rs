// ============================================================================
// Module:       strobe_app::ui::audio
// Description:  The audio section of Settings: the active source, the input
//               device picker, reactivity and speed, and tap tempo.
//
// Dependencies: egui; strobe-audio (device types); super::{model, theme,
//               widgets}; crate::audio
// ============================================================================

//! The audio section (in the Settings pane).
//!
//! It shows what the title bar's audio readout also shows (the active
//! source), because on stage the first question about a dead-looking show is
//! "is it hearing anything?". Missing audio is a state, not an error: the
//! visuals keep running on the fixed synthetic track, and this section says
//! so and offers the two things that fix it, *Rescan* and choosing a device.
//!
//! The widgets only edit the model's values and push actions; the app owns
//! the settings, so they survive a scene change.

use super::model::{Action, AudioPanel};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::audio::GAIN;
use crate::icon::Icon;
use crate::palette::Palette;
use strobe_audio::capture::{DeviceChoice, DeviceInfo, DeviceKind};

/// Width of the device picker, in points.
const PICKER_WIDTH: f32 = 190.0;

/// Draws the section.
pub fn show(
    ui: &mut egui::Ui,
    theme: &Palette,
    audio: &mut AudioPanel<'_>,
    actions: &mut Vec<Action>,
) {
    if audio.live {
        ui.label(
            egui::RichText::new(audio.source)
                .small()
                .color(theme::rgb(theme.success)),
        );
    } else if audio.can_capture {
        widgets::banner(
            ui,
            theme,
            Tone::Warning,
            &format!(
                "{}. The visuals keep running without audio reaction.",
                audio.source
            ),
            |_| {},
        );
    } else {
        ui.label(
            egui::RichText::new(audio.source)
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
    }
    ui.add_space(SPACE_XS);

    ui.add_enabled_ui(audio.can_capture, |ui| {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("audio-input")
                .width(PICKER_WIDTH)
                .selected_text(choice_label(audio.choice, audio.devices))
                .show_ui(ui, |ui| input_choices(ui, audio, actions));
            if widgets::button(ui, theme, Some(Icon::Refresh), "Rescan")
                .on_hover_text("List the audio devices again")
                .clicked()
            {
                actions.push(Action::RefreshAudioDevices);
            }
        });
    });
    ui.add_space(SPACE_XS);

    let reactivity = ui.add(
        egui::Slider::new(audio.reactivity, GAIN)
            .text("Reactivity")
            .custom_formatter(|value, _| format!("{value:.2}")),
    );
    if reactivity.changed() {
        actions.push(Action::SetReactivity(*audio.reactivity));
    }
    let speed = ui.add(
        egui::Slider::new(audio.speed, GAIN)
            .text("Audio speed")
            .custom_formatter(|value, _| format!("{value:.2}")),
    );
    if speed.changed() {
        actions.push(Action::SetAudioSpeed(*audio.speed));
    }

    ui.add_space(SPACE_SM);
    ui.horizontal(|ui| {
        let tap = ui
            .add_enabled_ui(audio.live, |ui| {
                widgets::button(ui, theme, None, "Tap tempo (T)")
            })
            .inner
            .on_disabled_hover_text("Tap tempo needs a live audio input");
        if tap.clicked() {
            actions.push(Action::TapTempo);
        }
        ui.label(
            egui::RichText::new(format!("{:.1} BPM", audio.bpm))
                .monospace()
                .color(theme::rgb(theme.text_muted)),
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
