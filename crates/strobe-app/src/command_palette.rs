// ============================================================================
// Module:       strobe_app::command_palette
// Description:  What the command palette lists, how a query ranks it, and why
//               an entry is unavailable. Pure, no egui.
//
// Dependencies: crate::{commands, workspace}
// ============================================================================

//! The command palette (Ctrl+K, UI_DESIGN section 25), as data.
//!
//! The palette is one more way to invoke the commands in
//! [`crate::commands`], next to the keyboard, the buttons, MIDI, OSC and the
//! remote. It lists every command (except the one that opens it), each with
//! its current shortcut, plus one entry per pane, so anything reachable from
//! the rail is reachable by typing. Running an entry goes through the same
//! code as its shortcut, so the palette can never mean something different
//! from the key.
//!
//! ## Ranking
//!
//! [`rank`] is a fuzzy match on the title and the category: every typed
//! character must appear in order. A match at the start of the title beats
//! one at the start of a word, which beats one anywhere, and fewer gaps
//! beat more. Ties keep the table order, so the list never reshuffles
//! between two equally good answers as the operator types.
//!
//! ## Unavailable entries stay listed
//!
//! An entry that cannot run now (Hold with no playlist, Next with one scene)
//! is shown with the reason instead of being hidden. A palette that hides
//! commands teaches the operator they do not exist. Choosing one does
//! nothing but keep the palette open.

use crate::commands::{Bindings, Category, CommandId};
use crate::workspace::Pane;

/// What an entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Run a command.
    Command(CommandId),
    /// Open a pane.
    Pane(Pane),
}

/// One row of the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What choosing it does.
    pub target: Target,
    /// The words shown and matched.
    pub title: String,
    /// The group shown beside it, also matched.
    pub category: &'static str,
    /// The shortcut, as the operator has bound it.
    pub shortcut: Option<String>,
    /// Why it cannot run now, if it cannot.
    pub unavailable: Option<&'static str>,
}

/// What the app's state allows right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Availability {
    /// A playlist is loaded, so the cue commands mean something.
    pub playlist: bool,
    /// There is more than one scene to step through.
    pub can_step: bool,
}

/// Every entry, in table order: the commands, then the panes.
pub fn items(bindings: &Bindings, availability: Availability) -> Vec<Item> {
    let mut items: Vec<Item> = CommandId::ALL
        .into_iter()
        .filter(|command| *command != CommandId::OpenPalette)
        .map(|command| Item {
            target: Target::Command(command),
            title: command.name().to_owned(),
            category: category_name(command.category()),
            shortcut: Some(bindings.chord(command).label()),
            unavailable: unavailable(command, availability),
        })
        .collect();
    items.extend(Pane::ALL.into_iter().map(|pane| Item {
        target: Target::Pane(pane),
        title: format!("Show {}", pane.title()),
        category: "Pane",
        shortcut: None,
        unavailable: None,
    }));
    items
}

fn category_name(category: Category) -> &'static str {
    match category {
        Category::Navigation => "Navigation",
        Category::Playback => "Playback",
        Category::Window => "Window",
    }
}

/// Why `command` cannot run in this state.
fn unavailable(command: CommandId, availability: Availability) -> Option<&'static str> {
    match command {
        CommandId::ToggleHold
        | CommandId::ExtendCue
        | CommandId::RestartCue
        | CommandId::SkipSection
            if !availability.playlist =>
        {
            Some("needs a playlist")
        }
        CommandId::NextScene | CommandId::PrevScene if !availability.can_step => {
            Some("only one scene")
        }
        CommandId::ToggleHold
        | CommandId::ExtendCue
        | CommandId::RestartCue
        | CommandId::SkipSection
        | CommandId::NextScene
        | CommandId::PrevScene
        | CommandId::ToggleBlackout
        | CommandId::ToggleFullscreen
        | CommandId::FocusBrowserSearch
        | CommandId::CloseTopmostPane
        | CommandId::TapTempo
        | CommandId::OpenPalette => None,
    }
}

/// The indices of `items` that match `query`, best first. An empty query
/// matches everything, in table order.
pub fn rank(items: &[Item], query: &str) -> Vec<usize> {
    let query: Vec<char> = query
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let mut scored: Vec<(u32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let title = score(&item.title, &query);
            let category = score(item.category, &query).map(|s| s + CATEGORY_PENALTY);
            let best = match (title, category) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            };
            best.map(|best| (best, index))
        })
        .collect();
    // Stable: equal scores keep table order.
    scored.sort_by_key(|(score, _)| *score);
    scored.into_iter().map(|(_, index)| index).collect()
}

/// Added to a match on the category, so a title match always wins.
const CATEGORY_PENALTY: u32 = 1000;

/// Lower is better; `None` when `query` is not a subsequence of `text`.
fn score(text: &str, query: &[char]) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    let mut at = 0;
    let mut first = None;
    let mut gaps = 0_u32;
    let mut last: Option<usize> = None;
    for wanted in query {
        let found = chars
            .get(at..)?
            .iter()
            .position(|c| c == wanted)
            .map(|offset| at + offset)?;
        if first.is_none() {
            first = Some(found);
        }
        if let Some(previous) = last
            && found != previous + 1
        {
            gaps += 1;
        }
        last = Some(found);
        at = found + 1;
    }
    let first = first?;
    let word_start = first == 0 || chars.get(first - 1).is_some_and(|c| !c.is_alphanumeric());
    let start = if first == 0 {
        0
    } else if word_start {
        100
    } else {
        200
    };
    Some(start + gaps * 10)
}

/// The highlighted row after a move of `delta` rows, clamped to the list
/// (`None` for an empty list).
pub fn moved(selected: Option<usize>, delta: isize, len: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;
    let current = selected.unwrap_or(0).min(last);
    Some(current.saturating_add_signed(delta).min(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(items: &[Item], query: &str) -> Vec<String> {
        rank(items, query)
            .into_iter()
            .filter_map(|i| items.get(i).map(|item| item.title.clone()))
            .collect()
    }

    fn everything() -> Vec<Item> {
        items(
            &Bindings::default(),
            Availability {
                playlist: true,
                can_step: true,
            },
        )
    }

    #[test]
    fn every_command_but_the_palette_and_every_pane_is_listed() {
        let items = everything();
        for command in CommandId::ALL {
            let listed = items
                .iter()
                .any(|item| item.target == Target::Command(command));
            assert_eq!(
                listed,
                command != CommandId::OpenPalette,
                "{} listed: {listed}",
                command.id()
            );
        }
        for pane in Pane::ALL {
            assert!(
                items.iter().any(|item| item.target == Target::Pane(pane)),
                "{} has an entry",
                pane.title()
            );
        }
    }

    #[test]
    fn a_query_ranks_title_starts_first_and_drops_non_matches() {
        let items = everything();
        assert_eq!(
            titles(&items, "black").first().map(String::as_str),
            Some("Blackout"),
            "the title that starts with it"
        );
        let next = titles(&items, "next");
        assert_eq!(
            next.first().map(String::as_str),
            Some("Next scene"),
            "a start-of-title match first; got {next:?}"
        );
        assert!(
            titles(&items, "zzzz").is_empty(),
            "no command contains zzzz"
        );
        assert_eq!(
            titles(&items, "").len(),
            items.len(),
            "an empty query lists everything"
        );
        assert_eq!(
            titles(&items, "  BLACK ").first().map(String::as_str),
            Some("Blackout"),
            "case and surrounding spaces do not matter"
        );
    }

    #[test]
    fn a_category_match_comes_after_title_matches() {
        let items = everything();
        let window = titles(&items, "window");
        assert_eq!(
            window,
            vec!["Fullscreen output".to_owned()],
            "only Fullscreen is in the Window category, and no title says window"
        );
        let show = titles(&items, "show");
        assert!(
            show.iter()
                .take(Pane::ALL.len())
                .all(|t| t.starts_with("Show ")),
            "every pane entry, by title, before anything else; got {show:?}"
        );
    }

    #[test]
    fn unavailable_commands_stay_listed_with_a_reason() {
        let items = items(&Bindings::default(), Availability::default());
        let reason = |command: CommandId| {
            items
                .iter()
                .find(|item| item.target == Target::Command(command))
                .and_then(|item| item.unavailable)
        };
        assert_eq!(
            reason(CommandId::ToggleHold),
            Some("needs a playlist"),
            "hold"
        );
        assert_eq!(reason(CommandId::NextScene), Some("only one scene"), "next");
        assert_eq!(
            reason(CommandId::ToggleBlackout),
            None,
            "blackout always runs"
        );
    }

    #[test]
    fn shortcuts_follow_the_operators_bindings() {
        let table = std::collections::BTreeMap::from([(
            "toggle_blackout".to_owned(),
            "Ctrl+Shift+B".to_owned(),
        )]);
        let (bindings, problems) = Bindings::from_config(&table);
        assert!(problems.is_empty(), "{problems:?}");
        let items = items(&bindings, Availability::default());
        let blackout = items
            .iter()
            .find(|item| item.target == Target::Command(CommandId::ToggleBlackout))
            .and_then(|item| item.shortcut.clone());
        assert_eq!(
            blackout.as_deref(),
            Some("Ctrl+Shift+B"),
            "the rebound chord"
        );
    }

    #[test]
    fn the_selection_moves_and_clamps() {
        assert_eq!(moved(None, 1, 0), None, "nothing to select");
        assert_eq!(moved(None, 1, 3), Some(1), "from the first row");
        assert_eq!(moved(Some(2), 1, 3), Some(2), "clamped at the end");
        assert_eq!(moved(Some(0), -1, 3), Some(0), "clamped at the start");
        assert_eq!(moved(Some(9), 0, 3), Some(2), "a stale index is clamped");
    }
}
