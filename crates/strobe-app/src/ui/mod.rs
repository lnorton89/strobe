// ============================================================================
// Module:       strobe_app::ui
// Description:  The control window's interface: the egui host, the data the
//               panels read, the actions they return, the Contextual Perform
//               shell and its panes.
//
// Dependencies: egui, egui-winit, egui_glow (via submodules)
// ============================================================================

//! The control window's UI: **Contextual Perform** (D120).
//!
//! The UI is a pure function of a [`Model`] that returns [`Action`]s. Panels
//! read app state through the model, write control targets through the one
//! `&mut ControlBank` it carries, change *presentation* state (which panes
//! are open, the browser's query) on the [`UiState`] it carries, and ask the
//! app for everything else by returning an action. Product state (selection,
//! render scale, blackout, fullscreen) lives in the app, never in egui's
//! widget memory, so a future workspace layout, MIDI mapping or playlist can
//! change it without going through a widget.
//!
//! ## Layers (UI_IMPLEMENTATION_PLAN section 3)
//!
//! | Layer | Where |
//! |---|---|
//! | product state | the app (`app.rs`, `session.rs`, `audio.rs`) |
//! | presentation state | [`state`], persisted by [`crate::prefs`] |
//! | adapters | [`model`]: borrowed snapshots for one frame |
//! | commands | [`model::Action`], [`crate::commands`] |
//! | rendering | the preview paints the scene's GPU texture directly |
//!
//! ## Modules
//!
//! - Foundations, ported from rustaman (D5): [`theme`] (the palette as egui
//!   `Visuals` and the spacing scale), [`motion`], [`icon`], [`widgets`],
//!   [`modal`].
//! - The shell: [`shell`] composes [`chrome`] (title bar, readouts, rail,
//!   resize handles), the main view, [`side`] (the persistent panel:
//!   preview, status strip, fav slots and tabs) and [`transport`] with
//!   [`macros`]. [`perform`] draws the live preview, at full size as the
//!   PLAY view and fitted inside the side panel.
//! - The main views: [`browser`] (the library grid), [`import`],
//!   [`playlist`], [`editor`], [`map`], [`diagnostics`] and [`settings`]
//!   (with [`audio`]).
//! - The side panel's tabs: [`inspector`] (with [`controls`]; [`tags`] is
//!   the tag row it shares with the browser and import) and [`presets`].
//! - [`keys`] turns key events into commands under the focus rule.
//! - [`host`] runs egui on the control window.
//!
//! ## Rules the tests enforce
//!
//! The drawing modules are scanned by the tests below, the same ones
//! rustaman's UI has: no colour literals, no hand-picked gaps, no icon set in
//! a font, no animation outside [`motion`], and no fixed panel that offers a
//! resize handle that does nothing.

pub mod audio;
pub mod browser;
pub mod chrome;
pub mod command_palette;
pub mod controls;
pub mod diagnostics;
pub mod editor;
pub mod host;
pub mod icon;
pub mod import;
pub mod inspector;
pub mod keys;
pub mod lan;
pub mod macros;
pub mod map;
pub mod modal;
pub mod model;
pub mod motion;
pub mod perform;
pub mod playlist;
pub mod presets;
pub mod settings;
pub mod shell;
#[cfg(test)]
mod shell_tests;
pub mod side;
pub mod state;
pub mod tags;
#[cfg(test)]
mod testing;
pub mod theme;
pub mod thumbs;
pub mod transport;
pub mod widgets;

pub use host::UiHost;
pub use model::{
    Action, AddForm, AddKind, AudioPanel, CueTransport, EditorPanel, FailurePanel, GeneratePanel,
    ImportPanel, LanPanel, ListenerPanel, MapUi, MediaFrom, MediaPanel, Model, NoticePanel, Offer,
    PerfPanel, PlaylistPanel, PlaylistUi, Preview, RemotePanel, ScenePreview, TransportPanel,
};
pub use shell::ShellState;
pub use state::UiState;

#[cfg(test)]
mod tests {
    /// Every module that draws, by name and source. `theme` is not here: it
    /// is where colours are made. `motion` is not here either: it is the
    /// one module allowed to animate. `icon` holds the painting of the
    /// geometry and `state` has no drawing at all.
    const DRAWING_MODULES: [(&str, &str); 22] = [
        ("tags", include_str!("tags.rs")),
        ("audio", include_str!("audio.rs")),
        ("browser", include_str!("browser.rs")),
        ("chrome", include_str!("chrome.rs")),
        ("controls", include_str!("controls.rs")),
        ("diagnostics", include_str!("diagnostics.rs")),
        ("import", include_str!("import.rs")),
        ("inspector", include_str!("inspector.rs")),
        ("macros", include_str!("macros.rs")),
        ("map", include_str!("map.rs")),
        ("modal", include_str!("modal.rs")),
        ("perform", include_str!("perform.rs")),
        ("playlist", include_str!("playlist.rs")),
        ("presets", include_str!("presets.rs")),
        ("settings", include_str!("settings.rs")),
        ("shell", include_str!("shell.rs")),
        ("side", include_str!("side.rs")),
        ("transport", include_str!("transport.rs")),
        ("widgets", include_str!("widgets.rs")),
        ("icon", include_str!("icon.rs")),
        ("keys", include_str!("keys.rs")),
        ("lan", include_str!("lan.rs")),
    ];

    /// The lines a source scan should not read: comments explain the rules,
    /// and doc comments quote them. Anything after the `#[cfg(test)]` marker
    /// is a test, which may name the things it checks for.
    fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
        let end = source.find("#[cfg(test)]").unwrap_or(source.len());
        source[..end].lines().enumerate().filter(|(_, line)| {
            let trimmed = line.trim_start();
            !(trimmed.starts_with("//") || trimmed.starts_with('*'))
        })
    }

    #[test]
    fn no_drawing_module_sets_an_icon_in_a_font() {
        // egui's bundled fonts cover Latin, Greek, Cyrillic and an emoji
        // subset, and almost nothing in the Geometric Shapes, Miscellaneous
        // Symbols or Dingbats blocks, which is where every icon-shaped
        // character lives. A glyph the font lacks renders as an empty box.
        // Every icon is geometry now (`crate::icon`).
        for (name, source) in DRAWING_MODULES {
            for (number, line) in code_lines(source) {
                for character in line.chars() {
                    let code = character as u32;
                    let pictographic = matches!(
                        code,
                        // Arrows, Miscellaneous Technical, Geometric
                        // Shapes, Miscellaneous Symbols, Dingbats.
                        0x2190..=0x2BFF
                        // Emoji and the symbols-and-pictographs blocks.
                        | 0x1F000..=0x1FAFF
                    );
                    assert!(
                        !pictographic,
                        "{name}:{} sets an icon in a font: U+{code:04X}. \
                         egui's bundled fonts do not carry it, so it renders \
                         as an empty box: use crate::icon::Icon",
                        number + 1
                    );
                }
            }
        }
    }

    #[test]
    fn no_drawing_module_animates_by_hand() {
        // Every animation goes through `ui::motion`, which is what makes the
        // app's four durations four decisions rather than thirty call sites
        // that each picked a number, and what lets reduced motion work
        // without any widget's cooperation.
        for (name, source) in DRAWING_MODULES {
            for (number, line) in code_lines(source) {
                assert!(
                    !line.contains(".animate_"),
                    "{name}:{} animates by hand: {}. Use ui::motion, which \
                     owns the durations",
                    number + 1,
                    line.trim()
                );
            }
        }
    }

    #[test]
    fn no_drawing_module_holds_a_bare_duration() {
        // The other half of the rule above: a duration reaching `motion`
        // from a call site is the same drift with one more step in it.
        for (name, source) in DRAWING_MODULES {
            for (number, line) in code_lines(source) {
                let suspicious = line.contains("motion::toggle(")
                    || line.contains("motion::transition(")
                    || line.contains("motion::settled(");
                if !suspicious {
                    continue;
                }
                let arguments = line.split("motion::").nth(1).unwrap_or("");
                assert!(
                    !arguments.contains("0."),
                    "{name}:{} passes a bare duration: {}. Use INSTANT, \
                     QUICK, SETTLE or ENTER",
                    number + 1,
                    line.trim()
                );
            }
        }
    }

    #[test]
    fn no_fixed_panel_offers_a_resize_handle_that_does_nothing() {
        // `egui::Panel` is `resizable: true` by default. A fixed-size panel
        // that is left resizable shows a resize cursor, highlights, and then
        // clamps the drag straight back: a control that invites a gesture
        // and cannot answer it. Its separator line reads as the same
        // handle. (Strobe's docked panes resize through their own splitter,
        // which writes the width into the pane model.)
        for (name, source) in DRAWING_MODULES {
            let lines: Vec<&str> = source.lines().collect();
            for (number, line) in code_lines(source) {
                if !line.contains(".exact_size(") {
                    continue;
                }
                let chain = lines[number..(number + 6).min(lines.len())].join("\n");
                assert!(
                    chain.contains(".resizable(false)"),
                    "{name}:{} sizes a panel exactly but leaves it \
                     resizable: {}",
                    number + 1,
                    line.trim()
                );
                assert!(
                    chain.contains(".show_separator_line(false)"),
                    "{name}:{} sizes a panel exactly but still strokes its \
                     separator: {}",
                    number + 1,
                    line.trim()
                );
            }
        }
    }

    #[test]
    fn no_drawing_module_holds_a_colour_literal() {
        // Every colour comes from the theme. A single hard-coded grey is
        // invisible in review and then unreadable in half the themes.
        // Alpha-only constructors are lighting rather than theme colours.
        for (name, source) in DRAWING_MODULES {
            for (number, line) in code_lines(source) {
                if line.contains("from_black_alpha") || line.contains("from_white_alpha") {
                    continue;
                }
                assert!(
                    !line.contains("Color32::from_rgb")
                        && !line.contains("Color32::from_gray")
                        && !line.contains("Color32::WHITE")
                        && !line.contains("Color32::BLACK")
                        && !line.contains("Color32::RED")
                        && !line.contains("Color32::GREEN")
                        && !line.contains("Color32::BLUE"),
                    "{name}:{} holds a colour literal: {}",
                    number + 1,
                    line.trim()
                );
            }
        }
    }

    #[test]
    fn no_drawing_module_holds_a_hand_picked_pixel_gap() {
        // Every margin, inset and gap is one of the scale's values. An
        // `add_space(7.0)` is how a window ends up with four different gaps
        // that are each nearly the same.
        for (name, source) in DRAWING_MODULES {
            for (number, line) in code_lines(source) {
                if let Some(rest) = line.split("add_space(").nth(1) {
                    let argument = rest.split(')').next().unwrap_or("");
                    let named = argument.starts_with("SPACE_")
                        || argument.starts_with("PAD")
                        || argument.starts_with("theme::");
                    assert!(
                        named,
                        "{name}:{} adds a hand-picked gap of {argument}; use a \
                         step of the spacing scale",
                        number + 1
                    );
                }
            }
        }
    }

    #[test]
    fn no_drawing_module_reaches_for_the_driver_or_the_disk() {
        // UI_IMPLEMENTATION_PLAN section 18: nothing in the paint path scans
        // the filesystem, compiles a shader, enumerates devices or asks GL
        // a question. The panes read snapshots; the app does the work.
        let forbidden = [
            "std::fs::",
            "read_dir(",
            "glGet",
            "gl.get_",
            "std::thread::sleep",
            "TcpStream",
            "Command::new(",
        ];
        for (name, source) in DRAWING_MODULES {
            for (number, line) in code_lines(source) {
                for word in forbidden {
                    assert!(
                        !line.contains(word),
                        "{name}:{} reaches outside the frame with `{word}`: {}",
                        number + 1,
                        line.trim()
                    );
                }
            }
        }
    }
}
