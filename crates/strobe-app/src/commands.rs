// ============================================================================
// Module:       strobe_app::commands
// Description:  The keyboard commands: what each shortcut is, and the focus
//               rules that decide whether a key press may fire one. Pure, no
//               egui.
//
// Dependencies: std only
// ============================================================================

//! Commands and the focus model (UI_IMPLEMENTATION_PLAN sections 15 and 16).
//!
//! A keyboard shortcut is one way to invoke a command; the buttons in the
//! transport strip are another, and MIDI and OSC will be more. All of them
//! converge on the same action, so this module is just the *table* of
//! commands (id, name, category, default shortcut) and the *rule* that says
//! when a key press may fire one.
//!
//! ## The focus rule
//!
//! In a VJ app a stray key can change the show, so focus decides:
//!
//! - while a **text field** has focus, typing is typing: no command fires,
//!   and Esc belongs to the field (it leaves it);
//! - while any other **widget** has keyboard focus (a slider, a button),
//!   the keys it uses itself (arrows, Space) are its own and do not fire
//!   transport commands, so nudging a slider with the arrow keys never also
//!   changes scene. Keys it does not use (B, F11, Ctrl+F, Esc) still work;
//! - while a **modal** question is open, no command fires at all: the modal
//!   owns the keyboard (its own Esc cancels it);
//! - otherwise (the preview, a pane's background) every command is live.
//!
//! [`dispatch`] is that rule, and it is a pure function of the key and the
//! focus, so every row of the table above is a unit test.

/// A key a command can be bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// The space bar.
    Space,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Escape.
    Escape,
    /// F11.
    F11,
    /// The letter B.
    B,
    /// The letter F.
    F,
    /// The letter T.
    T,
}

/// A key with the modifiers that matter. `command` is Ctrl on Windows and
/// Linux and Cmd on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    /// The key.
    pub key: Key,
    /// Whether Ctrl (Cmd on macOS) is held.
    pub command: bool,
    /// Whether Shift is held.
    pub shift: bool,
    /// Whether Alt is held.
    pub alt: bool,
}

impl Chord {
    /// A key with no modifiers.
    pub const fn plain(key: Key) -> Self {
        Self {
            key,
            command: false,
            shift: false,
            alt: false,
        }
    }

    /// A key with the command modifier.
    pub const fn command(key: Key) -> Self {
        Self {
            key,
            command: true,
            shift: false,
            alt: false,
        }
    }

    /// The text for a tooltip: `Ctrl+F`, `Space`, `F11`.
    pub fn label(self) -> String {
        let key = match self.key {
            Key::Space => "Space",
            Key::Left => "Left",
            Key::Right => "Right",
            Key::Escape => "Esc",
            Key::F11 => "F11",
            Key::B => "B",
            Key::F => "F",
            Key::T => "T",
        };
        let mut text = String::new();
        if self.command {
            text.push_str("Ctrl+");
        }
        if self.shift {
            text.push_str("Shift+");
        }
        if self.alt {
            text.push_str("Alt+");
        }
        text.push_str(key);
        text
    }
}

/// What a command belongs with, for a future palette and for the help text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// Moving between panes and surfaces.
    Navigation,
    /// Playback and cues.
    Playback,
    /// The output window.
    Window,
}

/// A command the keyboard can invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandId {
    /// Step to the next scene in the library.
    NextScene,
    /// Step to the previous scene.
    PrevScene,
    /// Hold or resume the cue (not wired yet).
    ToggleHold,
    /// Black out the output, or bring it back.
    ToggleBlackout,
    /// Fullscreen the output on its monitor.
    ToggleFullscreen,
    /// Open the scene browser and focus its search field.
    FocusBrowserSearch,
    /// Close the topmost contextual pane.
    CloseTopmostPane,
    /// Tap the tempo.
    TapTempo,
}

impl CommandId {
    /// Every command, in the order the help text lists them.
    pub const ALL: [CommandId; 8] = [
        Self::PrevScene,
        Self::NextScene,
        Self::ToggleHold,
        Self::ToggleBlackout,
        Self::ToggleFullscreen,
        Self::FocusBrowserSearch,
        Self::CloseTopmostPane,
        Self::TapTempo,
    ];

    /// The command's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::NextScene => "Next scene",
            Self::PrevScene => "Previous scene",
            Self::ToggleHold => "Hold / resume",
            Self::ToggleBlackout => "Blackout",
            Self::ToggleFullscreen => "Fullscreen output",
            Self::FocusBrowserSearch => "Search scenes",
            Self::CloseTopmostPane => "Close pane",
            Self::TapTempo => "Tap tempo",
        }
    }

    /// The command's category.
    pub fn category(self) -> Category {
        match self {
            Self::NextScene
            | Self::PrevScene
            | Self::ToggleHold
            | Self::ToggleBlackout
            | Self::TapTempo => Category::Playback,
            Self::FocusBrowserSearch | Self::CloseTopmostPane => Category::Navigation,
            Self::ToggleFullscreen => Category::Window,
        }
    }

    /// The command's default shortcut.
    pub fn chord(self) -> Chord {
        match self {
            Self::NextScene => Chord::plain(Key::Right),
            Self::PrevScene => Chord::plain(Key::Left),
            Self::ToggleHold => Chord::plain(Key::Space),
            Self::ToggleBlackout => Chord::plain(Key::B),
            Self::ToggleFullscreen => Chord::plain(Key::F11),
            Self::FocusBrowserSearch => Chord::command(Key::F),
            Self::CloseTopmostPane => Chord::plain(Key::Escape),
            Self::TapTempo => Chord::plain(Key::T),
        }
    }

    /// Whether the command uses a key a focused widget (a slider, a button)
    /// uses itself, and so must stand down while one has focus.
    pub fn yields_to_focused_widget(self) -> bool {
        match self {
            Self::NextScene | Self::PrevScene | Self::ToggleHold => true,
            Self::ToggleBlackout
            | Self::ToggleFullscreen
            | Self::FocusBrowserSearch
            | Self::CloseTopmostPane
            | Self::TapTempo => false,
        }
    }
}

/// Where keyboard focus is, as far as shortcuts care.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Focus {
    /// A text field has focus.
    pub text_edit: bool,
    /// Some widget has keyboard focus (this includes a text field).
    pub widget: bool,
    /// A modal question is open.
    pub modal: bool,
}

/// The command a key press invokes, if the focus allows one.
pub fn dispatch(chord: Chord, focus: Focus) -> Option<CommandId> {
    if focus.modal || focus.text_edit {
        return None;
    }
    let command = CommandId::ALL.into_iter().find(|c| c.chord() == chord)?;
    if focus.widget && command.yields_to_focused_widget() {
        return None;
    }
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FREE: Focus = Focus {
        text_edit: false,
        widget: false,
        modal: false,
    };

    #[test]
    fn every_listed_key_invokes_its_command() {
        for command in CommandId::ALL {
            assert_eq!(
                dispatch(command.chord(), FREE),
                Some(command),
                "{} on {}",
                command.name(),
                command.chord().label()
            );
        }
    }

    #[test]
    fn no_shortcut_is_bound_twice() {
        for (index, a) in CommandId::ALL.iter().enumerate() {
            for b in CommandId::ALL.iter().skip(index + 1) {
                assert!(
                    a.chord() != b.chord(),
                    "{} and {} share {}",
                    a.name(),
                    b.name(),
                    a.chord().label()
                );
            }
        }
    }

    #[test]
    fn typing_in_a_text_field_fires_nothing() {
        let typing = Focus {
            text_edit: true,
            widget: true,
            modal: false,
        };
        for command in CommandId::ALL {
            assert_eq!(
                dispatch(command.chord(), typing),
                None,
                "{} fired while typing",
                command.name()
            );
        }
    }

    #[test]
    fn a_modal_owns_the_keyboard() {
        let modal = Focus {
            modal: true,
            ..Focus::default()
        };
        for command in CommandId::ALL {
            assert_eq!(dispatch(command.chord(), modal), None, "{}", command.name());
        }
    }

    #[test]
    fn a_focused_slider_keeps_the_arrows_and_space_but_not_blackout() {
        let on_a_widget = Focus {
            widget: true,
            ..Focus::default()
        };
        for yielded in [
            CommandId::NextScene,
            CommandId::PrevScene,
            CommandId::ToggleHold,
        ] {
            assert_eq!(
                dispatch(yielded.chord(), on_a_widget),
                None,
                "{} must not fire over a focused widget",
                yielded.name()
            );
        }
        for live in [
            CommandId::ToggleBlackout,
            CommandId::ToggleFullscreen,
            CommandId::FocusBrowserSearch,
            CommandId::CloseTopmostPane,
        ] {
            assert_eq!(
                dispatch(live.chord(), on_a_widget),
                Some(live),
                "{} must stay live",
                live.name()
            );
        }
    }

    #[test]
    fn modifiers_must_match() {
        assert_eq!(
            dispatch(Chord::plain(Key::F), FREE),
            None,
            "a bare F is not Ctrl+F"
        );
        let shifted = Chord {
            shift: true,
            ..Chord::plain(Key::B)
        };
        assert_eq!(dispatch(shifted, FREE), None, "Shift+B is not B");
    }

    #[test]
    fn chords_read_the_way_a_person_would_say_them() {
        assert_eq!(Chord::command(Key::F).label(), "Ctrl+F");
        assert_eq!(Chord::plain(Key::Space).label(), "Space");
        assert_eq!(Chord::plain(Key::F11).label(), "F11");
    }
}
