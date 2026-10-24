# UI review (2026-10-23)

An evidence-based look at the control window as it is, not as
`UI_DESIGN.md` says it should be. That document has not kept up with the
code, so this one starts from what is actually on screen.

**How it was gathered.** Every pane captured from the release build with the
app's own `--screenshot` mode at 1600x1000 (`--open-pane <pane> --scene
01-aurora-gradient --audio synthetic`), plus the owner's own capture of
Settings at about 1765x1190. The source that draws each surface was read
alongside. External references are listed at the end.

The finding in one sentence: **the app has widgets but no layout grammar.**
Each pane stacks labels, controls and paragraphs of help text top to bottom
at nearly one type size, so every surface reads as a wall of text. Fixing
panes one at a time will not hold up; the missing pieces are shared ones.

---

## 1. Systemic problems

These recur on every pane. Each names its cause in the code.

### S1. No type hierarchy
Body text is 13.5 pt, small text 11.5 pt, and a section heading
(`widgets::section`) is **10.5 pt** uppercase in `text_muted`. The heading is
*smaller and fainter* than the labels under it. Pane titles
(`widgets::pane_header`) are `small().strong()` uppercase. `TextStyle::Heading`
(17 pt) exists and is barely used. The result: nothing on a page is visibly
more important than anything else. A settings heading should be clearly
larger and heavier than a row label (UX Movement's rule of thumb is several
sizes larger).

### S2. No label, description and control row
Settings, Map, Import and Playlist place a label, then the control on the
next line, then a grey paragraph under it. Sliders take the full pane width:
the render-scale track is about 1640 px in the owner's capture, with its
value box at the far edge. Help text is a loose `small()` paragraph instead
of a description attached to the setting it explains. Windows' own settings
guidance (and every modern settings UI) uses one row per setting: a header
and a one-line description on the left, the control right-aligned, rows
grouped in a card under a section heading.

### S3. Fields with no affordance, and file paths typed by hand
`TextEdit` fields render as a dark bar with no visible border, so empty
hint text (`a folder beside the library`, `this computer's LAN ...`) reads
like a value. Playlist (`playlists/example.toml` + Load), Import (`file
path, shader id or URL`), the watched folder and the venue trust file all
ask the operator to type a path. There is no file or folder picker anywhere
in the app (no `rfd` or similar dependency).

### S4. Status lines orphaned under controls
`Off` sits on its own line under the watched folder and again under Start
remote; `Device workers are not running in this mode` sits under MIDI input.
State belongs *in* the row: the toggle's position, or a status chip beside
the title.

### S5. The same control in several places
- **Show output / Fullscreen / Output monitor:** the Play view header, the
  side panel's icon row, the preview overlay (`Show output`), and Settings.
- **The live scene's controls:** the side panel's Controls tab, plus the
  macro strip under the transport.
- **Playlist and Edit:** both a rail destination and a side-panel tab.
- **Tap tempo:** in Settings > Audio, though it is a performance action.

Each duplicate costs space and makes the operator wonder whether the copies
differ.

### S6. Developer vocabulary on primary surfaces
The title bar permanently shows `native (OpenGL 3.3 core)`, `GPU measuring`,
`100% adaptive` and `Synthetic audio`. Elsewhere: `229/229, 150 from
Synesthesia`, `Output (meta): hide`, `master`, `Built-in /strobe/...
addresses`, `Device workers are not running in this mode`, `synthetic
(opening audio input...)`. These are diagnostics, and Diagnostics already
shows them properly in cards.

### S7. Navigation by abbreviation
The rail labels are `PLAY LIB GET LIST EDIT MAP DIAG SET` in about 9 pt
capitals under 16 px icons. `GET` (Import) and `LIST` (Playlist) are not
guessable. The labels are abbreviated to fit a 60 px rail; a wider rail with
real words, or icons with tooltips only, would both be clearer.

### S8. Panes that are mostly void
Import is one text field and one checkbox in a 1000 px pane. Playlist with
nothing loaded is a path field, a Generate button and a centred empty state.
Map stacks Connections, Activity and Macros as text lines above a large
empty list. Content has no maximum width, so on a wide window a short form
stretches edge to edge and the eye has to travel the whole screen.

### S9. Checkboxes that read as dots
Checkboxes are small grey rounded squares. Checked and unchecked differ by
a faint tick (compare `Adaptive resolution` with `Reduce motion` in the
owner's capture). Every one of these settings applies at once, which is
the case toggle switches exist for.

### S10. Settings is one undivided scroll
Output, Performance, Audio, Appearance, Synesthesia, Watched folder, Remote
(host), Control another Strobe (client), Layouts, the whole Keyboard
command table and Graphics are one column. There is no category navigation,
no search, and actions (Show output, Fullscreen, Tap tempo, Start remote,
Connect to desktop) sit among preferences.

### S11. Outright bugs seen in the captures
- **Banner text is right-aligned.** `widgets::banner` lays the message
  inside a `right_to_left` layout, so on a wide pane it is pushed against
  the buttons at the far right, away from its `WARNING` word (the owner's
  capture, Audio section).
- **Two reset icons that look alike.** Each inspector group has an undo
  arrow and a reload arrow at the same size, unlabelled.
- **Cut-off section.** The inspector ends on a `COLOR` heading with its
  first control clipped by the panel edge at the default window size.
- **Placeholder thumbnails.** Scenes without a picture get a flat tile with
  one large letter (`A`, `S`). Several in a row look like broken images.

---

## 2. Per-surface notes

| Surface | What works | What doesn't |
|---|---|---|
| Play (main) | Large preview, LIVE badge, clear empty state with one action | Three output buttons top right duplicate the side panel and Settings; macro strip is three narrow cards in a 1600 px strip |
| Title bar | Custom chrome, compact | Six diagnostic readouts at small size with low contrast; jargon (S6) |
| Library | Thumbnail grid, S/M/L sizes, playing badge | Count string reads like debug output; subtitles truncated mid-word; letter placeholders; filter row is five unrelated controls |
| Side panel | Preview + quick favourites | FAV 1-8 buttons with no content; four tabs, two of them duplicating rail destinations; `Output (meta)` and `master` jargon |
| Inspector | Grouped controls with counts | Full-width sliders, value box far from label; two look-alike reset icons; clipping |
| Playlist | Empty state says what to do | Typed file path; no picker; no recent playlists; pane is otherwise blank |
| Import | Does one job | Typed path or URL; void below; the userscript switch has an orphaned `Off` |
| Editor | Syntax colours, file tabs | Four buttons of equal weight (`Save and reload` vs `Save` vs `Revert` vs `Restore backup`); status line in small blue text |
| Map | Everything is there | Connections are a column of checkboxes and bare number fields; Activity and Macros are prose; the empty list is most of the pane |
| Diagnostics | The best pane: cards, key/value rows, status chips | Console fills the rest of the pane; `Needs work` is prose |
| Settings | Every option exists | All of S1-S10; the owner's capture is the clearest example |

Diagnostics is the closest to the target already. Its cards with key/value
rows and a status chip in the header are the pattern the rest should grow
from.

---

## 3. What good looks like (references)

- **Windows app settings guidelines** (Microsoft Learn). This is the
  platform Strobe ships on first. Single column with a max width of about
  1000-1100 px. Sections under a strong heading. One `SettingsCard` per
  setting: header, optional description, optional icon, control on the
  right. An expander for sub-options, never nested more than one level.
  Toggle switches for on/off. Changes apply immediately with no Apply button.
  Disabled settings say *why* in their description. Workflow commands do
  not belong in settings.
- **Settings-page practice** (UXCel, WooCommerce extension guidelines, the
  GitLab settings redesign issues). Group by what the operator is doing.
  Most-used first, destructive last. Headings scannable and noticeably
  larger. Avoid long flat lists. Generic labels on collapsed groups hide
  things. Search for large sets.
- **egui prior art.** Rerun's `re_ui` (the most polished egui design system
  in the open) centres on design tokens applied once to the context and a
  `list_item` module whose `PropertyContent` is a two-column property row
  shared by every panel. That is the same row-and-group primitive this app
  lacks, built in the same toolkit.
- **VJ software conventions** (Resolume, Synesthesia, VDMX). The performance
  surface is the output, the clip or scene grid, and the live parameters.
  Configuration lives in a separate Preferences window with category tabs
  (Resolume's include General, Video and OSC). Nothing about devices
  or the GL backend is on the performance surface unless something is wrong.

---

## 4. Direction

### Foundations first (one change, every pane benefits)
1. **Type scale.** Page title 20, section heading 15 semibold in `text`,
   row label 13.5, description 12 in `text_muted`, caption 11. Retire the
   10.5 pt uppercase heading.
2. **Setting row** (`widgets::setting_row`): title and an optional
   one-line description on the left; the control right-aligned in a fixed
   column (sliders about 240 px with the value beside them, combos about
   220 px); an optional status chip after the title; a disabled reason in
   the description.
3. **Group card** (`widgets::group`): a section heading over a card whose
   rows are separated by hairlines, the Diagnostics card generalised.
4. **Toggle switch** for every immediate on/off, replacing the checkbox.
5. **Text field style**: a visible border, a focus ring in the accent,
   placeholder text clearly fainter than a value.
6. **Content max width** for form-like panes (about 760 px, left-aligned
   with the pane inset), so a short form does not stretch across a
   projector-width window.
7. **Fix the banner** to lay its message out left to right.

### Then surface by surface, in this order
1. **Settings** (the owner's example). Category list on the left: Output,
   Performance, Audio, Appearance, Library, Remote, Shortcuts, About. Search
   across rows. Actions move out: Show output and Fullscreen stay on Play;
   Tap tempo goes to the transport; Remote's host and client halves become
   two cards with a clear on/off. Graphics and the build line move into
   About.
2. **Map**: Connections as setting rows (OSC, MIDI, feedback) with status
   chips; Activity as a live log card; Macros and Mappings as proper lists.
3. **Playlist and Import**: file and folder pickers (`rfd`, section 5),
   recent files, and drop targets; empty states sized to their
   content, not the pane.
4. **Inspector and side panel**: compact control rows (label, value, slider
   in one row at side-panel width); one labelled reset; remove the tabs
   that duplicate rail destinations; plain names for `Output (meta)` and
   `master`.
5. **Title bar**: fps and one health dot; everything else on hover, or
   shown when it degrades.
6. **Rail**: wider, with full words (section 5).
7. **Library**: a count that reads as a sentence, generated placeholder art
   instead of letters, a filter row in one segmented group.

Each step is checked the same way this review was made: a `--screenshot`
capture before and after, at 1600x1000 and at 1280x720.

---

## 5. Owner's answers (2026-10-23)

- **Rail:** wider, with full words (Play, Library, Import, Playlist,
  Editor, Mappings, Diagnostics, Settings).
- **Settings:** stays a page in the main view, with a category list on the
  left and search.
- **File pickers:** add `rfd` for native file and folder dialogs, with a
  Browse button beside each path field. Typed paths and drag-and-drop keep
  working. Log the dependency as a decision when it lands.
- **Density:** keep both Comfortable and Compact; every new primitive
  takes both.

## Sources

- Microsoft Learn, [Guidelines for app settings](https://learn.microsoft.com/en-us/windows/apps/design/app-settings/guidelines-for-app-settings)
- UXCel, [Settings best practices](https://uxcel.com/lessons/settings-best-practices-572)
- UX Movement, [Optimal page layout for settings](https://uxmovement.substack.com/p/optimal-page-layout-for-settings?s=r)
- WooCommerce, [Extension settings UX guidelines](https://developer.woocommerce.com/docs/extensions/ux-guidelines-extensions/settings/)
- GitLab, [Improve settings pages by prioritizing content](https://gitlab.com/gitlab-org/gitlab/-/issues/23483)
- Rerun, [`re_ui` crate docs](https://docs.rs/re_ui)
- Resolume, [Preferences (v6 manual)](https://resolume.com/support/en/6/preferences)
