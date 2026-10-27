// ============================================================================
// Module:       strobe_app::command_palette
// Description:  What the command palette lists (commands, panes, layouts,
//               scenes and presets), how a query ranks it, and why an entry
//               is unavailable. Pure, no egui.
//
// Dependencies: crate::{commands, library, workspace}
// ============================================================================

//! The command palette (Ctrl+K, UI_DESIGN section 25), as data.
//!
//! The palette is one more way to invoke the commands in
//! [`crate::commands`], next to the keyboard, the buttons, MIDI, OSC and the
//! remote. It lists every command (except the one that opens it), each with
//! its current shortcut, plus one entry per pane, so anything reachable from
//! the rail is reachable by typing, and the library's scenes and the playing
//! scene's presets. Running a command goes through the same
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
//!
//! ## Scenes and presets (D262)
//!
//! Each library scene is two entries, **Load** and **Preview**, and each
//! preset of the playing scene is one, **Apply preset**. They are matched on
//! their whole title and on the name alone, so `aurora` finds `Load:
//! 01-aurora` as well as a title start would. They rank after every command
//! whose title the query matches exactly (at the title's start or a word's,
//! with no gaps), and before commands it matches only loosely, so `black` is
//! still Blackout, and `aurora` is the scene even if some command happens to
//! contain those letters in order.
//!
//! The list is built by [`Listing`] when the palette opens and again only
//! when what it lists changes (the library, the playing scene, its presets,
//! the layouts, what is available), and ranked again only when the query
//! changes. A library of a thousand scenes costs nothing per frame while the
//! palette sits open, and nothing at all while it is closed.

use crate::commands::{Bindings, Category, CommandId};
use crate::library::Library;
use crate::workspace::Pane;

/// What an entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Run a command.
    Command(CommandId),
    /// Open a pane.
    Pane(Pane),
    /// Apply a saved layout, by name (D215).
    Layout(String),
    /// Open Settings at the form for controlling another Strobe desktop
    /// (D275): the connect form is otherwise the last paragraph of Remote.
    ControlDesktop,
    /// Load the library scene at this index (it becomes live).
    LoadScene(usize),
    /// Preview the library scene at this index without changing what is
    /// live.
    PreviewScene(usize),
    /// Recall this preset of the playing scene.
    ApplyPreset(String),
}

impl Target {
    /// Whether the entry names a scene or a preset rather than a command, a
    /// pane or a layout; those rank after exact command matches.
    fn is_content(&self) -> bool {
        match self {
            Self::LoadScene(_) | Self::PreviewScene(_) | Self::ApplyPreset(_) => true,
            Self::Command(_) | Self::Pane(_) | Self::Layout(_) | Self::ControlDesktop => false,
        }
    }
}

/// One row of the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// What choosing it does.
    pub target: Target,
    /// The words shown and matched.
    pub title: String,
    /// Where in `title` the scene's or preset's own name starts (0 for a
    /// command), so the name alone is matched too.
    pub name_at: usize,
    /// The group shown beside it, also matched for commands.
    pub category: &'static str,
    /// The shortcut, as the operator has bound it.
    pub shortcut: Option<String>,
    /// Why it cannot run now, if it cannot.
    pub unavailable: Option<&'static str>,
}

impl Item {
    /// An entry whose title is `verb` followed by `name`.
    fn named(target: Target, verb: &str, name: &str, category: &'static str) -> Self {
        Self {
            target,
            title: format!("{verb}{name}"),
            name_at: verb.len(),
            category,
            shortcut: None,
            unavailable: None,
        }
    }

    /// The scene's or preset's name, or the whole title for a command.
    pub fn name(&self) -> &str {
        self.title.get(self.name_at..).unwrap_or(&self.title)
    }
}

/// What the app's state allows right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Availability {
    /// A playlist is loaded, so the cue commands mean something.
    pub playlist: bool,
    /// There is more than one scene to step through.
    pub can_step: bool,
    /// A scene is playing.
    pub playing: bool,
    /// A generate-controls worker is running already.
    pub generating: bool,
}

/// Every command entry, in table order: the commands, the panes, then the
/// saved layouts.
pub fn items(bindings: &Bindings, availability: Availability, layouts: &[String]) -> Vec<Item> {
    let mut items: Vec<Item> = CommandId::ALL
        .into_iter()
        .filter(|command| *command != CommandId::OpenPalette)
        .map(|command| Item {
            target: Target::Command(command),
            title: command.name().to_owned(),
            name_at: 0,
            category: category_name(command.category()),
            shortcut: bindings.chord(command).map(|chord| chord.label()),
            unavailable: unavailable(command, availability),
        })
        .collect();
    items.extend(Pane::ALL.into_iter().map(|pane| Item {
        target: Target::Pane(pane),
        title: format!("Show {}", pane.title()),
        name_at: 0,
        category: "Pane",
        shortcut: None,
        unavailable: None,
    }));
    items.push(Item {
        target: Target::ControlDesktop,
        title: "Control another Strobe\u{2026}".to_owned(),
        name_at: 0,
        category: "Remote",
        shortcut: None,
        unavailable: None,
    });
    items.extend(layouts.iter().map(|name| Item {
        target: Target::Layout(name.clone()),
        title: format!("Layout: {name}"),
        name_at: 0,
        category: "Layout",
        shortcut: None,
        unavailable: None,
    }));
    items
}

/// What a scene's Load and Preview entries say when it is already live.
const PLAYING_NOW: &str = "playing now";

/// One Load and one Preview entry per library scene, in library order, then
/// one Apply entry per preset of the playing scene (D262).
pub fn content_items(library: &Library, playing: Option<usize>, presets: &[String]) -> Vec<Item> {
    let mut items = Vec::with_capacity(library.entries.len() * 2 + presets.len());
    for (index, entry) in library.entries.iter().enumerate() {
        let live = (playing == Some(index)).then_some(PLAYING_NOW);
        // Synesthesia's scenes say so where the category goes, so a name in
        // both places reads as two (D271).
        let category = entry.source.label().unwrap_or("Scene");
        let mut load = Item::named(Target::LoadScene(index), "Load: ", &entry.name, category);
        load.unavailable = live;
        let mut preview = Item::named(
            Target::PreviewScene(index),
            "Preview: ",
            &entry.name,
            category,
        );
        preview.unavailable = live;
        items.push(load);
        items.push(preview);
    }
    items.extend(presets.iter().map(|name| {
        Item::named(
            Target::ApplyPreset(name.clone()),
            "Apply preset: ",
            name,
            "Preset",
        )
    }));
    items
}

fn category_name(category: Category) -> &'static str {
    match category {
        Category::Navigation => "Navigation",
        Category::Playback => "Playback",
        Category::Window => "Window",
        Category::Scene => "Scene",
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
        CommandId::GenerateControls if !availability.playing => Some("no scene playing"),
        CommandId::GenerateControls if availability.generating => Some("already measuring"),
        CommandId::ToggleHold
        | CommandId::ExtendCue
        | CommandId::RestartCue
        | CommandId::SkipSection
        | CommandId::NextScene
        | CommandId::PrevScene
        | CommandId::ToggleBlackout
        | CommandId::BlackoutOn
        | CommandId::BlackoutOff
        | CommandId::ToggleFullscreen
        | CommandId::FocusBrowserSearch
        | CommandId::CloseTopmostPane
        | CommandId::TapTempo
        | CommandId::NewSong
        | CommandId::OpenPalette
        | CommandId::GenerateControls
        | CommandId::ToggleControlsView => None,
    }
}

/// What the listing is built from this frame, borrowed from the model.
#[derive(Clone, Copy)]
pub struct Sources<'a> {
    /// The keyboard shortcuts in force.
    pub bindings: &'a Bindings,
    /// What can run.
    pub availability: Availability,
    /// The scene library.
    pub library: &'a Library,
    /// Bumped whenever the library or its health changes.
    pub library_rev: u64,
    /// The library entry playing.
    pub playing: Option<usize>,
    /// The playing scene's presets.
    pub presets: &'a [String],
}

/// What a listing was built from, compared each frame without allocating.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Inputs {
    availability: Availability,
    library_rev: u64,
    playing: Option<usize>,
    presets: Vec<String>,
    layouts: Vec<String>,
}

/// What the palette lists, built once per change of what it lists and
/// ranked once per change of the query (D262).
#[derive(Debug, Default)]
pub struct Listing {
    /// What the entries were built from.
    built_from: Option<Inputs>,
    /// Every entry: [`items`], then [`content_items`].
    items: Vec<Item>,
    /// The query the ranking was made for.
    ranked_for: Option<String>,
    /// Indices into `items`, best first.
    ranked: Vec<usize>,
}

impl Listing {
    /// Forgets what it was built from, so the next [`Listing::update`]
    /// builds afresh (on opening: the shortcuts may have changed since).
    pub fn clear(&mut self) {
        self.built_from = None;
        self.ranked_for = None;
        // An opening palette's highlight follows nothing from before.
        self.ranked.clear();
    }

    /// Brings the entries and the ranking up to date with `sources`,
    /// `layouts` and `query`. Allocates only when one of them changed.
    /// Returns whether the ranking was made again.
    ///
    /// `selected`, the highlighted row, stays on the entry it was on when
    /// the entries are built again (a scene starts playing, the library
    /// changes): the row it is at now, or none if it is gone, so Enter
    /// never runs an entry the operator did not see highlighted (D403).
    pub fn update<'k>(
        &mut self,
        sources: Sources<'_>,
        layouts: impl Iterator<Item = &'k String> + Clone,
        query: &str,
        selected: &mut Option<usize>,
    ) -> bool {
        let fresh = self.built_from.as_ref().is_some_and(|built| {
            built.availability == sources.availability
                && built.library_rev == sources.library_rev
                && built.playing == sources.playing
                && built.presets.as_slice() == sources.presets
                && built.layouts.iter().eq(layouts.clone())
        });
        let mut kept = None;
        if !fresh {
            let highlighted = selected.and_then(|row| self.ranked.get(row).copied());
            let mut old = std::mem::take(&mut self.items);
            kept = highlighted
                .filter(|index| *index < old.len())
                .map(|index| old.swap_remove(index));
            let layouts: Vec<String> = layouts.cloned().collect();
            self.items = items(sources.bindings, sources.availability, &layouts);
            self.items.extend(content_items(
                sources.library,
                sources.playing,
                sources.presets,
            ));
            self.built_from = Some(Inputs {
                availability: sources.availability,
                library_rev: sources.library_rev,
                playing: sources.playing,
                presets: sources.presets.to_vec(),
                layouts,
            });
            self.ranked_for = None;
        }
        if self.ranked_for.as_deref() == Some(query) {
            return false;
        }
        self.ranked = rank(&self.items, query);
        match self.ranked_for.as_mut() {
            Some(text) => query.clone_into(text),
            None => self.ranked_for = Some(query.to_owned()),
        }
        if let Some(kept) = kept {
            *selected = self.row_of(&kept);
        }
        true
    }

    /// The row now showing `item`: the same target and title, or else the
    /// one entry of the same kind, title and category (a scene whose
    /// library index moved). `None` when it is gone or cannot be told.
    fn row_of(&self, item: &Item) -> Option<usize> {
        let at = |same: &dyn Fn(&Item) -> bool| {
            self.ranked
                .iter()
                .position(|index| self.items.get(*index).is_some_and(same))
        };
        if let Some(row) = at(&|other| other.target == item.target && other.title == item.title) {
            return Some(row);
        }
        let kind = std::mem::discriminant(&item.target);
        let similar = |other: &Item| {
            std::mem::discriminant(&other.target) == kind
                && other.title == item.title
                && other.category == item.category
        };
        let row = at(&similar)?;
        let twins = self
            .ranked
            .iter()
            .filter(|index| self.items.get(**index).is_some_and(similar))
            .count();
        (twins == 1).then_some(row)
    }

    /// Every entry.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Indices into [`Listing::items`] that match the query, best first.
    pub fn ranked(&self) -> &[usize] {
        &self.ranked
    }
}

/// The indices of `items` that match `query`, best first. An empty query
/// matches everything, in table order.
///
/// Commands, panes and layouts whose title the query matches exactly come
/// first, then scenes and presets, then commands it matches only loosely,
/// then matches on a command's category (see the module docs).
pub fn rank(items: &[Item], query: &str) -> Vec<usize> {
    let query: Vec<char> = query
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let mut scored: Vec<(Tier, u32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let best = if item.target.is_content() {
                better(score(&item.title, &query), score(item.name(), &query))
                    .map(|found| (Tier::Content, found.cost()))
            } else {
                let title = score(&item.title, &query).map(|found| {
                    let tier = if found.exact() {
                        Tier::Exact
                    } else {
                        Tier::Loose
                    };
                    (tier, found.cost())
                });
                let category =
                    score(item.category, &query).map(|found| (Tier::Category, found.cost()));
                match (title, category) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                }
            };
            best.map(|(tier, cost)| (tier, cost, index))
        })
        .collect();
    // Stable: equal scores keep table order.
    scored.sort_by_key(|(tier, cost, _)| (*tier, *cost));
    scored.into_iter().map(|(_, _, index)| index).collect()
}

/// Where a match ranks, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    /// A command, pane or layout title matched at a title or word start
    /// with no gaps.
    Exact,
    /// A scene or preset, matched in any way.
    Content,
    /// A command, pane or layout title matched with gaps or mid-word.
    Loose,
    /// A command's category matched.
    Category,
}

/// How a query matched a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Found {
    /// 0 at the start of the text, [`WORD_START`] at a word's start,
    /// [`ANYWHERE`] elsewhere.
    start: u32,
    /// Breaks between matched characters.
    gaps: u32,
}

/// A match that begins at the start of a word.
const WORD_START: u32 = 100;

/// A match that begins mid-word.
const ANYWHERE: u32 = 200;

impl Found {
    /// Lower is better.
    fn cost(self) -> u32 {
        self.start + self.gaps.saturating_mul(10)
    }

    /// Whether the query is in the text as typed, at a title or word start.
    fn exact(self) -> bool {
        self.gaps == 0 && self.start <= WORD_START
    }
}

/// The better of two matches; the first on a tie.
fn better(a: Option<Found>, b: Option<Found>) -> Option<Found> {
    match (a, b) {
        (Some(a), Some(b)) if b.cost() < a.cost() => Some(b),
        (Some(a), Some(_)) | (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

/// How `query` (lowercase, no spaces) matches `text`, each character at the
/// first place it can go; `None` when it is not a subsequence of it. Reads
/// `text` once and allocates nothing, so ranking a large library on a
/// keystroke stays cheap.
fn score(text: &str, query: &[char]) -> Option<Found> {
    let mut wanted = query.iter().peekable();
    let mut start = None;
    let mut gaps = 0_u32;
    let mut last: Option<usize> = None;
    let mut after_alphanumeric = false;
    for (at, c) in text.chars().flat_map(char::to_lowercase).enumerate() {
        let Some(&&next) = wanted.peek() else {
            break;
        };
        if c == next {
            if start.is_none() {
                start = Some(if at == 0 {
                    0
                } else if after_alphanumeric {
                    ANYWHERE
                } else {
                    WORD_START
                });
            }
            if let Some(previous) = last
                && at != previous + 1
            {
                gaps += 1;
            }
            last = Some(at);
            // Advancing the iterator is the point: this query character is
            // matched.
            let _ = wanted.next();
        }
        after_alphanumeric = c.is_alphanumeric();
    }
    if wanted.peek().is_some() {
        return None;
    }
    Some(Found {
        start: start.unwrap_or(0),
        gaps,
    })
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
                playing: true,
                generating: false,
            },
            &["Show".to_string()],
        )
    }

    #[test]
    fn a_saved_layout_is_an_entry_found_by_its_name() {
        let all = everything();
        assert_eq!(
            titles(&all, "layout").first().map(String::as_str),
            Some("Layout: Show"),
            "found"
        );
        assert!(
            all.iter()
                .any(|item| item.target == Target::Layout("Show".to_string())),
            "it applies that layout"
        );
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
        let items = items(&Bindings::default(), Availability::default(), &[]);
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
        let items = items(&bindings, Availability::default(), &[]);
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

    fn library(names: &[&str]) -> Library {
        Library {
            dir: std::path::PathBuf::from("scenes"),
            entries: names
                .iter()
                .map(|name| {
                    crate::library::Entry::new((*name).to_string(), std::path::PathBuf::from(name))
                })
                .collect(),
        }
    }

    fn with_content(library: &Library, playing: Option<usize>, presets: &[String]) -> Vec<Item> {
        let mut all = everything();
        all.extend(content_items(library, playing, presets));
        all
    }

    #[test]
    fn scenes_and_presets_are_entries_found_by_their_names() {
        let library = library(&["01-aurora", "02-black-hole", "03-tunnel"]);
        let all = with_content(&library, Some(0), &["Slow burn".to_string()]);
        let aurora = titles(&all, "aurora");
        assert_eq!(
            aurora.get(..2),
            Some(
                &[
                    "Load: 01-aurora".to_string(),
                    "Preview: 01-aurora".to_string()
                ][..]
            ),
            "a scene's name finds Load then Preview; got {aurora:?}"
        );
        assert_eq!(
            titles(&all, "slow burn").first().map(String::as_str),
            Some("Apply preset: Slow burn"),
            "a preset of the playing scene"
        );
        assert_eq!(
            titles(&all, "preview tun").first().map(String::as_str),
            Some("Preview: 03-tunnel"),
            "the verb and the name together"
        );
        assert!(
            all.iter()
                .any(|item| item.target == Target::ApplyPreset("Slow burn".to_string())),
            "it recalls that preset"
        );
    }

    #[test]
    fn synesthesias_scenes_are_listed_and_say_where_they_come_from() {
        let mut library = library(&["01-aurora", "ink"]);
        library.entries.push(
            crate::library::Entry::new(
                "ink".to_string(),
                std::path::PathBuf::from("release/ink.synScene"),
            )
            .from_source(crate::library::Source::Synesthesia),
        );
        let content = content_items(&library, None, &[]);
        let inks: Vec<(&Target, &str)> = content
            .iter()
            .filter(|item| item.title == "Load: ink")
            .map(|item| (&item.target, item.category))
            .collect();
        assert_eq!(
            inks,
            [
                (&Target::LoadScene(1), "Scene"),
                (&Target::LoadScene(2), "Synesthesia")
            ],
            "the library's own first, then Synesthesia's, told apart by category"
        );
    }

    #[test]
    fn scenes_rank_after_exact_command_matches_and_before_loose_ones() {
        let library = library(&["01-blackout-glow", "02-tempo-drift"]);
        let all = with_content(&library, None, &[]);
        let black = titles(&all, "black");
        let scene = black.iter().position(|t| t == "Load: 01-blackout-glow");
        assert!(
            scene.is_some_and(|at| at > 0
                && black
                    .iter()
                    .take(at)
                    .all(|t| t.to_lowercase().starts_with("black"))),
            "every command that starts with it comes first, then the scene; got {black:?}"
        );
        // Tap tempo has t, m, p, o in order, but only with a gap.
        let tempo = titles(&all, "tmpo");
        assert_eq!(
            tempo.first().map(String::as_str),
            Some("Load: 02-tempo-drift"),
            "a scene beats a loose command match; got {tempo:?}"
        );
        assert!(
            tempo.iter().any(|t| t == "Tap tempo"),
            "the loose match is still listed; got {tempo:?}"
        );
        let empty = titles(&all, "");
        assert_eq!(
            empty.iter().position(|t| t.starts_with("Load:")),
            Some(everything().len()),
            "with nothing typed, every command in table order, then the scenes"
        );
    }

    #[test]
    fn the_playing_scene_is_listed_but_cannot_be_loaded_again() {
        let library = library(&["01-aurora", "02-tunnel"]);
        let content = content_items(&library, Some(1), &[]);
        let reason = |target: Target| {
            content
                .iter()
                .find(|item| item.target == target)
                .map(|item| item.unavailable)
        };
        assert_eq!(
            reason(Target::LoadScene(1)),
            Some(Some(PLAYING_NOW)),
            "load"
        );
        assert_eq!(
            reason(Target::PreviewScene(1)),
            Some(Some(PLAYING_NOW)),
            "preview"
        );
        assert_eq!(reason(Target::LoadScene(0)), Some(None), "another scene");
        assert_eq!(content.len(), 4, "two entries a scene, no presets");
    }

    /// The title at `row` of the listing.
    fn title_at(listing: &Listing, row: Option<usize>) -> Option<&str> {
        row.and_then(|row| listing.ranked().get(row))
            .and_then(|index| listing.items().get(*index))
            .map(|item| item.title.as_str())
    }

    #[test]
    fn the_highlight_stays_on_its_entry_when_the_list_changes_under_it() {
        let bindings = Bindings::default();
        let layouts = std::collections::BTreeMap::<String, ()>::new();
        let before = library(&["01-aurora", "02-tunnel", "03-autumn"]);
        let none: Vec<String> = Vec::new();
        let sources = Sources {
            bindings: &bindings,
            availability: Availability::default(),
            library: &before,
            library_rev: 1,
            playing: Some(1),
            presets: &none,
        };
        let mut listing = Listing::default();
        let mut selected = Some(0);
        listing.update(sources, layouts.keys(), "autumn", &mut selected);
        // The operator moves the highlight down to Load: 03-autumn.
        let row = listing.ranked().iter().position(|index| {
            listing
                .items()
                .get(*index)
                .is_some_and(|item| item.title == "Load: 03-autumn")
        });
        assert!(row.is_some(), "the scene is listed");
        selected = row;
        // Then 03-autumn's neighbour starts playing (a cue, a phone), with
        // presets that match the query better, and a scene dropped into the
        // library ahead of it moves its index.
        let after = library(&["00-autumn-leaves", "01-aurora", "02-tunnel", "03-autumn"]);
        let presets = vec!["Autumn".to_string(), "Autumn dusk".to_string()];
        let changed = Sources {
            library: &after,
            library_rev: 2,
            playing: Some(1),
            presets: &presets,
            ..sources
        };
        let stale = selected;
        listing.update(changed, layouts.keys(), "autumn", &mut selected);
        assert_ne!(
            title_at(&listing, stale),
            Some("Load: 03-autumn"),
            "the old row names another entry now"
        );
        assert_eq!(
            title_at(&listing, selected),
            Some("Load: 03-autumn"),
            "the highlight followed its entry"
        );
        let target = selected
            .and_then(|row| listing.ranked().get(row))
            .and_then(|index| listing.items().get(*index))
            .map(|item| item.target.clone());
        assert_eq!(target, Some(Target::LoadScene(3)), "at its new index");

        // An entry that went away takes the highlight with it: Enter then
        // runs nothing rather than a neighbour.
        let gone = library(&["00-autumn-leaves", "01-aurora", "02-tunnel"]);
        listing.update(
            Sources {
                library: &gone,
                library_rev: 3,
                ..changed
            },
            layouts.keys(),
            "autumn",
            &mut selected,
        );
        assert_eq!(selected, None, "nothing highlighted");
    }

    #[test]
    fn a_listing_is_built_and_ranked_only_when_something_changed() {
        let library = library(&["01-aurora", "02-tunnel"]);
        let bindings = Bindings::default();
        let presets = vec!["Peak".to_string()];
        let layouts = std::collections::BTreeMap::from([("Show".to_string(), ())]);
        let sources = Sources {
            bindings: &bindings,
            availability: Availability::default(),
            library: &library,
            library_rev: 1,
            playing: Some(0),
            presets: &presets,
        };
        let mut listing = Listing::default();
        assert!(
            listing.update(sources, layouts.keys(), "tun", &mut None),
            "the first update ranks"
        );
        assert!(
            !listing.update(sources, layouts.keys(), "tun", &mut None),
            "the same sources and query: nothing to do"
        );
        let first = listing
            .ranked()
            .first()
            .and_then(|i| listing.items().get(*i))
            .map(|item| item.title.as_str());
        assert_eq!(first, Some("Load: 02-tunnel"), "ranked for the query");
        assert!(
            listing.update(sources, layouts.keys(), "aur", &mut None),
            "a new query ranks again"
        );
        let moved_on = Sources {
            library_rev: 2,
            ..sources
        };
        assert!(
            listing.update(moved_on, layouts.keys(), "aur", &mut None),
            "a changed library builds again"
        );
        let other_presets = vec!["Calm".to_string()];
        assert!(
            listing.update(
                Sources {
                    presets: &other_presets,
                    ..moved_on
                },
                layouts.keys(),
                "aur",
                &mut None
            ),
            "another scene's presets build again"
        );
    }
}
