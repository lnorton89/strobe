# Strobe UI Research Addendum

> Agent-facing research cache, October 2026. This document supplements `docs/UI_DESIGN.md` and `docs/UI_MOCKUP_WORKFLOW.md`. It is deliberately implementation-oriented: observations are converted into rules, tradeoffs, and acceptance tests.

## 1. Research conclusions

The strongest current evidence points toward a **contextual workstation**, not a permanent four-pane dashboard.

The common pattern across serious creative tools is not “show everything.” It is **keep the high-frequency controls immediately reachable, while allowing the user to expand the browser/editor/diagnostic surface when the task requires it**.

- Synesthesia separates scene controls, meta controls, and media into explicit control banks and supports bank-level actions such as lock, undo, default, and randomize. This validates Strobe's distinction between scene controls and global/performance controls. citeturn0search5
- Resolume's Dashboard is explicitly intended for the parameters that matter during performance, avoiding repeated navigation into effect stacks. This strongly supports Strobe Performance Macros as a first-class concept rather than a copy of Synesthesia's full control panel. citeturn0search4
- Ableton Live 12 makes its Browser hideable, searchable, filterable, taggable, resizable, and capable of history. This is strong evidence against making a large scene browser permanently consume Perform workspace width. citeturn0search0turn0search2
- Synesthesia's built-in editor combines live compilation, compare mode, backups, scene metadata, controls, assets and passes. Strobe should therefore treat Editor as a serious workspace, not as a modal dialog attached to Perform. citeturn0search8
- egui's immediate-mode model is a good fit for dynamic controls, but large continuously-laid-out scroll regions can become expensive. UI architecture should therefore avoid giant always-open trees and unnecessarily repainting huge inspectors. citeturn0search9

### Resulting product stance

**Perform is for operating the show. Build is for authoring the show.**

Perform should preserve:
1. Live preview.
2. Current scene identity.
3. Performance Macros.
4. Cue/transport state.
5. A small, persistent health indicator.
6. Fast access to the current scene's most relevant controls.

Perform should not permanently expose:
- the entire scene library,
- every control,
- editor source code,
- full diagnostics,
- playlist authoring,
- mapping configuration.

Those are task-specific surfaces.

## 2. Recommended Perform topology

### Default

Use:

`rail | preview | contextual inspector | transport/macros`

The scene browser becomes a **toggleable contextual pane** rather than a mandatory permanent column.

When opened:

`rail | scene browser | preview | inspector | transport/macros`

When both browser and inspector are hidden:

`rail | preview | transport/macros`

This makes the existing four mockup directions a useful progression rather than four unrelated skins.

### Why contextual wins

A VJ spends substantially more time watching and manipulating the output than browsing the library. A persistent library steals horizontal pixels from the most important visual surface even when it is not needed.

The browser should appear when:
- selecting a new scene,
- searching,
- tagging,
- previewing,
- importing,
- organizing.

It should recede during continuous performance.

The inspector should behave similarly: it is useful while tuning a scene, but the macro strip is the higher-frequency performance surface.

## 3. Performance Macro model

The Macro system should combine the useful part of Resolume's Dashboard with Strobe's scene/control architecture.

Each macro needs:

- Stable display name.
- Target type.
- Target identifier.
- Current resolved value.
- Mapping source.
- Range override.
- Optional curve.
- Optional smoothing.
- Availability state.
- Shortcut.
- Stable identity marker.

### Resolution states

Never silently retarget a macro.

Use explicit states:

- **Resolved** — target exists and is writable.
- **Unavailable** — target is valid but not present in the current scene.
- **Broken** — target reference no longer resolves.
- **Disabled** — intentionally disabled.
- **Learned** — external MIDI/OSC source is actively learning.
- **Conflict** — multiple mappings compete for the same target/source.

A broken macro must remain visible enough to repair. Silent disappearance is unacceptable in a live-performance application.

### Suggested target precedence

1. Exact stable control identifier.
2. Normalized control name within its group.
3. Explicit user alias.
4. Global/meta target.
5. Playlist/system action.

Do not use fuzzy matching as an automatic fallback.

## 4. Scene Browser research

Borrow the **mechanics**, not the appearance, of Live's Browser.

Recommended Strobe browser:

- Search field.
- Recent.
- Favorites.
- Tags.
- Creator.
- Health.
- Source/type.
- Performance suitability.
- Import state.
- Preview.
- Load.

The browser should support:
- hide/show,
- width adjustment,
- keyboard navigation,
- persistent filters,
- search history,
- preview without loading,
- explicit Load,
- context menu actions.

Live's Browser demonstrates that filtering and tagging can coexist with a compact result view and that browser width should be user-adjustable. citeturn0search0turn0search2

### Important Strobe-specific addition

Every scene card should expose **runtime health** separately from aesthetic metadata.

Examples:

- READY
- COMPILE ERROR
- MISSING ASSET
- UNSUPPORTED PASS
- SCRIPT ERROR
- HIGH GPU
- DEGRADED AUDIO

This lets the performer make a reliable decision before loading.

## 5. Control inspector research

Synesthesia's group-based controls are the right compatibility model: controls belong to semantic groups and the interface provides bank-level actions. citeturn0search5

Strobe should improve this by making group density adaptive.

### Density tiers

**Focused**
- 1–4 controls.
- Larger controls.
- Useful while actively tuning.

**Normal**
- 5–12 controls.
- Standard compact sliders/toggles.

**Dense**
- 13+ controls.
- Compact rows.
- Search/filter within the inspector.
- Collapse groups by default based on persistence.

Do not render every control as a large card.

### Parameter interaction

Primary interaction should be:
- click-drag slider,
- click-drag numeric field,
- double-click numeric reset,
- right-click context menu,
- keyboard arrows,
- Shift for fine adjustment,
- Enter to commit typed value,
- Escape to cancel typed value.

For performance-critical controls, provide a visible but compact current value. The value should not require opening another dialog.

## 6. Library preview vs load

Preview and Load must remain distinct.

**Preview**
- renders the scene in the local Strobe preview surface,
- does not alter live output,
- does not change playlist state,
- does not change current macro targets,
- may compile/cache asynchronously.

**Load**
- commits the scene to the active performance state,
- resolves controls,
- updates macros,
- records the cue transition,
- reports failures visibly.

This distinction is especially important because imported or user-authored shaders can fail compilation or asset resolution.

## 7. Playlist implications

The playlist should behave more like a performance cue sheet than a file list.

A cue needs:

- scene,
- duration,
- transition,
- transition duration,
- notes,
- tags,
- optional macro snapshot,
- optional seed,
- validation state.

The Perform surface only needs to show:

`NOW → NEXT → FOLLOWING`

with time remaining and transition state.

Playlist authoring belongs in the Playlist workspace.

## 8. Diagnostics should be adjacent, not intrusive

A live visualizer cannot afford modal error dialogs during performance.

Use three levels:

### Level 1 — persistent health

Compact indicators:
- GPU
- FPS
- audio
- script
- output

### Level 2 — transient event

Small event notification:
- shader compiled,
- scene loaded,
- asset missing,
- device disconnected.

### Level 3 — diagnostic workspace

Full investigation:
- subsystem,
- subject,
- phase,
- root cause,
- recovery action,
- event history,
- technical details.

Only Level 3 should consume the main workspace.

## 9. Responsive behavior

The current 1440×900 target is useful, but 1280×720 should be treated as a first-class operating environment.

At 1440×900:
- rail + preview + optional inspector + transport.
- browser can coexist with inspector when explicitly opened.

At 1280×720:
- never force all panes open.
- browser and inspector should compete for the same contextual region.
- preview remains dominant.
- macro strip remains available.
- health collapses to icons with tooltips/details.

At very small sizes:
- inspector becomes an overlay/drawer.
- browser becomes a full-height workspace.
- transport retains only play/cue/blackout/next essentials.

## 10. UI performance constraints

egui performs layout each frame, and very large scroll regions can increase CPU cost. It also avoids continuous repaint when idle unless interaction or animation requires it. citeturn0search9

For Strobe:

- Do not render thousands of scene rows at once.
- Avoid rebuilding expensive shader thumbnails every frame.
- Cache scene thumbnails.
- Avoid CPU readback from the renderer.
- Keep the preview as an independently rendered GPU surface.
- Update performance telemetry at a lower cadence than frame rendering.
- Keep diagnostic log rendering bounded.
- Avoid animation in navigation chrome unless it communicates state.
- Do not let a complex editor force the entire application into a high-repaint mode.

## 11. Keyboard-first performance workflow

Recommended minimum:

- `Space` — play/hold.
- `N` — next cue.
- `P` — previous cue.
- `B` — blackout.
- `Tab` — cycle contextual pane.
- `Ctrl/Cmd+K` — command palette.
- `Ctrl/Cmd+P` — scene search.
- `F` — focus preview.
- `1–9` — Performance Macro slots when assigned.
- `Shift+1–9` — alternate macro action where configured.
- Arrow keys — focused numeric adjustment.
- `Shift+`Arrow — fine adjustment.
- `R` — reset focused control.
- `Shift+R` — randomize focused group when supported.

All destructive or output-affecting actions should provide a clear focus/active-state indication.

## 12. Mockup decision matrix

Score each direction 1–5:

| Criterion | A Instrument | B Contextual | C Minimal | D Studio |
|---|---:|---:|---:|---:|
| Preview dominance | 3 | 4 | 5 | 3 |
| Scene discovery | 5 | 3 | 1 | 5 |
| Control access | 5 | 4 | 2 | 5 |
| Performance focus | 3 | 5 | 5 | 3 |
| Build/Perform separation | 3 | 5 | 5 | 4 |
| 1280×720 viability | 2 | 5 | 5 | 2 |
| Beginner discoverability | 5 | 4 | 2 | 4 |
| Advanced workflow | 4 | 5 | 4 | 5 |
| UI implementation risk | 3 | 4 | 5 | 2 |

These are starting hypotheses, not final scores.

**Current recommendation: B — Contextual.**

It preserves enough of Synesthesia's discoverability while avoiding its tendency to keep every editing surface visible during performance.

## 13. Questions the next interactive prototype must answer

1. Can a performer change scenes without losing sight of output?
2. Can a performer expose controls without permanently shrinking the preview?
3. Can a macro target disappear without becoming confusing?
4. Can Preview and Load be distinguished at a glance?
5. Can a failed shader be rejected before it becomes live output?
6. Can the user operate the essential show controls from keyboard alone?
7. Can the same design remain comfortable at 1280×720?
8. Can the UI remain responsive while the renderer is compiling/loading?
9. Can diagnostics be inspected without turning into a modal interruption?
10. Can an advanced user configure the workspace without forcing that complexity onto a first-time user?

## 14. Agent implementation order

1. Implement shell and workspace state.
2. Implement contextual pane state.
3. Implement scene browser with search/filter/preview/load separation.
4. Implement ControlDef-driven inspector.
5. Implement Performance Macro model and explicit resolution states.
6. Implement transport/cue strip.
7. Implement diagnostics indicators/events.
8. Add persistence for pane visibility, widths, filters and macros.
9. Add keyboard command system.
10. Validate 1440×900 and 1280×720.
11. Only then add visual polish.

## 15. Sources

- Synesthesia Controls: https://app.synesthesia.live/docs/manual/controls.html
- Synesthesia Scene Editor: https://app.synesthesia.live/docs/manual/scene_editor.html
- Resolume Dashboard: https://www.resolume.com/support/dashboard
- Resolume Wire UI: https://www.resolume.com/support/en/wire-user-interface
- Ableton Live 12 Browser: https://help.ableton.com/hc/en-us/articles/12927340213660-The-Live-12-Browser
- Ableton Live 12 Browser manual: https://www.ableton.com/en/live-manual/12/working-with-the-browser/
- Ableton Live 12 new features: https://www.ableton.com/en/live/all-new-features/
- egui repository/performance notes: https://github.com/emilk/egui

## 15. Second-pass research: interaction and runtime implications

This pass checked the current upstream documentation again rather than treating the first research pass as settled.

### Ableton Live 12: browser state is a first-class workflow

Live 12's Browser can be shown/hidden, resized, filtered, tagged, searched, navigated through history, and saved as custom labels. Search can be combined with filters, and the Browser supports keyboard-first navigation. The important lesson for Strobe is not the visual appearance of Live's Browser; it is that browser state is part of the user's workflow and therefore deserves persistence.

Strobe should persist:
- query
- filters
- sort
- selected collection/tag
- browser width
- browser visibility
- recent searches

It should also support saved searches such as favorite, broken, author:lnorton89, and tag:feedback rather than requiring users to repeatedly reconstruct filters.

Source: https://help.ableton.com/hc/en-us/articles/12927340213660-The-Live-12-Browser and https://www.ableton.com/en/live-manual/12/working-with-the-browser/

### Resolume: the Dashboard is more than a shortcut strip

Resolume's Dashboard is explicitly designed around parameters the operator needs frequently, and it supports multiple parameters on one dial with parameter and dial ranges. This suggests that Strobe's Performance Macro should eventually support one operator action driving multiple compatible targets, not merely one macro-to-one-control mappings.

This should be implemented as a later capability because it complicates target resolution and feedback display. The first implementation should support one macro → one target, but the data model should not make multi-target macros impossible.

Source: https://resolume.com/support/en/6/dashboard

### Synesthesia: precision interaction should be deliberate

Synesthesia documents relative slider dragging, Shift-drag precision, direct numeric manipulation and quick reset/default/random/preset/lock actions. These are not cosmetic details. They establish a useful performance interaction contract: normal gestures should be fast; modifiers should make them precise; reset should be immediate and discoverable.

Strobe should therefore avoid sliders that jump to the pointer position when the operator clicks near the handle. Relative dragging should be the default where practical.

Source: https://app.synesthesia.live/docs/manual/control_hotkeys.html

### egui: avoid accidental repaint coupling

egui supports multiple native viewports, but its documentation distinguishes deferred and immediate viewports. Immediate child viewports can cause parent and child repaint coupling, multiplying CPU work when multiple windows animate. Deferred viewports are the preferred architecture when independent repainting matters.

For Strobe this reinforces the existing decision to keep the output window visually separate from the control UI. The output is already a continuously animated surface; the control surface should not be forced into an equally expensive repaint lifecycle merely because both are represented by egui.

Source: https://docs.rs/egui/latest/egui/viewport/

### egui: product state should not hide inside widget memory

egui's Memory stores widget state such as scroll positions, collapsing headers and text cursor state, and can be persisted. Its documentation explicitly says Memory is not intended to store important application data.

Therefore Strobe should distinguish:
- UI memory: collapse state, scroll position, transient widget state
- Strobe state: active scene, macros, playlist, mappings, workspace, pane layout, output assignment

This distinction is important because future UI agents otherwise tend to put product behavior into widget-local state, making it difficult for MIDI/OSC, command palette and persistence to share the same behavior.

Source: https://docs.rs/egui/latest/egui/struct.Memory.html

### egui: expensive derived data belongs in caches or workers

egui provides a cache facility specifically for computations that are expensive but still appropriate to perform in the UI process. Its documentation recommends moving very slow work to another thread.

Strobe should classify work before implementation:

| Work | UI thread | Cache | Worker |
|---|---:|---:|---:|
| control layout | yes | no | no |
| current values | yes | no | no |
| scene thumbnail lookup | yes | yes | maybe |
| thumbnail generation | no | yes | yes |
| directory scan | no | no | yes |
| shader compile | no | no | yes/runtime preparation |
| audio analysis | no | no | yes |
| diagnostic formatting | small only | yes | yes for bulk |
| playlist generation | no | no | yes |

The rule should be: egui decides what to draw; it does not perform expensive product work just because a panel is visible.

Sources: https://docs.rs/egui/latest/egui/struct.Memory.html and https://github.com/emilk/egui

### winit: lifecycle should be explicit from day one

Current Winit 0.30 uses the ApplicationHandler lifecycle and recommends graphics/window creation after the first resumed event. Even though Strobe is Windows-first, adopting this lifecycle model from the beginning avoids baking window creation into global initialization and makes future suspend/resume and monitor changes easier to reason about.

Source: https://docs.rs/winit/latest/winit/application/trait.ApplicationHandler.html

## 16. Revised interaction principles

The second pass produces five stronger principles:

1. Preview is persistent; configuration is contextual.
2. Performance actions are commands, not widget callbacks.
3. UI memory and product state are separate.
4. The browser is a saved workflow, not a file picker.
5. The UI must never make the renderer pay for hidden complexity.

These principles should be treated as architecture constraints for future agents.

## 17. New questions to verify against real scenes and live use

The research should not invent answers where the real Synesthesia behavior can settle them. Add these to the verification queue:

- exact behavior when a control referenced by a preset disappears
- whether FavSlots preserve unresolved targets and how they present them
- whether scene changes preserve global/meta values independently
- precise timing of scene preview preparation versus active load
- whether script-created uniforms can change arity during a scene lifetime
- actual spectrum/level-trail texture coordinate semantics
- expected keyboard precedence while text editing
- whether a blackout should pause scene time or only suppress output
- whether output monitor loss should automatically rebind to the primary monitor
- whether playlist cue state should survive an application restart
- whether preview rendering should use the same adaptive render scale as live output

Until verified, agents should implement explicit state and diagnostics rather than silently guessing.
