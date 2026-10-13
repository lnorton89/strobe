// ============================================================================
// Module:       strobe_app::ui::keys
// Description:  Turns egui key events into commands under the focus rule, and
//               carries each command out as an action or a presentation
//               change.
//
// Dependencies: egui; crate::commands; super::{model, browser}, crate::workspace
// ============================================================================

//! The keyboard, as the control window sees it.
//!
//! [`crate::commands`] owns the table of commands and the focus rule as pure
//! data. This module reads the key events egui collected this frame, asks
//! [`crate::commands::dispatch`] whether each may fire given where focus is,
//! and carries out what it names:
//!
//! - **runtime** commands (next, previous, hold, blackout, fullscreen, tap
//!   tempo) become [`Action`]s for the app to carry out after the frame;
//! - **presentation** commands (focus the search field, close the topmost
//!   pane) change the [`UiState`](super::state::UiState) directly.
//!
//! Keys pressed in the output window do not come through here; the app maps
//! those with the same table (`App::output_key`).

use super::model::{Action, Model};
use crate::commands::{Chord, CommandId, Focus, Key, dispatch_with};
use crate::workspace::Pane;
use egui::Context;

/// The most key presses handled in one frame; a held-down burst beyond this
/// is not a performance gesture.
const MAX_PER_FRAME: usize = 8;

/// Maps an egui key to a command key: the named keys by hand, letters,
/// digits and function keys by their name (`A`, `7`, `F5`).
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "egui::Key has ~150 variants; the named ones are listed, every \
              other key is read from its name and ignored unless it is a \
              letter, a digit or a function key"
)]
pub fn key_from(key: egui::Key) -> Option<Key> {
    match key {
        egui::Key::Space => Some(Key::Space),
        egui::Key::ArrowLeft => Some(Key::Left),
        egui::Key::ArrowRight => Some(Key::Right),
        egui::Key::Escape => Some(Key::Escape),
        other => Key::parse(other.name()),
    }
}

/// Reads this frame's key presses and runs the commands they invoke.
pub fn handle(ctx: &Context, model: &mut Model<'_>, modal_open: bool, actions: &mut Vec<Action>) {
    let focus = Focus {
        text_edit: ctx.text_edit_focused(),
        widget: ctx.egui_wants_keyboard_input(),
        modal: modal_open,
    };
    let bindings = model.remote.bindings;
    let mut commands: [Option<CommandId>; MAX_PER_FRAME] = [None; MAX_PER_FRAME];
    let mut count = 0;
    ctx.input(|input| {
        for event in &input.events {
            let egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            let Some(key) = key_from(*key) else {
                continue;
            };
            let chord = Chord {
                key,
                command: modifiers.command,
                shift: modifiers.shift,
                alt: modifiers.alt,
            };
            if let Some(command) = dispatch_with(chord, focus, bindings)
                && let Some(slot) = commands.get_mut(count)
            {
                *slot = Some(command);
                count += 1;
            }
        }
    });
    for command in commands.into_iter().flatten() {
        run(command, model, actions);
    }
}

/// Carries out one command.
pub fn run(command: CommandId, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    match command {
        CommandId::NextScene => actions.push(Action::NextScene),
        CommandId::PrevScene => actions.push(Action::PrevScene),
        CommandId::ToggleHold => actions.push(Action::ToggleHold),
        CommandId::ExtendCue => actions.push(Action::ExtendCue),
        CommandId::RestartCue => actions.push(Action::RestartCue),
        CommandId::SkipSection => actions.push(Action::SkipSection),
        CommandId::ToggleBlackout => {
            actions.push(Action::SetBlackout(!model.transport.blackout));
        }
        CommandId::BlackoutOn => actions.push(Action::SetBlackout(true)),
        CommandId::BlackoutOff => actions.push(Action::SetBlackout(false)),
        CommandId::ToggleFullscreen => actions.push(Action::ToggleFullscreen),
        CommandId::TapTempo => actions.push(Action::TapTempo),
        CommandId::FocusBrowserSearch => {
            model.ui.panes.open(Pane::Browser);
            model.ui.browser.focus_search = true;
        }
        CommandId::CloseTopmostPane => {
            let _ = model.ui.panes.close_topmost();
        }
        CommandId::OpenPalette => model.ui.command_palette.toggle(),
        CommandId::GenerateControls => actions.push(Action::GenerateControls),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_key_is_mapped_from_an_egui_key() {
        // The table's keys must all be reachable from egui, or a command
        // would exist and never fire.
        let egui_keys = [
            egui::Key::Space,
            egui::Key::ArrowLeft,
            egui::Key::ArrowRight,
            egui::Key::Escape,
            egui::Key::F11,
            egui::Key::B,
            egui::Key::E,
            egui::Key::R,
            egui::Key::F,
            egui::Key::T,
            egui::Key::K,
        ];
        for command in CommandId::ALL {
            let Some(wanted) = command.chord().map(|chord| chord.key) else {
                continue;
            };
            assert!(
                egui_keys.iter().any(|k| key_from(*k) == Some(wanted)),
                "{} is bound to {wanted:?}, which no egui key maps to",
                command.name()
            );
        }
    }

    #[test]
    fn letters_digits_and_function_keys_read_from_their_names_and_the_rest_are_ignored() {
        assert_eq!(key_from(egui::Key::A), Some(Key::Letter('A')), "a letter");
        assert_eq!(
            key_from(egui::Key::B),
            Some(Key::B),
            "B keeps its own variant"
        );
        assert_eq!(key_from(egui::Key::Num7), Some(Key::Digit(7)), "a digit");
        assert_eq!(
            key_from(egui::Key::F5),
            Some(Key::Function(5)),
            "a function key"
        );
        assert_eq!(key_from(egui::Key::F11), Some(Key::F11), "F11");
        assert_eq!(key_from(egui::Key::Enter), None, "enter");
        assert_eq!(key_from(egui::Key::Tab), None, "tab");
        assert_eq!(key_from(egui::Key::Minus), None, "punctuation");
    }
}
