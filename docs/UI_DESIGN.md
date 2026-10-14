# Strobe UI Design and Interaction Architecture

_Last researched: 2026-10-08. Sections 2, 4, 5, 7, 12 and 22 were rewritten on 2026-10-14 for the Synesthesia-style layout (D281 to D285)._

This is the UI contract for the Strobe desktop application. Synesthesia remains the compatibility reference for scene controls, presets, media, mapping and scripting. Since 2026-10-14 the owner's direction is also to follow its *layout* (three columns, a rail that switches the main view, one persistent right panel, a help bar), with Strobe's theme and every modern feature kept reachable. The target is a modern performance instrument for VJs and creative coders.

## 1. Product model

Strobe has two different jobs:

1. Perform: manipulate visuals while music is playing.
2. Build: browse scenes, edit shaders/scripts, arrange playlists, configure MIDI/OSC, and diagnose failures.

They share one visual language but use different workspace priorities.

The default screen must answer:

- What scene is playing?
- What is the output doing?
- What can I change immediately?
- What is the next cue?
- Is the system healthy?

Everything else should be contextual, collapsible, or behind another workspace.

## 2. Synesthesia: keep the behaviour, replace the topology

Synesthesia currently divides the control panel into Scene Controls, Meta Controls and Media. Scene controls are grouped with UI_GROUP, and common group actions include lock, default, random and undo. The current product also provides MIDI/OSC mapping, hotkeys, pane resizing, playlists, FavSlots and an integrated editor.

Keep:

- scene-specific controls;
- persistent meta controls;
- media controls;
- UI_GROUP semantics;
- presets;
- FavSlots concept;
- MIDI/OSC;
- playlists;
- scene library;
- editor.

Change:

- flat control walls become a contextual inspector;
- FavSlots become Performance Macros;
- library becomes visual/search-first;
- playlists become a dedicated timeline workspace;
- mappings become a Learn-first workflow;
- editor becomes a dedicated Build workspace;
- pane visibility becomes persistent workspace layout.

Synesthesia itself has been improving filtering, scene browsing, pane resizing and the scene builder, which validates these directions.

**Update (D281).** "Replace the topology" no longer holds for the window's frame. The owner asked for a layout "more similar to Synesthesia's while keeping our modern functionality", so the frame is Synesthesia's (sections 4, 5, 7 and 12) and what stays Strobe's is the content: the Performance Macros, the command palette, the Learn-first mapping workflow, Preview apart from Load, the health readouts, density and themes.

## 3. Core information hierarchy

The Perform workspace hierarchy is:

    OUTPUT
      ↓
    CURRENT ACTION
      ↓
    IMMEDIATE CONTROLS
      ↓
    NEXT ACTION
      ↓
    SYSTEM HEALTH
      ↓
    EVERYTHING ELSE

Do not give every control equal visual weight.

The UI should feel like a musical instrument with a modern control surface, not a settings application with a preview attached.

## 4. Workspace shell

Measured on Synesthesia 1.25.4 and matched (D281): three columns under the title bar, and a bar along the bottom.

    ┌─────────────────────────────────────────────────────────────────────┐
    │ title / view / health readouts / window controls                    │
    ├──┬────────────────────────────────────────────────┬─────────────────┤
    │  │ banners                                        │ preview (16:9)  │
    │r │                                                │ status strip    │
    │a │ main view: the library grid, or the view      │ [list][ctl][edit]│
    │i │ the rail chose; PLAY is the large preview     │ tab body        │
    │l │                                                │                 │
    ├──┴────────────────────────────────────────────────┴─────────────────┤
    │ transport: prev hold next / NOW NEXT / time bpm / BLACKOUT          │
    │ performance macros                                                  │
    ├─────────────────────────────────────────────────────────────────────┤
    │ help bar: what the control under the pointer does                   │
    └─────────────────────────────────────────────────────────────────────┘

Regions and their measure at 1920 wide:

- Rail, 60 points (3.1%): switches the main view. The active item is a filled tile. Labels drop below 1000 points.
- Main view, about 66%: one view at a time.
- Side panel, 31% (595): persistent across every view. Section 7.
- Bottom: the transport strip with BLACKOUT and the macros, then the help bar (24 points, D284), which is not drawn in a window under 520 points high.
- Output: separate window, almost entirely visual.

There is no Live/Edit mode. The rail switches the view and the panel never moves. Persist the panel's width (until it is dragged it follows the window; a double-click on its edge makes it follow again), its tab, and which view was open.

## 5. Navigation

The rail, top to bottom, each a main view (D281):

1. PLAY: the large live preview, with the output toolbar.
2. LIB: the scene library (the default view).
3. GET: Import, a Shadertoy shader.
4. LIST: the playlist.
5. EDIT: the scene's files.
6. MAP: MIDI and OSC.
7. DIAG: diagnostics.
8. SET (at the bottom): settings.

Clicking the shown item closes it, and Esc closes the topmost view; either reveals the one under it, and PLAY when none is left. Ctrl+K reaches every one of these, and every scene and preset, by name.

Expanded rail: icon + label. Below 1000 points: icon only. A short window scrolls the upper items and keeps SET.

Do not turn navigation into a row of permanent tabs consuming horizontal space.

## 6. Title bar and transport

Custom title bar follows the Rustaman model.

Left:

- Strobe mark;
- current workspace;
- dirty/unsaved state when relevant.

Right:

- audio status;
- FPS;
- render scale;
- GPU backend/timing;
- output target;
- window controls.

Perform transport:

- previous cue;
- play/hold;
- next cue;
- cue elapsed;
- next cue remaining;
- BPM;
- BPM confidence;
- transition state;
- blackout;
- master brightness.

Primary actions must look actionable. Numeric status must look like readouts.

## 7. The side panel and PLAY

The side panel is Synesthesia's right column (D283), top to bottom:

1. One 16:9 preview of the live scene, with the LIVE, BLACKOUT and output-hidden badges. No CPU readback. Left out when the panel is too short to give it 64 points at 38% of its height, so a drawer in a small window keeps room for the tab's body.
2. A status strip: Output (show or hide), Fullscreen, the frame rate with a dot, a horizontal audio level bar with the tempo, and Snapshot (keeps the live frame as the scene's picture) at the right edge.
3. The eight fav slots, in reach from every tab as Synesthesia keeps them. A panel under 360 points high moves them to the top of the Presets tab.
4. Icon tabs, with their names in the tooltips and beside the icons when the panel is wide:
   - Playlist: the loaded show's cues, double-click to jump;
   - Controls: the scene's controls by UI_GROUP, then the meta group;
   - Presets: the preset row and the scene's presets;
   - Edit: tags, picture, media, Generate controls and Open in Editor.

Closed, the panel is a handle strip. In a window too narrow to dock it (below the main view's 320 points plus the panel's 280) it is a drawer over the main view, below the banners, clipped to the surface.

PLAY, the rail's first item, is the large preview with the toolbar (Show output, Fullscreen, the monitor menu) for the moment the preview is the point. Bottom: the Performance Macros, the current and next cue, BLACKOUT.

Scene browsing must support Preview without replacing the active scene. Load is a separate action.

## 8. Performance Macros

This is the major UI improvement over Synesthesia.

A Performance Macro is a persistent performance-facing binding.

Possible targets:

- scene control;
- meta control;
- playlist action;
- transition parameter;
- audio-reactivity value;
- script-created uniform.

Each macro stores:

- name;
- target;
- source;
- kind;
- value;
- mapping state;
- optional range override;
- optional identity colour.

Source badges:

- SCENE
- META
- SCRIPT
- MIDI
- OSC
- PLAYLIST
- SYSTEM

If the current scene does not expose the target, the macro stays visible but becomes muted and says “unavailable in current scene”.

Never delete or reorder it just because a scene changed.

Suggested target resolution:

1. exact normalized name;
2. group + normalized name;
3. user-defined alias;
4. meta target;
5. playlist/system action.

Never guess silently.

## 9. Scene inspector

The inspector is hierarchical:

    SCENE
      name / creator / favorite / preset / pin

    CONTROLS
      Color
        Palette
        Hue
        Saturation
      Motion
        Speed
        Warp
      Audio
        Bass React
        Hits

    META
      Brightness
      Contrast
      Hue

    MEDIA
      Source
      Mix

    TRANSITIONS
      Direction
      Smoothness

The taxonomy comes from ControlDef.group. Do not create a second scene-control taxonomy in UI code.

## 10. Control widgets

Control rendering maps directly from ControlKind.

### Slider

- label;
- current value;
- horizontal rail;
- exact numeric interaction;
- Shift fine drag;
- middle-click reset;
- double-click reset;
- context menu.

These preserve Synesthesia's established modifier conventions.

### Knob

Use sparingly. Circular controls should communicate actual rotary/modulation behaviour.

### Toggle

Compact switch + label.

### Bang

Momentary button with clear pressed state.

### XY

Square field with:

- marker;
- precise drag;
- keyboard nudging;
- numeric values on focus/hover;
- reset.

### Color

Small swatch with popup picker, not a permanently large editor.

### Dropdown

Compact select, searchable for long option lists.

## 11. Control groups

UI_GROUP is the primary scene grouping key.

Group header:

- name;
- collapse/expand;
- lock;
- default;
- random;
- optional undo;
- overflow menu.

Do not create a four-button toolbar beside every control.

Descriptions remain hidden until hover/focus/help mode.

## 12. Scene library

The library is a creative asset browser, laid out as Synesthesia's is (D282):

    ┌─────────────────────────────────────────────────────────────────┐
    │ [+ New scene] [refresh]                              [S][M][L] │
    │ 88/88 [ search names, authors, tags ] [Sort v][^] [*] [Show v] │
    ├─────────────────────────────────────────────────────────────────┤
    │ thumbnail cards, as many columns as fit                         │
    │ ┌──────────┐ ┌──────────┐                                       │
    │ │ picture *│ │ picture  │   picture: state chip, star           │
    │ ├──────────┤ ├──────────┤                                       │
    │ │ Title  ..│ │ Title  ..│   caption: title, "..." menu,         │
    │ │ author   │ │ author   │   author (or why it failed)           │
    │ └──────────┘ └──────────┘                                       │
    └─────────────────────────────────────────────────────────────────┘

Toolbar: New scene (opens Import), rescan, and the small, medium or large thumbnail switch (saved).

Filter row: the count, a wide search, the sort (Library order, Name, Author, Favourites first) with its direction, a favourites-only star, and the health filter (All, Ready, Needs work, Favourites). Below 560 points the search keeps a row with the count and the rest take the next.

Cards:

- thumbnail, or the scene's identity colour and initial;
- a state chip over it: PLAYING, PREPARING, PREVIEW, FAILED;
- the star, over its corner;
- the title, in the largest text on the card;
- the "..." menu (Load, Preview without loading, favourite, Open in Editor, Copy to my library on a read-only scene), which is also the right-click menu;
- the author, else the tags; a check before it once the scene loaded; `Failed: <why>` in its place when it did not;
- a faint "Synesthesia" at the end of that line for a scene from Synesthesia's install or its scene folders (D271).

The selected card shows Preview and Load over its picture; double-click loads. The selected scene's tags sit above the grid.

Global search matches every word against the title, the author and the tags; the word `synesthesia` finds the scenes from its install. `tag:` and `author:` prefixes are not parsed yet.

This follows the modern content-browser pattern used by Ableton Live 12, and Synesthesia's own library.

## 13. Playlist workspace

Dedicated timeline/cue workspace:

    ┌─────────────────────────────────────────────────────────────────┐
    │ playlist / duration / cues / generate / save                   │
    ├──────────────┬──────────────────────────────────────────────────┤
    │ sections     │ cue list / timeline                             │
    │ Intro        │ Aurora  00:00  00:30  crossfade                │
    │ Main         │ Plasma  00:30  01:00  beat                     │
    │ Peak         │ Tunnel  01:00  01:30  cut                      │
    ├──────────────┴──────────────────────────────────────────────────┤
    │ selected cue inspector                                         │
    └─────────────────────────────────────────────────────────────────┘

Inheritance is visible:

- inherited values are muted;
- overridden values are emphasized;
- reset-to-inherited is direct;
- source of inheritance is inspectable.

Generator exposes:

- target duration;
- scene pool;
- tags;
- no-repeat window;
- duration rule;
- transition rule;
- seed.

Generated result must be previewable before commit.

## 14. Editor workspace

Dedicated Build workspace:

    ┌───────────────────────────┬──────────────────────────────────────┐
    │ GLSL / JS / JSON         │ live preview                         │
    ├───────────────────────────┴──────────────────────────────────────┤
    │ compiler / console / diagnostics                                │
    └─────────────────────────────────────────────────────────────────┘

Source tabs:

- main.glsl;
- scene.json;
- script.js;
- assets.

Compile states:

- Compiling;
- Ready;
- Stale;
- Error.

The renderer already maps compiler logs back to main.glsl line numbers. Display those directly.

Auto-compile is optional and debounced. Manual compile is the safe default for old hardware.

## 15. Mappings workspace

Do not make the old mapping table the primary interaction.

Learn-first workflow:

1. Click Learn.
2. Move a MIDI control or send OSC.
3. Strobe captures it.
4. Choose target.
5. Save.

Inspector fields:

- device;
- channel/address;
- message type;
- target;
- min/max;
- threshold;
- invert;
- learn state.

Search mappings by:

- control;
- device;
- MIDI channel;
- OSC address;
- scene;
- action.

Resolume's shortcut/effect/dashboard approach supports the same philosophy: surface repeated actions and parameters rather than forcing manual traversal for common operations.

## 16. Diagnostics workspace

Top cards:

- GL backend;
- GPU;
- output;
- audio;
- script;
- playlist;
- dropped frames;
- render scale;
- shader failures.

Below:

- event log;
- shader compiler errors;
- audio events;
- import failures;
- cue/runtime failures.

Every event includes:

- subsystem;
- subject;
- phase;
- root message;
- recovery action.

Example:

    AUDIO
    Default Output disconnected
    Retrying in 500 ms

    SHADER
    09-sdf-cathedral
    main.glsl:142
    unknown identifier
    [Open in Editor]

## 17. Output window

Output is almost entirely visual.

Default:

- no chrome;
- black background;
- direct scene output;
- aspect-preserving presentation;
- optional debug overlay.

A hidden output window must never steal focus from the control window.

## 18. Theme system

Reuse the Rustaman theme strategy.

Authored semantic colours:

- app;
- panel;
- raised;
- hover;
- border;
- accent;
- text;
- muted text;
- danger;
- warning;
- success;
- info;
- control.

Derive other colours.

Default theme (the brand palette, `docs/branding.md` section 3; D190):

- the brand's Void / Panel / Border surfaces, a very dark blue-black;
- the brand's Cyan as the one accent, for live, selected and focused only;
- pale primary text (brand Text);
- blue-grey secondary text (brand Muted);
- semantic status colours; information in blue, never cyan.

The original amber default is kept as the `amber` theme.

The UI itself should not become rainbow. Scene content and data visualizations can be.

Theme requirements:

- primary text strongly readable;
- secondary text readable;
- selected rows readable;
- control rail separated from background;
- status colours distinguishable without relying on hue alone.

## 19. Scene identity colour

A stable scene identity colour may be derived from scene metadata.

Use it for:

- thumbnail frame;
- cue marker;
- small badge;
- optional macro accent.

Do not recolour whole panes or body text.

## 20. Motion system

Reuse Rustaman's central motion vocabulary:

- INSTANT 120 ms: hover/press;
- QUICK 180 ms: selection/disclosure;
- SETTLE 350 ms: machine-driven values;
- ENTER 220 ms: view changes.

Rules:

- centralize timings;
- never invent per-widget durations;
- no full-page entrance animation;
- no animation merely because something can animate;
- animate state changes that benefit from continuity.

Respect reduced-motion settings by removing overshoot and minimizing transitions.

## 21. Density

Perform density:

- compact;
- high signal;
- minimal descriptions;
- strong action affordances.

Edit density:

- more context;
- descriptions available;
- more metadata.

Density is a workspace setting, not a property each widget invents independently.

## 22. Responsive desktop behaviour

Primary target sizes:

- 1920×1080;
- 1440×900;
- 1280×720.

At those sizes the three columns and the bottom bars all show. The side panel follows the window at 31% (595, 446, 397 points).

Below that, parts give way in a fixed order (D260, D281, D285), and never BLACKOUT, the transport buttons or the window controls:

1. the help bar goes below 520 points high;
2. the rail drops its labels below 1000 points wide;
3. the side panel shrinks towards 280 points while the main view keeps 320, then becomes a drawer over the main view, below the banners and clipped to the surface;
4. the library's filter row becomes two rows below 560 points, and its toolbar button an icon below 300;
5. banners take at most half the surface and scroll, and wrap their buttons;
6. the title bar drops the breadcrumb below 760 points and the wordmark below 420; the transport drops BPM, then the clock, then NOW.

The shell tests draw every state at the three above, at 1366×768, 900×600 and 640×480, and at the T480's four logical sizes (683×427, 533×400, 427×320, 320×533).

Output window is independent.

## 23. Persistence

Persist:

- workspace;
- pane visibility;
- pane widths;
- inspector section;
- library filters;
- theme;
- density;
- output assignment;
- macro order;
- editor split;
- playlist split.

Do not persist:

- hover;
- context menus;
- temporary error dialogs;
- ephemeral learn state.

Named layouts:

- Perform;
- Edit;
- Playlist;
- Mapping;
- Debug.

## 24. Keyboard model

Global:

- Ctrl/Cmd+K: command palette;
- Space: play/hold when transport owns focus;
- Left/Right: previous/next cue;
- F: focus preview;
- B: blackout;
- Esc: close transient state.

Focused control:

- arrows: fine;
- Shift+arrows: larger;
- Enter: exact edit;
- R: reset;
- M: MIDI learn in mapping context.

Preserve compatible Synesthesia hotkeys where practical, but use focus rather than “select a control first” as the primary keyboard model.

## 25. Command palette

Ctrl/Cmd+K is the power-user escape hatch.

Commands:

- Load scene;
- Preview scene;
- Next/Previous cue;
- Hold/Resume;
- Blackout;
- Reset;
- Randomize;
- Save/Load preset;
- Toggle library;
- Toggle inspector;
- MIDI Learn;
- Diagnostics;
- Settings;
- Editor;
- Recompile;
- Shadertoy import;
- Generate playlist.

Each result shows command, shortcut, state and scope.

## 26. Loading and failure states

Scene loading shows:

- scene name;
- compile progress;
- pass count;
- current phase.

Failed scene card shows:

- FAILED;
- short reason;
- Diagnostics;
- Editor;
- Retry.

Missing asset shows:

- filename;
- affected scene/pass;
- Locate;
- Ignore;
- Open folder.

Import failures stay in Needs Work with acquisition/conversion phase.

## 27. Accessibility

Requirements:

- keyboard access to major actions;
- visible focus;
- no colour-only state;
- status text paired with icons;
- readable numeric values;
- reduced motion;
- high-contrast theme.

High contrast should change surface/text relationships rather than simply increasing saturation.

## 28. UI architecture

strobe-app should split UI code roughly into:

    app/
      state/
      actions/
      workspaces/
      chrome/
      controls/
      library/
      playlist/
      mappings/
      editor/
      diagnostics/
      settings/

The UI consumes application snapshots and emits actions.

It must not:

- call CPAL directly;
- call QuickJS directly;
- scan the filesystem;
- make network requests;
- mutate ScenePlayer internals directly.

Introduce an action layer for operations such as:

- LoadScene;
- PreviewScene;
- NextCue;
- PreviousCue;
- SetControl;
- ResetControl;
- RandomizeGroup;
- PinMacro;
- SavePreset;
- LoadPreset;
- MapMidi;
- MapOsc;
- CompileScene;
- OpenEditor;
- SetOutput;
- SetTheme;
- SetWorkspace.

Immediate-mode UI creates actions; application state executes them.

## 29. Data-model boundaries

The current ControlDef already supplies:

- name;
- kind;
- min/max/default;
- params;
- group;
- description;
- labels/values.

That is enough for the first UI.

Do not put UI layout state into Synesthesia-compatible scene JSON.

Persistent Strobe UI state belongs in Strobe configuration/state:

- workspaces;
- macros;
- favorites;
- local tags;
- theme;
- pane layouts.

Use the planned X_STROBE namespace only for deliberately scene-local Strobe extensions.

## 30. Performance rules

Never perform these in the egui paint path:

- filesystem scans;
- shader compilation;
- image decoding;
- audio analysis;
- network requests;
- playlist generation;
- GPU readback.

Use worker/preparation paths and send compact state/messages to the UI thread.

Cache:

- thumbnails;
- metadata;
- search indexes;
- static labels;
- syntax/editor state;
- GPU preview resources.

Virtualize large lists.

## 31. Preview resource rule

The renderer already exposes the output texture.

The control UI should render that GPU resource directly.

Never use:

    ScenePlayer -> read_output -> CPU pixels -> UI image

every frame.

That would directly violate the old-hardware performance goal.

## 32. Testing

UI component tests:

- minimum size;
- keyboard interaction;
- NaN/non-finite values;
- hover/focus/selected states;
- dark theme;
- high-contrast theme.

Visual regression fixtures:

- Perform;
- Library;
- Playlist;
- Editor;
- Mappings;
- Diagnostics;
- Settings;
- collapsed rail;
- 1280×720;
- 1440×900;
- 1920×1080;
- high-contrast.

Interaction regression:

- command palette;
- preview without load;
- load scene;
- macro pin/unpin;
- unavailable macro;
- preset save/recall;
- playlist selection;
- MIDI Learn;
- blackout;
- output selection;
- workspace persistence.

## 33. Performance regression

Measure:

- UI frame time;
- thumbnail decode spikes;
- library search latency;
- editor typing latency;
- preview overhead;
- UI allocations.

Set production thresholds after measurements on the owner's T480 and at least one modern GPU.

A UI change that increases render-thread instability is a regression.

## 34. Build order

### UI-01: shell
Custom chrome, rail, workspace state, two-window behavior, theme/motion.

### UI-02: preview
Direct GPU preview, output routing, transport and health strip.

### UI-03: controls
All current ControlKind variants, groups, search, reset/random/lock.

### UI-04: Performance Macros
Pinning, semantic target resolution, unavailable-state behavior.

### UI-05: library
Indexing, thumbnails, filters, preview/load distinction, favorites.

### UI-06: playlists
Cue list, timeline, inheritance inspector, generator.

### UI-07: mappings
Learn flow, search, device state.

### UI-08: editor
Source tabs, preview, diagnostics, controlled compile.

### UI-09: diagnostics/settings
Structured log, outputs, audio, theme/layout.

### UI-10: polish
Motion, focus, keyboard navigation, visual regression, high-DPI, reduced motion.

## 35. Perform acceptance bar

At 1440×900, a user must be able to:

- identify the active scene immediately;
- see the visual output immediately;
- find the next cue without opening Playlist;
- change a common scene control without a modal;
- use Performance Macros without changing panes;
- trigger next/previous/hold/blackout from keyboard;
- see audio/BPM/output health;
- open a shader error directly in Editor;
- hide library and inspector for a larger preview;
- restore the same workspace after restart.

## 36. Anti-patterns

Reject:

- permanent giant control sidebar;
- giant rows of tiny icon buttons;
- modal dialogs for normal parameter adjustment;
- CPU preview readback;
- rainbow UI chrome;
- colour-only status;
- every control becoming a card;
- every pane permanently visible;
- arbitrary spacing values;
- arbitrary animation durations;
- per-widget ad-hoc colours;
- whole-page entrance animations;
- disappearing controls on scene change;
- destructive library browsing;
- modal-only scene preview;
- hidden playlist inheritance;
- manual mapping before Learn;
- developer diagnostics mixed permanently into Perform.

## 37. Research sources

Synesthesia:
- https://app.synesthesia.live/docs/manual/controls.html
- https://app.synesthesia.live/docs/manual/control_hotkeys.html
- https://app.synesthesia.live/docs/tutorials/presets_favslots_playlists.html
- https://getsynesthesia.com/docs/manual/scene_editor.html
- https://www.getsynesthesia.com/docs/ssf/json.html
- https://vjun.io/vdmo/synesthesia-120-24bm
- https://getsynesthesia.com/features

Resolume:
- https://resolume.com/support/en/composition
- https://resolume.com/support/en/dashboard
- https://resolume.com/support/en/layers
- https://resolume.com/support/effects
- https://resolume.com/support/en/layouts
- https://resolume.com/support/en/vocabulary

Ableton:
- https://www.ableton.com/en/release-notes/live-12/
- https://help.ableton.com/hc/en-us/articles/12927340213660-The-Live-12-Browser
- https://www.ableton.com/en/live-manual/12/working-with-the-browser/
- https://www.ableton.com/en/live/learn-live/interface/

TouchDesigner:
- https://derivative.ca/UserGuide/Parameter_Dialog

OBS:
- https://obsproject.com/kb/sources-guide

Rustaman:
- https://github.com/lnorton89/rustaman/tree/main/src/gui
- https://github.com/lnorton89/rustaman/tree/main/src/theme
- https://github.com/lnorton89/rustaman/blob/main/assets/themes.toml
- https://github.com/lnorton89/rustaman/blob/main/src/motion.rs

## 38. UI source-of-truth rules

1. Synesthesia documentation defines compatibility behaviour, not Strobe's visual design.
2. ControlDef defines available scene control data.
3. ControlBank owns control behaviour/state.
4. strobe-gfx owns rendering and the live output texture.
5. strobe-app owns interaction, layout and presentation.
6. Rustaman supplies design-system patterns unless explicitly replaced.
7. Persistent Strobe UI state belongs in Strobe config/state, not compatible scene files.
8. Perform is the primary product surface and receives the most polish/testing.
9. Features that slow the live renderer are regressions.
10. Every UI surface requires deliberate loading, empty and error states.
