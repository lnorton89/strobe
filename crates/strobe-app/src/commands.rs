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
    /// The letter E.
    E,
    /// The letter R.
    R,
    /// The letter F.
    F,
    /// The letter T.
    T,
    /// Any other letter, `A` to `Z`. Build it with [`Key::letter`], which
    /// maps B, E, R, F and T to their own variants so a key has one form.
    Letter(char),
    /// A digit, 0 to 9.
    Digit(u8),
    /// A function key other than F11, 1 to 12. Build it with
    /// [`Key::function`].
    Function(u8),
}

impl Key {
    /// The key for a letter (either case), or `None` for anything else.
    pub fn letter(c: char) -> Option<Self> {
        let c = c.to_ascii_uppercase();
        Some(match c {
            'B' => Self::B,
            'E' => Self::E,
            'R' => Self::R,
            'F' => Self::F,
            'T' => Self::T,
            'A'..='Z' => Self::Letter(c),
            _ => return None,
        })
    }

    /// The key for function key `n`, 1 to 12.
    pub fn function(n: u8) -> Option<Self> {
        match n {
            11 => Some(Self::F11),
            1..=12 => Some(Self::Function(n)),
            _ => None,
        }
    }

    /// The key named `text`: a letter, a digit, `F1` to `F12`, `Space`,
    /// `Left`, `Right`, `Esc` or `Escape` (any case).
    pub fn parse(text: &str) -> Option<Self> {
        let lower = text.trim().to_ascii_lowercase();
        match lower.as_str() {
            "space" => return Some(Self::Space),
            "left" => return Some(Self::Left),
            "right" => return Some(Self::Right),
            "esc" | "escape" => return Some(Self::Escape),
            _ => {}
        }
        if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            return Self::function(n);
        }
        let mut chars = lower.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_digit() => Some(Self::Digit(c as u8 - b'0')),
            (Some(c), None) => Self::letter(c),
            _ => None,
        }
    }
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

    /// The chord written as text: `Ctrl+Shift+B`, `Alt+F3`, `Space`.
    /// `Ctrl`, `Shift`, `Alt` and `Cmd` (the same as `Ctrl`) are the
    /// modifiers, joined to the key by `+`, in any order and any case.
    pub fn parse(text: &str) -> Option<Self> {
        let mut chord = Self::plain(Key::Space);
        let mut key = None;
        for part in text.split('+') {
            match part.trim().to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => chord.command = true,
                "shift" => chord.shift = true,
                "alt" | "option" => chord.alt = true,
                other => {
                    if key.is_some() {
                        return None;
                    }
                    key = Some(Key::parse(other)?);
                }
            }
        }
        chord.key = key?;
        Some(chord)
    }

    /// The text for a tooltip: `Ctrl+F`, `Space`, `F11`.
    pub fn label(self) -> String {
        let key = match self.key {
            Key::Space => "Space".to_string(),
            Key::Left => "Left".to_string(),
            Key::Right => "Right".to_string(),
            Key::Escape => "Esc".to_string(),
            Key::F11 => "F11".to_string(),
            Key::B => "B".to_string(),
            Key::E => "E".to_string(),
            Key::R => "R".to_string(),
            Key::F => "F".to_string(),
            Key::T => "T".to_string(),
            Key::Letter(c) => c.to_string(),
            Key::Digit(d) => d.to_string(),
            Key::Function(n) => format!("F{n}"),
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
        text.push_str(&key);
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
    /// The playing scene itself.
    Scene,
}

/// A command the keyboard can invoke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandId {
    /// Step to the next scene in the library.
    NextScene,
    /// Step to the previous scene.
    PrevScene,
    /// Hold or resume the cue (needs a playlist loaded).
    ToggleHold,
    /// Give the current cue 30 more seconds (needs a playlist).
    ExtendCue,
    /// Start the current cue again (needs a playlist).
    RestartCue,
    /// Go to the first cue of the next section (needs a playlist).
    SkipSection,
    /// Black out the output, or bring it back.
    ToggleBlackout,
    /// Black out the output; already black stays black.
    BlackoutOn,
    /// Bring the output back; already showing stays showing.
    BlackoutOff,
    /// Fullscreen the output on its monitor.
    ToggleFullscreen,
    /// Open the scene browser and focus its search field.
    FocusBrowserSearch,
    /// Close the topmost contextual pane.
    CloseTopmostPane,
    /// Tap the tempo.
    TapTempo,
    /// Open the command palette, or close it.
    OpenPalette,
    /// Generate controls for the playing scene in the background (D202).
    GenerateControls,
}

impl CommandId {
    /// Every command, in the order the help text lists them.
    pub const ALL: [CommandId; 15] = [
        Self::PrevScene,
        Self::NextScene,
        Self::ToggleHold,
        Self::ExtendCue,
        Self::RestartCue,
        Self::SkipSection,
        Self::ToggleBlackout,
        Self::BlackoutOn,
        Self::BlackoutOff,
        Self::ToggleFullscreen,
        Self::FocusBrowserSearch,
        Self::CloseTopmostPane,
        Self::TapTempo,
        Self::OpenPalette,
        Self::GenerateControls,
    ];

    /// The command's stable id, the name mappings, OSC addresses
    /// (`/strobe/cmd/<id>`) and the hotkey file use. It never changes once
    /// shipped, whatever the display name does.
    pub fn id(self) -> &'static str {
        match self {
            Self::NextScene => "next_scene",
            Self::PrevScene => "prev_scene",
            Self::ToggleHold => "toggle_hold",
            Self::ExtendCue => "extend_cue",
            Self::RestartCue => "restart_cue",
            Self::SkipSection => "skip_section",
            Self::ToggleBlackout => "toggle_blackout",
            Self::BlackoutOn => "blackout_on",
            Self::BlackoutOff => "blackout_off",
            Self::ToggleFullscreen => "toggle_fullscreen",
            Self::FocusBrowserSearch => "focus_browser_search",
            Self::CloseTopmostPane => "close_topmost_pane",
            Self::TapTempo => "tap_tempo",
            Self::OpenPalette => "open_palette",
            Self::GenerateControls => "generate_controls",
        }
    }

    /// The command with this id (case-insensitive), if there is one.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| command.id().eq_ignore_ascii_case(id))
    }

    /// Whether a controller may run it. The two presentation commands act on
    /// the control window's panes, and a pad that closes a pane in the
    /// middle of a show is a mistake waiting to happen.
    pub fn mappable(self) -> bool {
        match self {
            // A pad that starts minutes of measuring mid-show is a mistake
            // waiting to happen.
            Self::FocusBrowserSearch
            | Self::CloseTopmostPane
            | Self::OpenPalette
            | Self::GenerateControls => false,
            Self::NextScene
            | Self::PrevScene
            | Self::ToggleHold
            | Self::ExtendCue
            | Self::RestartCue
            | Self::SkipSection
            | Self::ToggleBlackout
            | Self::BlackoutOn
            | Self::BlackoutOff
            | Self::ToggleFullscreen
            | Self::TapTempo => true,
        }
    }

    /// The command's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::NextScene => "Next scene",
            Self::PrevScene => "Previous scene",
            Self::ToggleHold => "Hold / resume",
            Self::ExtendCue => "Extend cue by 30 s",
            Self::RestartCue => "Restart cue",
            Self::SkipSection => "Skip section",
            Self::ToggleBlackout => "Blackout",
            Self::BlackoutOn => "Blackout on",
            Self::BlackoutOff => "Blackout off",
            Self::ToggleFullscreen => "Fullscreen output",
            Self::FocusBrowserSearch => "Search scenes",
            Self::CloseTopmostPane => "Close pane",
            Self::TapTempo => "Tap tempo",
            Self::OpenPalette => "Command palette",
            Self::GenerateControls => "Generate controls",
        }
    }

    /// The command's category.
    pub fn category(self) -> Category {
        match self {
            Self::NextScene
            | Self::PrevScene
            | Self::ToggleHold
            | Self::ExtendCue
            | Self::RestartCue
            | Self::SkipSection
            | Self::ToggleBlackout
            | Self::BlackoutOn
            | Self::BlackoutOff
            | Self::TapTempo => Category::Playback,
            Self::FocusBrowserSearch | Self::CloseTopmostPane | Self::OpenPalette => {
                Category::Navigation
            }
            Self::ToggleFullscreen => Category::Window,
            Self::GenerateControls => Category::Scene,
        }
    }

    /// The command's default shortcut. The explicit blackout commands have
    /// none: they are for controllers and the remote, where a toggle can get
    /// out of step with the output, and a key is already on the toggle.
    pub fn chord(self) -> Option<Chord> {
        Some(match self {
            Self::NextScene => Chord::plain(Key::Right),
            Self::PrevScene => Chord::plain(Key::Left),
            Self::ToggleHold => Chord::plain(Key::Space),
            Self::ExtendCue => Chord::plain(Key::E),
            Self::RestartCue => Chord::plain(Key::R),
            Self::SkipSection => Chord {
                shift: true,
                ..Chord::plain(Key::Right)
            },
            Self::ToggleBlackout => Chord::plain(Key::B),
            Self::ToggleFullscreen => Chord::plain(Key::F11),
            Self::FocusBrowserSearch => Chord::command(Key::F),
            Self::CloseTopmostPane => Chord::plain(Key::Escape),
            Self::TapTempo => Chord::plain(Key::T),
            Self::OpenPalette => Chord::command(Key::Letter('K')),
            Self::BlackoutOn | Self::BlackoutOff | Self::GenerateControls => return None,
        })
    }

    /// Whether the command uses a key a focused widget (a slider, a button)
    /// uses itself, and so must stand down while one has focus.
    pub fn yields_to_focused_widget(self) -> bool {
        match self {
            Self::NextScene | Self::PrevScene | Self::ToggleHold | Self::SkipSection => true,
            Self::ExtendCue
            | Self::RestartCue
            | Self::ToggleBlackout
            | Self::BlackoutOn
            | Self::BlackoutOff
            | Self::ToggleFullscreen
            | Self::FocusBrowserSearch
            | Self::CloseTopmostPane
            | Self::TapTempo
            | Self::OpenPalette
            | Self::GenerateControls => false,
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

/// The chord text that means "no shortcut" in the `hotkeys` table.
pub const UNBOUND: &str = "none";

/// The operator's keyboard shortcut overrides (D178): the `hotkeys` table of
/// `show-control.json`, command id to chord text (or [`UNBOUND`]), read at
/// start and again whenever the Settings editor changes it (D208).
///
/// A line that names an unknown command, writes a chord that cannot be read,
/// or would put two commands on one chord is **left out and reported**; the
/// command keeps its default. Lines that unbind are applied first, so a
/// table that moves a key from one command to another reads the same
/// whatever order its lines are in. Nothing here puts two commands on one
/// key; a command loses its key only when a line says so.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bindings {
    overrides: Vec<(CommandId, Option<Chord>)>,
}

impl Bindings {
    /// Reads the table. Returns the bindings and one line per rejected entry.
    pub fn from_config(table: &std::collections::BTreeMap<String, String>) -> (Self, Vec<String>) {
        let mut bindings = Self::default();
        let mut problems = Vec::new();
        let mut chords = Vec::new();
        for (id, text) in table {
            let Some(command) = CommandId::from_id(id) else {
                problems.push(format!("hotkeys: there is no command \"{id}\""));
                continue;
            };
            if text.trim().eq_ignore_ascii_case(UNBOUND) {
                bindings.overrides.retain(|(c, _)| *c != command);
                bindings.overrides.push((command, None));
                continue;
            }
            let Some(chord) = Chord::parse(text) else {
                problems.push(format!(
                    "hotkeys: \"{text}\" for {id} is not a key (try Ctrl+Shift+B, F5, Space, or none)"
                ));
                continue;
            };
            chords.push((id, command, chord));
        }
        for (id, command, chord) in chords {
            if let Some(other) = CommandId::ALL
                .into_iter()
                .find(|c| *c != command && bindings.chord(*c) == Some(chord))
            {
                problems.push(format!(
                    "hotkeys: {} is already {}; {id} keeps {}",
                    chord.label(),
                    other.id(),
                    command
                        .chord()
                        .map_or_else(|| "no shortcut".to_owned(), Chord::label)
                ));
                continue;
            }
            bindings.overrides.retain(|(c, _)| *c != command);
            bindings.overrides.push((command, Some(chord)));
        }
        (bindings, problems)
    }

    /// The chord that runs `command`: the operator's, or the default; none
    /// for a command with no default that the operator has not bound, or
    /// one the operator unbound.
    pub fn chord(&self, command: CommandId) -> Option<Chord> {
        self.overrides
            .iter()
            .find(|(c, _)| *c == command)
            .map_or_else(|| command.chord(), |(_, chord)| *chord)
    }

    /// The command other than `except` that answers to `chord`, if any.
    pub fn holder(&self, chord: Chord, except: CommandId) -> Option<CommandId> {
        CommandId::ALL
            .into_iter()
            .find(|c| *c != except && self.chord(*c) == Some(chord))
    }

    /// Writes `command`'s shortcut into the operator's `table`: `chord`, or
    /// no shortcut for `None`. A chord equal to the default removes the
    /// line, so the file holds only real changes.
    ///
    /// If another command already answers to `chord`, nothing is written and
    /// that command is returned, unless `steal`, which leaves the other
    /// command with no shortcut (written as [`UNBOUND`]): the operator saw
    /// the clash and chose. Nothing is ever moved silently.
    pub fn assign(
        table: &mut std::collections::BTreeMap<String, String>,
        command: CommandId,
        chord: Option<Chord>,
        steal: bool,
    ) -> Result<(), CommandId> {
        let (current, _) = Self::from_config(table);
        if let Some(chord) = chord
            && let Some(other) = current.holder(chord, command)
        {
            if !steal {
                return Err(other);
            }
            Self::write(table, other, None);
        }
        Self::write(table, command, chord);
        Ok(())
    }

    /// Puts `command` back on its default shortcut. Refused, with the
    /// holder, if another command has taken that key meanwhile.
    pub fn reset(
        table: &mut std::collections::BTreeMap<String, String>,
        command: CommandId,
    ) -> Result<(), CommandId> {
        Self::assign(table, command, command.chord(), false)
    }

    fn write(
        table: &mut std::collections::BTreeMap<String, String>,
        command: CommandId,
        chord: Option<Chord>,
    ) {
        if chord == command.chord() {
            table.remove(command.id());
        } else {
            let text = chord.map_or_else(|| UNBOUND.to_owned(), Chord::label);
            table.insert(command.id().to_owned(), text);
        }
    }

    /// Whether `command` has been rebound.
    pub fn is_custom(&self, command: CommandId) -> bool {
        self.overrides.iter().any(|(c, _)| *c == command)
    }
}

/// The command a key press invokes with the default shortcuts, if the focus
/// allows one.
pub fn dispatch(chord: Chord, focus: Focus) -> Option<CommandId> {
    dispatch_with(chord, focus, &Bindings::default())
}

/// [`dispatch`] with the operator's overrides.
pub fn dispatch_with(chord: Chord, focus: Focus, bindings: &Bindings) -> Option<CommandId> {
    if focus.modal || focus.text_edit {
        return None;
    }
    let command = CommandId::ALL
        .into_iter()
        .find(|c| bindings.chord(*c) == Some(chord))?;
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

    /// Every command with a default shortcut, and the shortcut.
    fn bound() -> Vec<(CommandId, Chord)> {
        CommandId::ALL
            .into_iter()
            .filter_map(|command| command.chord().map(|chord| (command, chord)))
            .collect()
    }

    #[test]
    fn every_listed_key_invokes_its_command() {
        for (command, chord) in bound() {
            assert_eq!(
                dispatch(chord, FREE),
                Some(command),
                "{} on {}",
                command.name(),
                chord.label()
            );
        }
    }

    #[test]
    fn no_shortcut_is_bound_twice() {
        let bound = bound();
        for (index, (a, chord_a)) in bound.iter().enumerate() {
            for (b, chord_b) in bound.iter().skip(index + 1) {
                assert!(
                    chord_a != chord_b,
                    "{} and {} share {}",
                    a.name(),
                    b.name(),
                    chord_a.label()
                );
            }
        }
    }

    #[test]
    fn the_commands_with_no_default_key_are_the_explicit_ones() {
        let unbound: Vec<&str> = CommandId::ALL
            .into_iter()
            .filter(|c| c.chord().is_none())
            .map(CommandId::id)
            .collect();
        assert_eq!(
            unbound,
            ["blackout_on", "blackout_off", "generate_controls"],
            "which"
        );
        let (bindings, problems) = Bindings::from_config(&table(&[("blackout_on", "Shift+B")]));
        assert!(problems.is_empty(), "{problems:?}");
        let shift_b = Chord {
            shift: true,
            ..Chord::plain(Key::B)
        };
        assert_eq!(
            dispatch_with(shift_b, FREE, &bindings),
            Some(CommandId::BlackoutOn),
            "the operator can still give one a key"
        );
    }

    #[test]
    fn ids_are_unique_stable_and_round_trip() {
        for (index, a) in CommandId::ALL.iter().enumerate() {
            assert_eq!(
                CommandId::from_id(a.id()),
                Some(*a),
                "{} round trip",
                a.id()
            );
            assert_eq!(
                CommandId::from_id(&a.id().to_uppercase()),
                Some(*a),
                "{} ignores case",
                a.id()
            );
            for b in CommandId::ALL.iter().skip(index + 1) {
                assert!(a.id() != b.id(), "{} is used twice", a.id());
            }
        }
        assert_eq!(CommandId::from_id("nope"), None, "unknown");
        assert_eq!(
            CommandId::NextScene.id(),
            "next_scene",
            "the OSC docs name this"
        );
        assert_eq!(
            CommandId::ToggleBlackout.id(),
            "toggle_blackout",
            "and this"
        );
    }

    #[test]
    fn only_the_presentation_commands_are_kept_from_controllers() {
        let unmappable: Vec<&str> = CommandId::ALL
            .into_iter()
            .filter(|c| !c.mappable())
            .map(CommandId::id)
            .collect();
        assert_eq!(
            unmappable,
            [
                "focus_browser_search",
                "close_topmost_pane",
                "open_palette",
                "generate_controls"
            ],
            "which"
        );
    }

    #[test]
    fn keys_and_chords_parse_from_the_text_a_person_writes() {
        assert_eq!(
            Key::parse("b"),
            Some(Key::B),
            "letters with their own variant"
        );
        assert_eq!(Key::parse("Q"), Some(Key::Letter('Q')), "other letters");
        assert_eq!(Key::parse("7"), Some(Key::Digit(7)), "digits");
        assert_eq!(Key::parse("F11"), Some(Key::F11), "F11 keeps its variant");
        assert_eq!(Key::parse("f5"), Some(Key::Function(5)), "function keys");
        assert_eq!(Key::parse("F13"), None, "no F13");
        assert_eq!(Key::parse("Esc"), Some(Key::Escape), "esc");
        assert_eq!(Key::parse("ab"), None, "two letters");
        assert_eq!(Key::parse(""), None, "nothing");
        let chord = Chord::parse("shift + Ctrl+ b").map(Chord::label);
        assert_eq!(
            chord.as_deref(),
            Some("Ctrl+Shift+B"),
            "modifiers in any order"
        );
        assert_eq!(Chord::parse("Ctrl+"), None, "no key");
        assert_eq!(Chord::parse("A+B"), None, "two keys");
        assert_eq!(Chord::parse("Hyper+A"), None, "unknown modifier");
        for (command, chord) in bound() {
            assert_eq!(
                Chord::parse(&chord.label()),
                Some(chord),
                "{} survives its own label",
                command.name()
            );
        }
    }

    fn table(pairs: &[(&str, &str)]) -> std::collections::BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
            .collect()
    }

    #[test]
    fn a_rebound_command_answers_to_its_new_chord_and_not_its_old_one() {
        let (bindings, problems) = Bindings::from_config(&table(&[("toggle_blackout", "Ctrl+B")]));
        assert!(problems.is_empty(), "{problems:?}");
        let new = Chord::command(Key::B);
        assert_eq!(
            dispatch_with(new, FREE, &bindings),
            Some(CommandId::ToggleBlackout),
            "new"
        );
        assert_eq!(
            dispatch_with(Chord::plain(Key::B), FREE, &bindings),
            None,
            "the old one is free"
        );
        assert!(bindings.is_custom(CommandId::ToggleBlackout), "custom");
        assert!(!bindings.is_custom(CommandId::NextScene), "others are not");
        assert_eq!(
            dispatch_with(Chord::plain(Key::Right), FREE, &bindings),
            Some(CommandId::NextScene),
            "unrelated commands are untouched"
        );
    }

    #[test]
    fn the_focus_rule_applies_to_rebound_keys_too() {
        let (bindings, _) = Bindings::from_config(&table(&[("next_scene", "N")]));
        let typing = Focus {
            text_edit: true,
            widget: true,
            modal: false,
        };
        assert_eq!(
            dispatch_with(Chord::plain(Key::Letter('N')), typing, &bindings),
            None,
            "typing"
        );
        let on_widget = Focus {
            widget: true,
            ..Focus::default()
        };
        assert_eq!(
            dispatch_with(Chord::plain(Key::Letter('N')), on_widget, &bindings),
            None,
            "next_scene still yields to a focused widget wherever it is bound"
        );
    }

    #[test]
    fn bad_hotkey_lines_are_reported_and_leave_the_defaults() {
        let (bindings, problems) = Bindings::from_config(&table(&[
            ("no_such", "X"),
            ("next_scene", "Ctrl+"),
            ("prev_scene", "B"),
            ("tap_tempo", "Shift+Q"),
        ]));
        assert_eq!(problems.len(), 3, "three rejected: {problems:?}");
        assert!(
            problems[0].contains("no_such") || problems.iter().any(|p| p.contains("no_such")),
            "unknown id named"
        );
        assert_eq!(
            bindings.chord(CommandId::PrevScene),
            CommandId::PrevScene.chord(),
            "a clash with blackout's B leaves prev_scene on Left"
        );
        assert_eq!(
            bindings.chord(CommandId::NextScene),
            CommandId::NextScene.chord(),
            "unreadable chord"
        );
        assert_eq!(
            bindings
                .chord(CommandId::TapTempo)
                .map(Chord::label)
                .as_deref(),
            Some("Shift+Q"),
            "the good line applied"
        );
    }

    #[test]
    fn assigning_a_taken_key_names_the_holder_until_the_operator_steals_it() {
        let mut table = table(&[]);
        let tap_on_b = Bindings::assign(
            &mut table,
            CommandId::TapTempo,
            Some(Chord::plain(Key::B)),
            false,
        );
        assert_eq!(
            tap_on_b,
            Err(CommandId::ToggleBlackout),
            "B is blackout's: the clash is named, nothing written"
        );
        assert!(table.is_empty(), "nothing written: {table:?}");

        assert_eq!(
            Bindings::assign(
                &mut table,
                CommandId::TapTempo,
                Some(Chord::plain(Key::B)),
                true
            ),
            Ok(()),
            "stolen on purpose"
        );
        let (bindings, problems) = Bindings::from_config(&table);
        assert!(
            problems.is_empty(),
            "the written table reads clean: {problems:?}"
        );
        assert_eq!(
            dispatch_with(Chord::plain(Key::B), FREE, &bindings),
            Some(CommandId::TapTempo),
            "B taps tempo now"
        );
        assert_eq!(
            bindings.chord(CommandId::ToggleBlackout),
            None,
            "blackout lost its key, visibly"
        );
        assert_eq!(
            table.get("toggle_blackout").map(String::as_str),
            Some(UNBOUND),
            "written as none"
        );

        assert_eq!(
            Bindings::reset(&mut table, CommandId::ToggleBlackout),
            Err(CommandId::TapTempo),
            "blackout cannot have B back while tap tempo holds it"
        );
        assert_eq!(
            Bindings::reset(&mut table, CommandId::TapTempo),
            Ok(()),
            "tap tempo returns to T"
        );
        assert_eq!(
            Bindings::reset(&mut table, CommandId::ToggleBlackout),
            Ok(()),
            "now blackout can"
        );
        assert!(
            table.is_empty(),
            "back to defaults, nothing in the file: {table:?}"
        );
    }

    #[test]
    fn every_label_reads_back_as_its_chord() {
        for command in CommandId::ALL {
            if let Some(chord) = command.chord() {
                assert_eq!(
                    Chord::parse(&chord.label()),
                    Some(chord),
                    "{}",
                    command.name()
                );
            }
        }
        for text in ["Ctrl+Shift+F5", "Alt+7", "Shift+Q", "Esc", "Left"] {
            let chord = Chord::parse(text);
            assert_eq!(
                chord.map(Chord::label).as_deref(),
                Some(text),
                "{text} round-trips"
            );
        }
    }

    #[test]
    fn taking_a_held_key_needs_its_holder_to_let_go() {
        // Taking blackout's B for tap tempo is a clash even though tap tempo
        // gives up T: blackout still holds B. The table has to say that
        // blackout lets go (`"none"`), which is what the editor's "Move it
        // here" writes (D208).
        let (_, problems) = Bindings::from_config(&table(&[("tap_tempo", "B")]));
        assert_eq!(problems.len(), 1, "{problems:?}");
    }

    #[test]
    fn typing_in_a_text_field_fires_nothing() {
        let typing = Focus {
            text_edit: true,
            widget: true,
            modal: false,
        };
        for (command, chord) in bound() {
            assert_eq!(
                dispatch(chord, typing),
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
        for (command, chord) in bound() {
            assert_eq!(dispatch(chord, modal), None, "{}", command.name());
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
                yielded
                    .chord()
                    .and_then(|chord| dispatch(chord, on_a_widget)),
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
                live.chord().and_then(|chord| dispatch(chord, on_a_widget)),
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

    #[test]
    fn every_remote_command_is_a_mappable_app_command() {
        for remote in strobe_remote::RemoteCommand::ALL {
            let command = CommandId::from_id(remote.id());
            assert!(
                command.is_some_and(CommandId::mappable),
                "{remote:?} runs the app command {:?}",
                remote.id()
            );
        }
    }
}
