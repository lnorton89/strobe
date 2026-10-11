# Strobe UI Implementation Plan

_Last researched: 2026-10-08_

This is the implementation handoff for the future `strobe-app` UI. It turns the research in `UI_DESIGN.md` and `UI_RESEARCH_ADDENDUM.md` into a concrete egui architecture without prematurely coupling the UI to renderer internals.

## Implementation status (2026-10-10, stage 1)

Built in `strobe-app` as D130 to D138, with screenshots in
[images/perform/](images/perform/). Section by section:

| Section | State |
|---|---|
| 1 to 3 Topology and layers | **Done.** Contextual Perform; `UiState` is presentation state, `Model` the adapter, `Action` the command layer. The module layout differs from section 4 (see `ARCHITECTURE.md`). |
| 5 Shell, 6 panes, 7 layout | **Done.** `ui::shell`, `workspace` (pane data, docks, collapse order, drawer). 55% preview share instead of fixed breakpoints (D131). |
| 8 Browser (Preview, Load) | **Done**; Queue is not. Preview is a second small player (D136). |
| 9 Scene health | **Partly.** Unknown, Ready and Failed (with the first error lines), filter by health. Preparing is shown on the row; MissingAsset, Unsupported, HighGpu, Degraded are not detected. |
| 10 Inspector | **Done** for every `ControlKind`, with Shift fine, reset, exact entry, keyboard; not randomise-per-control, lock or undo. |
| 11, 12 Macros | **Stage 2 built** (D175): a curated, pinned, ordered, saved set, resolution by name, group/name and alias, the six states on each card, MIDI/OSC Learn, a context menu. Not built: a range and colour editor, smoothing and a shortcut per macro. With no macros the strip falls back to stage 1. |
| 13 Transport | **Partly.** Prev, next and blackout work; Hold, cue time and BPM-confidence are not (no cue list; D135). |
| 14 Diagnostics | **Levels 1 and 2** (readouts; notices, banners, per-subsystem cards, the failed-scene list) and a console; no dedicated event model with recovery actions. |
| 15 Commands | **Table and shortcuts**, with stable ids that MIDI, OSC and macros share, and hotkeys configurable by file (D178); no palette. |
| 16 Focus | **Done** (D134). |
| 17 Persistence | **Done** for panes, widths, query, filter, collapsed groups, theme, motion, title bar (D133), and macro order, mappings, fav slots and hotkeys in `show-control.json` (D179); not named layouts or density. |
| 18 Paint cost | Rules enforced by tests and `shell_tests`; the T480 measurement is open. |
| 19, 20 Output window | Unchanged from D96; show, hide, fullscreen and monitor are in the toolbar and Settings. |
| 21 Loading and failure | **Done** (two-phase load, banners); "Open Editor" waits for the Editor. |
| 22 to 26 Acceptance | Perform met on Xvfb (STATUS_AUDIT); playlist, editor, mapping, library thumbnails and tags are later stages. |

## 1. Executive decision

Use **Contextual Perform** as the default topology.

Persistent:

- navigation rail
- live preview
- compact health/readout strip
- transport/cue state
- Performance Macros

Contextual:

- scene browser
- control inspector
- diagnostics detail
- mapping detail
- playlist detail

The browser and inspector are not separate applications. They are panes that can be opened, resized, focused, pinned, and hidden while the Perform work surface remains stable.

The key design constraint is:

> Opening more information should change the amount of information visible, not change what the application considers the primary task.

The primary task in Perform is always operating the current visual output.

## 2. Why this is the recommended topology

Research across Synesthesia, Resolume, Ableton Live 12, TouchDesigner-style parameter workflows, and egui points toward a common pattern:

- expose high-frequency parameters directly;
- keep deep configuration nearby but not permanently visible;
- make browsers searchable/filterable and hideable;
- preserve workspace state;
- use dedicated authoring surfaces;
- avoid forcing every subsystem into one dashboard.

Synesthesia validates grouped scene controls and explicit control actions.

Resolume's Dashboard validates a curated set of high-frequency parameters instead of requiring the operator to traverse the full effect stack.

Ableton Live 12 validates hideable browsers, search history, filters, tags, saved searches, adjustable widths and keyboard-first navigation.

egui's own documentation makes the performance implication unusually explicit: immediate-mode layout is performed each frame, large scroll regions can become expensive, and child immediate viewports can multiply CPU work.

Therefore Strobe should not make "show everything" the default.

## 3. UI architecture

Use five layers.

### Layer A: product state

Owned by the application, independent of egui:

- current workspace;
- active scene;
- preview scene;
- playlist state;
- cue state;
- macro bindings;
- mapping state;
- diagnostics;
- output state;
- renderer health;
- audio health;
- script health.

### Layer B: presentation state

Owned by `strobe-app`:

- pane visibility;
- pane widths;
- selected inspector group;
- selected scene/library item;
- search query;
- filters;
- sort order;
- density;
- theme;
- focused widget;
- command palette state;
- notification state;
- editor splits.

### Layer C: domain adapters

Thin adapters convert domain state into UI-safe snapshots:

- `SceneSnapshot`
- `ControlSnapshot`
- `AudioSnapshot`
- `RenderHealthSnapshot`
- `CueSnapshot`
- `MacroSnapshot`
- `DiagnosticSnapshot`

The UI should not reach through several crates to mutate renderer state.

### Layer D: commands

UI actions become commands:

- LoadScene
- PreviewScene
- NextCue
- PreviousCue
- SetControl
- ResetControl
- RandomizeGroup
- ToggleMacro
- TriggerBang
- SetBlackout
- SetWorkspace
- TogglePane
- LearnMapping
- CompileScene
- OpenDiagnostic
- RetrySubsystem

Commands are the only intended write path from UI into runtime state.

### Layer E: rendering

The preview/output renderer consumes the same authoritative runtime state as the rest of the application.

Never create a second fake control model for the UI.

## 4. Suggested module layout

Keep UI organization by responsibility rather than widget type.

Suggested future structure:

```text
crates/strobe-app/src/
  app/
    mod.rs
    state.rs
    commands.rs
    snapshots.rs
  ui/
    mod.rs
    shell.rs
    navigation.rs
    perform.rs
    library.rs
    playlist.rs
    editor.rs
    mappings.rs
    diagnostics.rs
    settings.rs
    inspector.rs
    macros.rs
    transport.rs
    output.rs
    command_palette.rs
    notifications.rs
    theme.rs
    layout.rs
  input/
    keyboard.rs
    mouse.rs
    midi.rs
    osc.rs
  runtime/
    mod.rs
    scene.rs
    playlist.rs
    audio.rs
    script.rs
    output.rs
```

The exact filenames can change. The separation of concerns should not.

## 5. Shell contract

The shell owns:

- navigation;
- title/status area;
- workspace routing;
- global command palette;
- notifications;
- output-window state.

The shell does not own scene controls.

A workspace receives a stable content rectangle and renders inside it.

This makes workspace changes testable without requiring a live GL scene.

## 6. Pane model

Represent panes as data, not hard-coded branches.

Each pane has:

- ID;
- visibility;
- preferred width;
- minimum width;
- maximum width;
- docking region;
- persistence key;
- collapse priority;
- workspace availability.

Recommended panes:

- SceneBrowser
- Inspector
- CueInspector
- Diagnostics
- MappingInspector
- MacroEditor
- Help

### Collapse priorities

When width becomes constrained:

1. hide secondary metadata;
2. collapse browser;
3. collapse inspector;
4. reduce navigation labels;
5. preserve preview;
6. preserve transport;
7. preserve blackout.

This is more robust than maintaining separate layout code for every resolution.

## 7. Layout rules

At 1920×1080:

```text
rail | browser(optional) | preview | inspector(optional)
--------------------------------------------------------
transport + macros
```

At 1440×900:

```text
rail | preview | inspector(optional)
------------------------------------
transport + macros
```

Browser is normally closed until requested.

At 1280×720:

```text
rail | preview | contextual drawer
---------------------------------
essential transport/macros
```

Browser and inspector should not both be forced open.

At smaller sizes:

- rail labels disappear;
- inspector becomes a drawer/overlay;
- browser becomes a full workspace;
- health becomes compact status indicators.

## 8. Scene browser interaction model

Opening the browser must not stop the show.

Three actions:

### Preview

Compiles/prepares the selected scene and renders it into a non-live preview target.

It must not:

- change the active scene;
- alter active macros;
- change playlist state;
- change output;
- change cue position.

### Load

Commits the selected scene.

Loading should:

1. validate/preflight;
2. compile/prepare;
3. resolve controls;
4. resolve macros;
5. update active scene;
6. schedule transition;
7. emit an event.

If preparation fails, the old scene remains live.

### Queue

Optional future action that adds the scene to the next cue without immediately loading it.

The distinction should be visually obvious.

## 9. Scene health model

A scene card needs a compact health state.

Suggested states:

- Ready
- Preparing
- CompileError
- MissingAsset
- Unsupported
- ScriptError
- HighGpu
- Degraded
- Unknown

Do not encode health using colour alone.

Every non-ready state needs:

- icon;
- short label;
- tooltip/detail;
- recovery action where available.

The browser should support filtering by health.

## 10. Inspector model

The inspector is generated from `ControlDef`.

Never duplicate control metadata in UI code.

Each control gets:

- stable control identity;
- display label;
- group;
- kind;
- range;
- default;
- current value;
- modifiers;
- description;
- availability.

### Interaction rules

Slider:

- click-drag;
- relative dragging where possible;
- Shift for precision;
- numeric field for exact values;
- reset action;
- keyboard adjustment.

Synesthesia documents Shift-drag precision and reset interactions; Resolume also exposes direct numeric entry and parameter manipulation. These are good compatibility expectations.

Toggle:

- click;
- Space when focused;
- explicit on/off appearance.

Bang:

- momentary activation;
- visible pressed feedback;
- keyboard trigger when focused.

Dropdown:

- keyboard searchable when large;
- current value always visible.

XY:

- pointer;
- keyboard nudge;
- numeric readout;
- reset.

Color:

- compact swatch;
- popup picker;
- keyboard focus;
- optional numeric channels.

## 11. Performance Macro model

Macros are not copies of controls.

They are stable operator bindings.

Each macro contains:

```text
MacroId
DisplayName
Target
Source
Kind
Range
Curve
Smoothing
Shortcut
Identity
```

Target resolution is explicit:

1. exact stable control ID;
2. normalized group + control;
3. user alias;
4. global/meta target;
5. playlist/system action.

Never fuzzy-match silently.

### Runtime states

- Resolved
- Unavailable
- Broken
- Disabled
- Learning
- Conflict

The operator can see that a macro is unavailable without losing the macro configuration.

## 12. Macro interaction

A macro should support:

- click-drag;
- numeric entry;
- reset;
- source indicator;
- target inspector;
- keyboard shortcut;
- MIDI/OSC learn;
- context menu.

Avoid making every macro a large knob.

Use sliders/buttons by semantic type.

Circular controls are reserved for parameters where rotational manipulation is genuinely useful.

## 13. Transport model

Persistent bottom strip:

```text
PREV | PLAY/HOLD | NEXT | NOW | NEXT | TIME | BPM | TRANSITION | BLACKOUT
```

Do not put detailed playlist editing here.

The strip is for answering:

- what is happening;
- what happens next;
- how much time remains;
- what immediate action is available.

Blackout should be visually distinct and require an intentional action.

## 14. Diagnostics model

Three levels.

### Level 1: status

Always visible:

- renderer;
- audio;
- script;
- output;
- FPS.

### Level 2: event

Transient:

- shader compiled;
- scene loaded;
- audio device changed;
- asset missing;
- cue triggered.

### Level 3: investigation

Dedicated Diagnostics workspace with:

- subsystem;
- subject;
- phase;
- error;
- recovery;
- timestamp;
- related scene/cue;
- action.

Avoid modal error dialogs during performance.

## 15. Command system

Implement commands before building many keyboard shortcuts.

A command has:

- ID;
- display name;
- category;
- default shortcut;
- enabled predicate;
- action;
- optional confirmation policy.

Command palette categories:

- Navigation
- Scene
- Playback
- Control
- Macro
- Playlist
- Mapping
- Diagnostics
- Window

This allows keyboard shortcuts, menus, command palette and future MIDI/OSC actions to converge on the same action model.

## 16. Focus model

Focus is critical in a VJ application because keyboard input can otherwise affect the wrong subsystem.

Define:

- Global focus;
- Workspace focus;
- Pane focus;
- Control focus;
- Text-edit focus.

Rules:

- text fields capture normal typing;
- transport shortcuts are disabled while editing text;
- Escape leaves transient/text focus before closing larger surfaces;
- command palette captures keyboard until dismissed;
- blackout remains available only according to an explicit global shortcut policy.

Never make a shortcut accidentally alter a shader value because a text editor happened to be active.

## 17. Persistence

Persist application preferences separately from runtime state.

Persistent:

- workspace;
- pane visibility;
- pane widths;
- selected workspace;
- library filters;
- sort;
- macro order;
- keyboard bindings;
- output assignment;
- editor splits;
- theme;
- density.

Ephemeral:

- hover;
- open context menu;
- drag operation;
- current learn transaction;
- transient notification;
- current pointer position.

egui already retains widget state by IDs and can persist its Memory, but important product state should remain in Strobe's own model rather than hiding it inside egui Memory.

## 18. egui performance architecture

The renderer is the performance-critical subsystem. UI work must not become a second render engine.

Rules:

- no scene thumbnail rendering in the frame loop;
- no filesystem scanning during UI paint;
- no shader compilation during UI paint;
- no blocking device enumeration during UI paint;
- no large unbounded logs;
- no giant always-expanded control trees;
- no per-frame allocation in high-frequency UI paths where avoidable;
- cache expensive derived data;
- perform expensive work asynchronously and return snapshots.

egui's documentation notes that full layout happens each frame and that very large scroll areas can be expensive. Its cache system is intended for work that is too expensive to recompute every frame, while very slow work should move to another thread.

Use bounded views and explicit caches.

## 19. Multiple-window architecture

Strobe needs a control window and a visual output window.

Do not put the live output inside an egui widget if doing so creates unnecessary readback or texture synchronization.

The output window should remain visual-only.

If egui is used for native viewports, prefer deferred viewports for independently repainting windows. egui explicitly warns that immediate child viewports can cause parent/child repaint coupling and multiply CPU work.

The existing Strobe architecture's one-GL-context/two-surface plan remains the preferred renderer-level design.

## 20. Output window rules

Output window:

- borderless/fullscreen capable;
- selectable monitor;
- aspect-preserving;
- black background;
- no UI by default;
- optional debug overlay;
- no focus stealing;
- recoverable if monitor disappears.

The control window owns the output assignment.

Changing output assignment must not destroy the active scene.

## 21. Loading and failure UX

Every async operation has four states:

- idle;
- working;
- success;
- failure.

Never leave the user with a spinner and no explanation.

Examples:

### Scene load

```text
Preparing Aurora Field…
Compiling 4 passes…
Ready
```

### Shader failure

```text
Aurora Field
Compile failed

main.glsl:142
unknown identifier: foo

[Open Editor] [Keep Current Scene]
```

### Audio device

```text
Audio input unavailable
Visuals continue without audio reaction

[Retry] [Choose Device]
```

The show should degrade gracefully whenever possible.

## 22. Mockup acceptance tests

The next HTML prototype must prove these interactions:

### Perform

- open/close browser;
- open/close inspector;
- resize panes;
- select scene;
- preview scene;
- load scene;
- switch cue;
- adjust control;
- reset control;
- trigger macro;
- blackout;
- recover blackout;
- show diagnostic.

### Library

- search;
- tag filter;
- health filter;
- favorite;
- preview;
- load;
- empty result;
- broken scene.

### Playlist

- select cue;
- reorder;
- edit duration;
- transition;
- inherited value;
- override;
- generate;
- preview generated result.

### Editor

- source tab;
- dirty state;
- compile;
- success;
- error;
- jump to error;
- restore backup.

### Mappings

- learn;
- detected input;
- target selection;
- conflict;
- save;
- remove.

### Diagnostics

- status;
- event;
- detailed error;
- recovery action.

## 23. Visual acceptance tests

At 1440×900:

- preview is unmistakably primary;
- no pane feels permanently bolted on;
- transport is readable from normal viewing distance;
- macros are distinguishable by function;
- current/next cue is readable without opening Playlist;
- health can be understood without opening Diagnostics.

At 1280×720:

- preview remains dominant;
- no horizontal scrolling;
- browser and inspector do not simultaneously crush the preview;
- blackout remains obvious;
- essential cue state remains visible.

## 24. Implementation phases

### UI-01 Shell

- application state;
- command model;
- workspace routing;
- navigation;
- persistence skeleton.

### UI-02 Perform

- preview surface;
- transport;
- health;
- contextual panes.

### UI-03 Controls

- ControlDef adapter;
- groups;
- widgets;
- keyboard interaction;
- reset/default/random.

### UI-04 Macros

- macro model;
- resolution;
- rendering;
- keyboard;
- edit flow.

### UI-05 Library

- indexing;
- search;
- tags;
- filters;
- health;
- preview/load.

### UI-06 Playlist

- cue model;
- timeline/list;
- inheritance;
- generator.

### UI-07 Diagnostics

- event stream;
- health;
- detailed errors;
- recovery commands.

### UI-08 Editor

- source tabs;
- compile state;
- diagnostics mapping;
- backup/restore.

### UI-09 Mappings

- learn;
- MIDI;
- OSC;
- conflict resolution.

### UI-10 Polish

- typography;
- spacing;
- motion;
- themes;
- reduced motion;
- accessibility;
- final performance pass.

## 25. Explicit non-goals

Do not:

- clone Synesthesia's layout;
- clone Resolume's interface;
- turn Strobe into a DAW;
- build a browser-heavy app around shader thumbnails;
- expose every renderer diagnostic in Perform;
- create a permanent control wall;
- make every control a knob;
- require dialogs for routine performance actions;
- run expensive work synchronously from egui paint;
- add decorative animation that competes with the visual output.

## 26. Definition of done for Perform

Perform is ready for production implementation when:

- scene can be previewed without loading;
- scene can be loaded without losing current scene on failure;
- controls derive from ControlDef;
- macros resolve explicitly;
- broken/unavailable macros remain visible;
- browser and inspector are contextual;
- cue state is visible;
- blackout is safe and obvious;
- diagnostics never require a modal interruption;
- 1280×720 remains usable;
- UI work is not visible in renderer frame-time measurements;
- all persistent layout state survives restart.

## 27. Primary research sources

- Synesthesia Controls: https://app.synesthesia.live/docs/manual/controls.html
- Synesthesia Control Hotkeys: https://app.synesthesia.live/docs/manual/control_hotkeys.html
- Synesthesia Scene Editor: https://app.synesthesia.live/docs/manual/scene_editor.html
- Resolume Dashboard: https://resolume.com/support/en/6/dashboard
- Resolume Parameters: https://resolume.com/support/en/parameters
- Resolume Layouts: https://www.resolume.com/support/en/layouts
- Ableton Live 12 Browser: https://help.ableton.com/hc/en-us/articles/12927340213660-The-Live-12-Browser
- Ableton Live 12 Browser/Tags FAQ: https://help.ableton.com/hc/en-us/articles/11425042663708-Browser-and-Tags-in-Live-12-FAQ
- Ableton Live 12 Browser manual: https://www.ableton.com/en/live-manual/12/working-with-the-browser/
- egui repository: https://github.com/emilk/egui
- egui Memory: https://docs.rs/egui/latest/egui/struct.Memory.html
- egui Viewports: https://docs.rs/egui/latest/egui/viewport/
- egui repaint API: https://docs.rs/egui/latest/egui/struct.Ui.html
- winit ApplicationHandler: https://docs.rs/winit/latest/winit/application/trait.ApplicationHandler.html
