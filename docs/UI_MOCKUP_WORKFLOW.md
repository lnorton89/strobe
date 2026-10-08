# Strobe UI Mockup and Design-Decision Workflow

_Last researched: 2026-10-08_

This document defines how agents should create visual mockups for Strobe before committing to production UI architecture. The goal is not to generate one attractive screenshot and immediately implement it. The goal is to give Lawrence a small set of materially different, comparable designs, let him choose the direction, then turn that decision into an interaction-accurate prototype and only then production UI.

## 1. Core rule

**Separate visual exploration from interaction validation.**

Use image generation for fast visual exploration and art direction. Use a code-backed prototype for geometry, behavior, state, keyboard interaction, resizing and real component validation.

Do not use an AI-generated screenshot as the sole specification for a desktop application.

The preferred sequence is:

1. research;
2. establish fixed product constraints;
3. generate 3-5 deliberately different visual directions;
4. compare them at the same viewport and with the same content;
5. select one direction;
6. build a high-fidelity interactive mockup;
7. test the mockup against real Strobe states;
8. refine;
9. promote the chosen design into the production UI contract.

## 2. Why this is the right workflow

Current AI design tooling is increasingly useful for exploration, but its strongest use is iterative generation rather than treating the first output as authoritative.

Figma Make can generate interactive prototypes from natural-language descriptions, iterate them visually, and work from an existing frame or codebase. This makes it useful after the initial visual direction has been selected.

Image generation is useful earlier because it can cheaply explore composition, density, materials, lighting, colour and visual character. OpenAI's image-generation guidance specifically recommends describing an interface as an existing product and focusing on layout, hierarchy, spacing and real UI elements rather than concept-art language.

For Strobe, the division should therefore be:

- **Image generation:** visual direction, composition, density, chrome, theme, component appearance.
- **Figma / interactive prototype tooling:** interaction and responsive behavior when available.
- **Impeccable / frontend implementation skills:** precise production-style UI prototypes and refinement.
- **Actual Strobe app:** final validation of egui constraints, live renderer integration, performance and real data.

## 3. What the first mockup set should contain

The first review should not be seven unrelated screens.

Generate a controlled set around the most important screen:

### Required: Perform workspace

Every variant uses exactly the same:

- 1440x900 desktop viewport;
- current scene;
- next cue;
- scene library entries;
- scene controls;
- performance macros;
- audio/BPM status;
- output status;
- blackout control;
- same number of visible controls;
- same approximate information hierarchy.

Only the visual direction and panel topology should vary.

This makes the comparison meaningful.

### Secondary screens

After the Perform direction is selected, create:

1. Scene Library
2. Playlist
3. Editor
4. Mappings
5. Diagnostics

These should inherit the chosen visual system rather than becoming independent designs.

## 4. The four directions worth testing

Do not ask an image model for "a modern UI". That produces generic dashboard aesthetics.

Use four intentionally different directions.

### Direction A: Instrument

Goal: make Strobe feel like a serious performance instrument.

Characteristics:

- dense but calm;
- dark blue-black application surface;
- restrained amber primary accent;
- strong numeric readouts;
- compact transport;
- large live preview;
- controls feel physical and immediate;
- minimal decoration;
- clear hierarchy between performance actions and configuration.

Risk:

- can become too utilitarian or resemble an audio DAW.

### Direction B: Studio

Goal: make Strobe feel like a contemporary creative workstation.

Characteristics:

- slightly larger typography;
- softer panel hierarchy;
- more visual scene cards;
- contextual inspector;
- generous preview;
- stronger distinction between Perform and Build;
- sophisticated but restrained surfaces.

Risk:

- can become too spacious for live performance.

### Direction C: Technical

Goal: make Strobe feel like a professional shader/visual engineering tool.

Characteristics:

- compact chrome;
- explicit backend/status information;
- highly legible diagnostics;
- strong editor integration;
- precise control values;
- low ornament;
- visually related to modern developer tools without becoming a code editor.

Risk:

- can over-prioritize implementation details and make performance controls feel secondary.

### Direction D: Minimal

Goal: make the output and current performance state dominate.

Characteristics:

- very little permanent chrome;
- floating/contextual panels;
- large preview;
- macros and transport remain;
- library/inspector can disappear;
- strong keyboard and command-palette model;
- almost no decorative containers.

Risk:

- discoverability suffers if secondary surfaces are too hidden.

## 5. Mockup prompt construction

Prompts should describe Strobe as an existing shipped desktop application.

Good prompt structure:

- product;
- user;
- exact screen;
- viewport;
- fixed information;
- layout;
- visual direction;
- interaction affordances;
- typography;
- surface hierarchy;
- what must not appear.

Example specification:

"Design a realistic desktop application UI for Strobe, a Rust/OpenGL VJ and shader performance instrument used live by a visual artist. Show the 1440x900 Perform workspace. The center is dominated by a live shader preview. A narrow navigation rail is on the far left, a compact scene browser sits beside the preview, and a contextual inspector sits on the right. A persistent bottom strip contains transport, current/next cue information and Performance Macros. The title bar contains workspace identity plus compact audio, FPS, GPU, output and render-scale health readouts. Use a dark blue-black professional creative-tool surface, restrained amber accent, pale text, subtle blue-grey secondary text, compact 4/8/12/20 spacing, 6/10px radii, minimal shadows, no gradients, no marketing-dashboard cards, no oversized icons, no glassmorphism. Make it look like a real shipped desktop application, not concept art."

Then append the direction-specific visual treatment.

## 6. Use fixed content

The mockup must use realistic content rather than placeholder rectangles.

Suggested scene names:

- Aurora Field
- SDF Cathedral
- Plasma Bloom
- Fractal Tunnel
- Signal Garden
- Feedback Bloom

Suggested controls:

- Speed
- Warp
- Bass React
- Hit React
- Hue
- Saturation
- Brightness
- Feedback

Suggested macros:

- COLOR
- WARP
- BASS
- FLASH
- NEXT
- BLACKOUT

Suggested status:

- GL 3.3 / NVIDIA
- Audio: WASAPI / Default Output
- 60 FPS
- Adaptive: 0.85
- Output: Display 2 / 60 Hz
- Playlist: Playing / Cue 17

Using identical content across directions makes visual decisions easier.

## 7. Generate variants systematically

Do not generate one prompt and accept whatever appears.

For each direction:

1. Generate an initial composition.
2. Reject obvious layout failures.
3. Regenerate while keeping the direction fixed.
4. Pick the strongest composition.
5. Make one targeted refinement pass.
6. Save the chosen image as a review candidate.

Useful refinement instructions:

- "Keep the exact overall composition. Reduce chrome by 20 percent."
- "Keep all panels in the same locations. Increase the preview area."
- "Keep the layout. Make the inspector more compact and information-dense."
- "Keep the visual language. Remove unnecessary cards and decorative elements."
- "Keep the hierarchy. Increase the visual prominence of the current and next cue."
- "Keep everything else unchanged. Make the transport easier to scan at a glance."

Do not simultaneously change layout, typography, colour, spacing and component style. One-variable iteration produces useful evidence.

## 8. What image generation is bad at

Do not trust generated images for:

- exact text;
- exact iconography;
- exact pixel spacing;
- complex tables;
- keyboard behavior;
- dynamic states;
- scroll behavior;
- responsive breakpoints;
- focus states;
- accessibility;
- disabled/error states;
- real shader previews;
- actual renderer integration.

If a generated screenshot contains attractive but ambiguous UI, treat it as visual inspiration rather than a specification.

## 9. What the interactive prototype must prove

Once a visual direction wins, the prototype must demonstrate:

### Workspace

- Perform / Library / Playlist / Editor / Mappings / Diagnostics switching;
- pane collapse;
- pane resizing;
- saved layout;
- 1440x900 and 1280x720 behavior.

### Preview

- actual or representative live render texture;
- preview focus;
- safe-area/debug overlay;
- output-window distinction.

### Controls

- slider;
- toggle;
- bang;
- XY;
- dropdown;
- colour;
- reset;
- keyboard adjustment;
- unavailable macro state.

### Performance

- previous cue;
- next cue;
- play/hold;
- blackout;
- macro adjustment;
- next-cue visibility.

### Library

- search;
- filters;
- Preview without Load;
- Load;
- favorite;
- failed scene;
- missing asset.

### Diagnostics

- healthy state;
- shader compile failure;
- audio disconnect;
- output failure;
- dropped-frame warning.

A prototype that only demonstrates the happy path is not sufficient.

## 10. Mockup review format

Every design review should show the same five things:

1. **Hero screenshot:** 1440x900 Perform.
2. **Compact screenshot:** 1280x720 Perform.
3. **Interaction state:** control selected / macro active.
4. **Failure state:** shader compile error.
5. **Secondary workspace:** Library or Playlist.

Do not review a design only from its prettiest static screenshot.

## 11. Decision rubric

Score each candidate from 1-5:

| Criterion | Question |
|---|---|
| Performance clarity | Can a VJ understand the current state immediately? |
| Preview dominance | Is the live visual output clearly primary? |
| Control access | Can common changes happen without hunting? |
| Cue awareness | Can current and next actions be understood instantly? |
| Density | Does it fit useful information without feeling cramped? |
| Calmness | Does it avoid visual noise during performance? |
| Discoverability | Can a new user understand where things live? |
| Build separation | Is editing clearly different from performing? |
| Failure visibility | Are problems visible and actionable? |
| Visual identity | Does it look like Strobe rather than a generic dashboard? |
| Resize behavior | Does it remain useful at 1280x720? |
| Implementation fit | Can the design realistically be built in egui? |

A direction should not win solely because it looks attractive.

## 12. Use A/B/C decisions instead of open-ended feedback

When showing mockups, ask focused questions.

Good:

- "Which Perform layout is strongest: A, B or C?"
- "Should the inspector be permanently visible or contextual?"
- "Should Performance Macros be a bottom strip or a floating dock?"
- "Should the scene browser show thumbnails or primarily text?"
- "Which density is better at 1440x900?"

Bad:

- "What do you think?"
- "Any feedback?"
- "Make it more modern."

The latter produce vague iteration and encourage agents to make arbitrary changes.

## 13. Recommended mockup sequence for Strobe

### Pass 1: topology

Generate intentionally different layouts with minimal styling.

Question:

**Where should the information live?**

Do not spend much time on colours.

### Pass 2: visual system

Take the winning topology and create three visual treatments.

Question:

**What should Strobe feel like?**

### Pass 3: density

Test compact, balanced and spacious variants.

Question:

**How much information should remain visible during a performance?**

### Pass 4: interaction

Build a clickable prototype.

Question:

**Does the chosen layout actually work?**

### Pass 5: failure and edge states

Add:

- shader error;
- missing media;
- unavailable macro;
- audio disconnect;
- output unavailable;
- playlist generation failure.

Question:

**Does the design remain understandable when things go wrong?**

### Pass 6: implementation prototype

Implement the chosen design using the actual Strobe UI technology and representative renderer/control state.

Question:

**Does the design survive real constraints?**

## 14. The best use of the Impeccable skill

Use Impeccable after the direction has been selected, not as the source of the initial product decision.

Its design guidance is useful because it explicitly encourages choosing a clear aesthetic direction, identifying the user/problem, defining technical constraints and avoiding generic AI-generated aesthetics.

For Strobe, an agent should give the skill:

- the selected visual direction;
- UI_DESIGN.md;
- this mockup workflow;
- Rustaman design-system references;
- actual Strobe control/scene data;
- target viewport;
- egui constraints;
- performance constraints.

The agent should then refine a concrete interface rather than inventing a new product concept.

## 15. The best use of image generation

Use image generation as a visual research instrument.

Best uses:

- layout exploration;
- component proportions;
- visual hierarchy;
- colour relationships;
- panel treatments;
- density comparisons;
- scene-library card concepts;
- transport concepts;
- empty/error-state art direction;
- output-window presentation.

Poor uses:

- final UI specification;
- exact component dimensions;
- exact text;
- interaction implementation;
- accessibility validation.

OpenAI's current image-generation guidance recommends treating interface previews as existing products and explicitly describing layout, hierarchy, spacing and real interface elements. That matches the Strobe workflow closely.

## 16. Best use of Figma, if available

Figma is valuable after the initial direction selection.

Current Figma tooling supports:

- natural-language generation;
- interactive prototypes;
- visual editing;
- annotations;
- comparison of multiple directions;
- importing an existing frame;
- code-backed prototypes;
- working from an existing codebase in Make.

If the Figma connector is available, use it for the high-fidelity prototype rather than trying to maintain dozens of static screenshots.

The important rule is that Figma becomes a decision/prototyping surface, not a second source of truth that drifts away from the Strobe implementation.

## 17. Production source of truth

After the decision:

1. Record the selected direction in UI_DESIGN.md.
2. Record rejected directions and why they were rejected.
3. Convert visual tokens into the Strobe theme system.
4. Convert layouts into explicit workspace state.
5. Convert controls into ControlDef-driven components.
6. Convert interactions into actions.
7. Build screenshot fixtures.
8. Keep the mockup available as a visual reference until implementation stabilizes.

Do not let an image become an undocumented design dependency.

## 18. Mockup artifact organization

Recommended repository structure:

    docs/
      UI_DESIGN.md
      UI_MOCKUP_WORKFLOW.md
      mockups/
        README.md
        perform/
          direction-a-instrument/
          direction-b-studio/
          direction-c-technical/
          direction-d-minimal/
        selected/
        states/

The repository should not accumulate every failed generation. Keep:

- final candidates;
- selected direction;
- important rejected candidates when their rejection teaches something;
- interaction/state references.

## 19. Agent handoff format

Every mockup task should produce:

    MOCKUP REVIEW
    Screen:
    Viewport:
    Direction:
    Goal:
    Fixed constraints:
    Deliberate differences:
    Known compromises:
    Questions for decision:
    Candidate artifacts:

Agents must explicitly state what changed between candidates.

Example:

    A vs B:
    - same topology
    - B removes permanent library pane
    - B increases preview width by approximately 12%
    - B moves macros into a denser bottom strip
    - same controls and scene content

This prevents agents from claiming two designs are alternatives when they actually change unrelated variables.

## 20. Recommended first Strobe mockup batch

The next visual batch should be exactly:

### A — Instrument / persistent panels

- left rail;
- scene browser;
- large center preview;
- right inspector;
- bottom macros/transport.

### B — Instrument / contextual panels

- left rail;
- large center preview;
- right inspector that can collapse;
- floating/expandable scene browser;
- bottom macros/transport.

### C — Minimal performance

- large center preview;
- very narrow rail;
- compact top status;
- bottom transport/macros;
- browser and inspector hidden until invoked.

### D — Studio workstation

- left library;
- center preview;
- right inspector;
- richer scene metadata;
- larger typography;
- softer panel hierarchy.

Keep all four at 1440x900 with identical scene/control/cue content.

The first decision should be **topology**, not colour.

## 21. Final decision gate

Do not begin production UI implementation until the Perform workspace has:

- one selected topology;
- one selected visual direction;
- defined panel visibility behavior;
- defined macro placement;
- defined control density;
- defined 1440x900 layout;
- defined 1280x720 degradation;
- defined loading/error behavior;
- defined keyboard behavior;
- explicit acceptance criteria.

This is the point where visual exploration ends and implementation begins.

## 22. Important constraint

The mockup process exists to reduce implementation churn.

If a mockup decision changes the architecture, update the architecture documents before production agents start coding.

If a visual detail does not materially affect interaction, implementation or identity, do not spend multiple agent cycles perfecting it before the core topology is accepted.
